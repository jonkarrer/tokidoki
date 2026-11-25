//! A web browser a model drives, with five actions (open, read, click, type, back) from which
//! it composes jobs nobody wrote a tool for: search a shop, add to a cart, fill in a form.
//! After super-agent's, which did this on Playwright.
//!
//! The model never sees a page's HTML. After every action it gets a `Snapshot`: the page's
//! title, address and visible text, and the things a person could act on (links, buttons,
//! fields), each with a number. It acts by number: "click 12", "type into 7".
//!
//! What shows the page is an `Engine`, and it only has to do two things: go to an address,
//! and run a script in the page and give back what it returned. The app's is a web view of
//! its own, with its own saved logins. Everything else is here, as scripts run in the page
//! (`snapshot.js`, `act.js`, `settle.js`), so it is the same whatever shows the page.
//!
//! Clicks and typing are made-up events, not a real mouse and keyboard: most pages take
//! them, some don't. And one page is all there is: a link that would open another window
//! opens in this one.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

pub type EngineFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What shows the page.
pub trait Engine: Send + Sync + 'static {
    /// Starts loading `url`; doesn't wait for it.
    fn navigate<'a>(&'a self, url: &'a str) -> EngineFuture<'a, ()>;

    /// Runs `script` in the page and gives back the string it evaluates to. May fail, or
    /// never answer, while the page is changing: the caller tries again.
    fn eval<'a>(&'a self, script: &'a str) -> EngineFuture<'a, String>;
}

/// Something on the page to act on, numbered for this snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    #[serde(rename = "ref")]
    pub number: u32,
    /// link, button, textbox, checkbox, select, …
    pub role: String,
    /// What a person would call it: its label, text or placeholder.
    pub name: String,
    /// What a field holds now. Never a sensitive field's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Where a link goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    /// A password or card field.
    #[serde(default)]
    pub sensitive: bool,
}

/// The page as the model sees it after each action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub url: String,
    pub title: String,
    /// Its visible text, shortened.
    pub text: String,
    pub elements: Vec<Element>,
}

impl Snapshot {
    /// The page as the model reads it: title and address, the text, then the numbered
    /// elements.
    pub fn describe(&self) -> String {
        let elements: Vec<String> = self
            .elements
            .iter()
            .map(|e| {
                let mut line = format!("[{}] {} \"{}\"", e.number, e.role, e.name);
                if let Some(value) = e.value.as_deref().filter(|v| !v.is_empty()) {
                    line.push_str(&format!(" value=\"{value}\""));
                }
                if e.disabled {
                    line.push_str(" (disabled)");
                }
                if e.sensitive {
                    line.push_str(" (sensitive)");
                }
                line
            })
            .collect();
        let text: String = self.text.chars().take(TEXT_CHARS).collect();
        format!(
            "Page: {}\nURL: {}\n\nText:\n{}\n\nElements (use their numbers):\n{}",
            if self.title.is_empty() {
                "(untitled)"
            } else {
                &self.title
            },
            self.url,
            if text.is_empty() { "(none)" } else { &text },
            if elements.is_empty() {
                "(none)".to_string()
            } else {
                elements.join("\n")
            }
        )
    }
}

/// How much of a page's text the model is shown.
const TEXT_CHARS: usize = 3_000;

/// Words on a button or link that spend money, commit an order or destroy something.
const RISKY: [&str; 22] = [
    "place order",
    "place your order",
    "buy now",
    "buy it now",
    "checkout",
    "check out",
    "pay now",
    "payment",
    "purchase",
    "complete order",
    "complete your order",
    "confirm order",
    "confirm purchase",
    "confirm payment",
    "submit order",
    "submit payment",
    "subscribe",
    "donate",
    "send money",
    "transfer",
    "delete account",
    "delete my account",
];

