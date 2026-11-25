//! The browser, for a mission: five tools (`browser_open`, `browser_read`, `browser_click`,
//! `browser_type`, `browser_back`) over `browser::Browser`, and what the mission is told of
//! them. Each answers with the page as it is after the step, its elements numbered for the
//! next one.
//!
//! What must not happen without the user is refused here, in code, where the model can't
//! talk its way past it: clicking something that buys, pays, commits an order or deletes an
//! account, and typing into a password or card field. Asking the user and going on when
//! they say yes comes next (approvals); until then such a step stops the mission, which
//! says what is left for the user to do.
//!
//! There is one page. Two missions at once would take turns on it, each finding the other's
//! page: the conversation is told to send one browsing job at a time.

use agent::{Call, Replay, Tool, ToolFuture, Tools};
use anyhow::bail;
use browser::{Browser, Element};
use serde_json::{Value, json};

/// What a mission's system prompt says of the browser, beside the tools.
pub const PROMPT: &str = "You have a web browser, for jobs that need a website (shopping, \
posting, booking, looking at a page that needs a login). Work step by step: browser_open an \
address, then act on the numbered elements in each result with browser_click and \
browser_type (submit: true presses Enter after). Element numbers change with every result: \
always use the latest. Read again with browser_read if a page looks unfinished. Prefer a \
site's own search box to guessing addresses, and check the page after each step before the \
next. If a site wants a login, a code or a CAPTCHA, stop: say in your report that the user \
needs to sign in to that site, and what is left to do once they have. Never ask for or guess \
a password. Some steps are refused because they need the user's say-so (buying, paying, \
placing an order, deleting an account, typing into a password or card field): when one is, \
stop there, don't look for another way, and report what you got ready (the cart, the draft) \
and the one step left for the user.";

/// The browser's tools, over `browser`.
pub fn tools(browser: &Browser) -> Tools {
    let mut tools = Tools::new();
    tools
        .add(Open(browser.clone()))
        .add(Read(browser.clone()))
        .add(Click(browser.clone()))
        .add(Type(browser.clone()))
        .add(Back(browser.clone()));
    tools
}

/// Why a step is refused, and what the model should do about it.
fn refused(what: &str) -> String {
    format!(
        "Not done: {what} needs the user's say-so, which a mission can't get yet. Stop here; \
         don't try another way. Report what is ready and that this step is left for the user."
    )
}

struct Open(Browser);

impl Tool for Open {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "browser_open".into(),
            description: "Opens an address in the browser and returns the page with its \
                          numbered elements."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }),
        }
    }

    // Opening a page again does no harm.
    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let url = call.arguments["url"].as_str().unwrap_or_default();
            Ok(self.0.open(url).await?.describe())
        })
    }
}

struct Read(Browser);

impl Tool for Read {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "browser_read".into(),
            description: "Reads the current page again, with freshly numbered elements.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, _: Call) -> ToolFuture<'_> {
        Box::pin(async move { Ok(self.0.read().await?.describe()) })
    }
}

/// The element a call names, by its number.
async fn named(browser: &Browser, arguments: &Value) -> anyhow::Result<(u32, Element)> {
    let Some(number) = arguments["ref"].as_u64() else {
        bail!("give the element's number as ref");
    };
    match browser.element(number as u32).await {
        Some(element) => Ok((number as u32, element)),
        None => bail!("there is no element [{number}] on the page as last read: read it again"),
    }
}

/// Where the page is, as a person would say it: its host.
async fn site(browser: &Browser) -> String {
    let url = browser.url().await.unwrap_or_default();
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next());
    host.unwrap_or("this site").to_string()
}

struct Click(Browser);

impl Tool for Click {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "browser_click".into(),
            description: "Clicks a numbered element from the latest page result.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "ref": { "type": "integer" } },
                "required": ["ref"]
            }),
        }
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let (number, element) = named(&self.0, &call.arguments).await?;
            if element.risky() {
                let what = match element.name.is_empty() {
                    true => element.href.clone().unwrap_or_default(),
                    false => element.name.clone(),
                };
                bail!(refused(&format!(
                    "clicking “{what}” on {}",
                    site(&self.0).await
                )));
            }
            Ok(self.0.click(number).await?.describe())
        })
    }
}

struct Type(Browser);

impl Tool for Type {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "browser_type".into(),
            description: "Types into a numbered field from the latest page result, in place \
                          of what it holds; submit: true presses Enter afterwards. For a \
                          select, the text is the option to choose."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "ref": { "type": "integer" },
                    "text": { "type": "string" },
                    "submit": { "type": "boolean" }
                },
                "required": ["ref", "text"]
            }),
        }
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let (number, element) = named(&self.0, &call.arguments).await?;
            if element.sensitive {
                bail!(refused(&format!(
                    "typing into “{}” (a password or card field) on {}",
                    element.name,
                    site(&self.0).await
                )));
            }
            let text = call.arguments["text"].as_str().unwrap_or_default();
            let submit = call.arguments["submit"].as_bool().unwrap_or(false);
            Ok(self.0.type_into(number, text, submit).await?.describe())
        })
    }
}

struct Back(Browser);

impl Tool for Back {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "browser_back".into(),
            description: "Goes back to the page before this one.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    fn execute(&self, _: Call) -> ToolFuture<'_> {
        Box::pin(async move { Ok(self.0.back().await?.describe()) })
    }
}