impl Element {
    /// Whether clicking it buys, pays, commits an order or deletes an account: by its words,
    /// or its link's.
    pub fn risky(&self) -> bool {
        if self.role == "textbox" {
            return false;
        }
        let said = |text: &str| {
            // Whole words: "pay now" is in "Pay now!", not in "repay nowhere".
            let words: Vec<String> = text
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(str::to_string)
                .collect();
            let text = format!(" {} ", words.join(" "));
            RISKY.iter().any(|risk| text.contains(&format!(" {risk} "))) || text.contains(" pay ")
        };
        said(&self.name) || self.href.as_deref().is_some_and(said)
    }
}

/// How long a page may take to load.
const LOAD: Duration = Duration::from_secs(20);
/// After it has loaded, how long it may keep changing before it is read anyway.
const BUSY: Duration = Duration::from_millis(2_500);
/// A page this long without a change has settled.
const QUIET_MS: u64 = 400;
/// How long one script may take to answer: a page in the middle of changing may never.
const ANSWER: Duration = Duration::from_secs(4);
const POLL: Duration = Duration::from_millis(150);

/// The browser: one page, acted on one step at a time. Cheap to clone.
#[derive(Clone)]
pub struct Browser {
    engine: Arc<dyn Engine>,
    /// The last snapshot, whose numbers the next action uses. Held through an action, so two
    /// never overlap.
    last: Arc<Mutex<Option<Snapshot>>>,
}

impl Browser {
    pub fn new(engine: Arc<dyn Engine>) -> Self {
        Self {
            engine,
            last: Arc::default(),
        }
    }

    /// Goes to `url` (`https://` if it names no scheme) and reads the page.
    pub async fn open(&self, url: &str) -> Result<Snapshot> {
        let url = url.trim();
        if url.is_empty() {
            bail!("open needs an address");
        }
        let scheme = url
            .split_once(':')
            .is_some_and(|(scheme, _)| scheme.chars().all(|c| c.is_ascii_alphanumeric()));
        let url = match scheme {
            true => url.to_string(),
            false => format!("https://{url}"),
        };
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("only web pages can be opened (http or https)");
        }
        let mut last = self.last.lock().await;
        self.engine.navigate(&url).await?;
        self.read_into(&mut last, Some(&url)).await
    }

    /// The page as it is now, freshly numbered.
    pub async fn read(&self) -> Result<Snapshot> {
        let mut last = self.last.lock().await;
        self.read_into(&mut last, None).await
    }

    /// The element numbered `number` in the last snapshot.
    pub async fn element(&self, number: u32) -> Option<Element> {
        let last = self.last.lock().await;
        last.as_ref()?
            .elements
            .iter()
            .find(|e| e.number == number)
            .cloned()
    }

    /// The address of the page last read.
    pub async fn url(&self) -> Option<String> {
        self.last.lock().await.as_ref().map(|page| page.url.clone())
    }

    pub async fn click(&self, number: u32) -> Result<Snapshot> {
        let mut last = self.last.lock().await;
        self.act(&last, number, "click", "", false).await?;
        self.read_into(&mut last, None).await
    }

    /// Replaces what the field holds with `text`; `submit` presses Enter after.
    pub async fn type_into(&self, number: u32, text: &str, submit: bool) -> Result<Snapshot> {
        let mut last = self.last.lock().await;
        self.act(&last, number, "type", text, submit).await?;
        self.read_into(&mut last, None).await
    }

    pub async fn back(&self) -> Result<Snapshot> {
        let mut last = self.last.lock().await;
        self.answer("(() => { history.back(); return '{}'; })()")
            .await?;
        self.read_into(&mut last, None).await
    }

    async fn act(
        &self,
        last: &Option<Snapshot>,
        number: u32,
        action: &str,
        text: &str,
        submit: bool,
    ) -> Result<()> {
        let known =
            (last.as_ref()).is_some_and(|page| page.elements.iter().any(|e| e.number == number));
        if !known {
            bail!("there is no element [{number}] on the page as last read: read it again");
        }
        let script = format!(
            "const REF = {number}, ACTION = {}, TEXT = {}, SUBMIT = {submit};\n{}",
            Value::from(action),
            Value::from(text),
            include_str!("act.js")
        );
        // In a block of its own, so its names are this script's alone.
        let done = self.answer(&format!("{{ {script} }}")).await?;
        if done.get("gone").is_some() {
            bail!("element [{number}] is gone (the page changed): read it again");
        }
        if let Some(error) = done.get("error").and_then(Value::as_str) {
            bail!("{error}");
        }
        Ok(())
    }

    /// Waits for the page to load and go quiet, then reads it. `expect` is the address being
    /// opened: until the page says it is somewhere else than where it was, the old page is
    /// still showing.
    async fn read_into(
        &self,
        last: &mut Option<Snapshot>,
        expect: Option<&str>,
    ) -> Result<Snapshot> {
        let before = last.as_ref().map(|page| page.url.clone());
        self.settle(expect.is_some().then_some(before).flatten())
            .await;
        let page = self
            .answer(include_str!("snapshot.js"))
            .await
            .context("the page couldn't be read")?;
        if let Some(error) = page.get("error").and_then(Value::as_str) {
            bail!("the page couldn't be read: {error}");
        }
        let page: Snapshot = serde_json::from_value(page).context("the page's snapshot")?;
        *last = Some(page.clone());
        Ok(page)
    }

    /// Waits until the page has loaded and stopped changing, within limits: a page that
    /// never stops (a ticker, an animation) is read as it is. `leaving` is the address of a
    /// page being navigated away from.
    async fn settle(&self, leaving: Option<String>) {
        let script = format!("const QUIET = {QUIET_MS};\n{}", include_str!("settle.js"));
        let script = format!("{{ {script} }}");
        let started = Instant::now();
        // What was done takes a moment to start changing the page.
        tokio::time::sleep(POLL).await;
        let mut loaded: Option<Instant> = None;
        while started.elapsed() < LOAD {
            if let Ok(state) = self.answer(&script).await {
                let ready = state["ready"].as_bool().unwrap_or(false);
                let left = leaving.as_deref().is_none_or(|old| {
                    state["url"].as_str() != Some(old) || started.elapsed() > BUSY
                });
                if ready && left {
                    let since = *loaded.get_or_insert_with(Instant::now);
                    if state["quiet"].as_bool().unwrap_or(false) || since.elapsed() > BUSY {
                        return;
                    }
                }
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Runs a script that answers with JSON, and reads the answer.
    async fn answer(&self, script: &str) -> Result<Value> {
        let said = tokio::time::timeout(ANSWER, self.engine.eval(script))
            .await
            .context("the page didn't answer")??;
        // An engine may hand the string back as JSON itself: a string in a string.
        let value: Value = serde_json::from_str(&said).context("what the page answered")?;
        match value {
            Value::String(inner) => serde_json::from_str(&inner).context("what the page answered"),
            other => Ok(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as Sync;

    /// A page that is always loaded and quiet, shows what it was last sent to, and keeps
    /// the scripts it was run.
    #[derive(Default)]
    struct Fake {
        url: Sync<String>,
        ran: Sync<Vec<String>>,
        /// What an action answers.
        acts: Sync<String>,
    }

    impl Engine for Fake {
        fn navigate<'a>(&'a self, url: &'a str) -> EngineFuture<'a, ()> {
            Box::pin(async move {
                *self.url.lock().unwrap() = url.to_string();
                Ok(())
            })
        }

        fn eval<'a>(&'a self, script: &'a str) -> EngineFuture<'a, String> {
            Box::pin(async move {
                self.ran.lock().unwrap().push(script.to_string());
                let url = self.url.lock().unwrap().clone();
                let answer = if script.contains("__tkWatch") {
                    serde_json::json!({ "ready": true, "quiet": true, "url": url })
                } else if script.contains("const REF") {
                    serde_json::from_str(&self.acts.lock().unwrap()).unwrap()
                } else {
                    serde_json::json!({
                        "url": url, "title": "Shop", "text": "Kettles",
                        "elements": [
                            { "ref": 1, "role": "searchbox", "name": "Search", "value": "kettle" },
                            { "ref": 2, "role": "button", "name": "Add to Cart" },
                            { "ref": 3, "role": "textbox", "name": "Password", "sensitive": true }
                        ]
                    })
                };
                // As a web view does: the string, as JSON.
                Ok(serde_json::to_string(&answer.to_string()).unwrap())
            })
        }
    }

    fn browser() -> (Browser, Arc<Fake>) {
        let fake = Arc::new(Fake::default());
        *fake.acts.lock().unwrap() = r#"{"ok":true}"#.into();
        (Browser::new(fake.clone()), fake)
    }

    #[tokio::test]
    async fn a_page_is_opened_and_read_as_numbered_elements() {
        let (browser, _) = browser();
        let page = browser.open("shop.example").await.unwrap();
        assert_eq!(page.url, "https://shop.example");
        assert_eq!(
            page.describe(),
            "Page: Shop\nURL: https://shop.example\n\nText:\nKettles\n\nElements (use their numbers):\n\
             [1] searchbox \"Search\" value=\"kettle\"\n[2] button \"Add to Cart\"\n\
             [3] textbox \"Password\" (sensitive)"
        );
        assert_eq!(browser.element(2).await.unwrap().name, "Add to Cart");
        assert!(browser.open("file:///etc/passwd").await.is_err());
        assert!(browser.open("javascript:alert(1)").await.is_err());
    }

    #[tokio::test]
    async fn an_action_names_its_element_and_what_to_type() {
        let (browser, fake) = browser();
        browser.open("https://shop.example").await.unwrap();
        browser.type_into(1, "say \"hi\"", true).await.unwrap();
        let ran = fake.ran.lock().unwrap();
        let act = ran.iter().find(|s| s.contains("const REF")).unwrap();
        assert!(
            act.contains(r#"const REF = 1, ACTION = "type", TEXT = "say \"hi\"", SUBMIT = true;"#)
        );
    }

    #[tokio::test]
    async fn an_element_that_isnt_there_is_refused_in_words() {
        let (browser, fake) = browser();
        // Nothing read yet: there are no numbers to act on.
        assert!(browser.click(2).await.is_err());
        browser.open("https://shop.example").await.unwrap();
        let unknown = browser.click(9).await.unwrap_err().to_string();
        assert!(unknown.contains("no element [9]"), "{unknown}");
        *fake.acts.lock().unwrap() = r#"{"gone":true}"#.into();
        let gone = browser.click(2).await.unwrap_err().to_string();
        assert!(gone.contains("is gone"), "{gone}");
    }

    #[test]
    fn what_spends_money_or_deletes_is_risky_by_its_words() {
        let element = |role: &str, name: &str, href: Option<&str>| Element {
            number: 1,
            role: role.into(),
            name: name.into(),
            value: None,
            href: href.map(str::to_string),
            disabled: false,
            sensitive: false,
        };
        for name in [
            "Place your order",
            "Buy Now",
            "Proceed to checkout",
            "Pay",
            "Delete my account",
        ] {
            assert!(element("button", name, None).risky(), "{name}");
        }
        for name in [
            "Add to Cart",
            "Search",
            "Repay nowhere",
            "Paypal settings",
            "Delete draft",
        ] {
            assert!(!element("button", name, None).risky(), "{name}");
        }
        assert!(element("link", "Continue", Some("https://shop.example/checkout")).risky());
        // Words typed into a field don't make the field risky.
        assert!(!element("textbox", "Buy now", None).risky());
    }
}
