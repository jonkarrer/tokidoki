//! Reading a web page straight from this computer: one request, no browser and no search
//! service. It is run where tokidoki runs, so it reaches what that computer reaches, a site
//! on the user's own network included, which a search service never sees.
//!
//! `fetch_page` gives a worker the page's text and its links, to read a site through by
//! following them. It signs in to nothing and runs no scripts: a page that wants a login, or
//! is put together by script in a browser, comes back with little to read, and the tool says
//! to use the browser for it.

use std::time::Duration;

use agent::{Call, Replay, Tool, ToolFuture};
use anyhow::{Context as _, bail};
use serde_json::json;

/// How long a page may take, and how much of one is read.
const WITHIN: Duration = Duration::from_secs(20);
const BODY_BYTES: usize = 3_000_000;
/// How much of a page's text a worker is shown, and how many of its links.
const TEXT_CHARS: usize = 12_000;
const LINKS: usize = 60;
/// A page with less text than this probably isn't all there (built by script, or a wall).
const THIN: usize = 200;

pub struct FetchPage(reqwest::Client);

impl FetchPage {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(WITHIN)
            .user_agent("Tokidoki")
            .build()
            .expect("an http client");
        Self(client)
    }
}

impl Tool for FetchPage {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "fetch_page".into(),
            description: "Reads a web page straight from this computer and returns its text \
                          and links. Reaches sites on the user's own network as well as the \
                          public web. No sign-in and no scripts: for a page that needs \
                          either, use the browser."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }),
        }
    }

    // Reading a page again does no harm.
    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let asked = call.arguments["url"].as_str().unwrap_or_default().trim();
            if asked.is_empty() {
                bail!("fetch_page needs an address");
            }
            // A site on a home network often has no https: with no scheme asked for, plain
            // http is tried when https doesn't answer.
            let tries = match asked.contains("://") {
                true => vec![asked.to_string()],
                false => vec![format!("https://{asked}"), format!("http://{asked}")],
            };
            let mut failed = None;
            for address in tries {
                match self.read(&address).await {
                    Ok(page) => return Ok(page),
                    Err(error) => failed = Some(error),
                }
            }
            Err(failed.expect("at least one address was tried"))
        })
    }
}

impl FetchPage {
    async fn read(&self, address: &str) -> anyhow::Result<String> {
        let url =
            reqwest::Url::parse(address).with_context(|| format!("{address} isn't an address"))?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("only web pages can be read (http or https)");
        }
        let response =
            (self.0.get(url).send().await).with_context(|| format!("{address} didn't answer"))?;
        let (status, landed) = (response.status(), response.url().clone());
        let kind = (response.headers().get(reqwest::header::CONTENT_TYPE))
            .and_then(|kind| kind.to_str().ok())
            .unwrap_or("text/html")
            .to_lowercase();
        if !status.is_success() {
            bail!("{landed} answered {status}");
        }
        let readable = ["text/", "json", "xml", "javascript"];
        if !readable.iter().any(|part| kind.contains(part)) {
            bail!("{landed} isn't a page to read ({kind})");
        }
        let body = response.bytes().await.context("reading the page")?;
        let body = String::from_utf8_lossy(&body[..body.len().min(BODY_BYTES)]).into_owned();
        if !kind.contains("html") {
            let text: String = body.chars().take(TEXT_CHARS).collect();
            return Ok(format!("URL: {landed}\n\n{text}"));
        }
        Ok(describe(&read_html(&body, &landed), &landed))
    }
}

/// A page, as much of it as reading needs.
#[derive(Debug, Default, PartialEq)]
struct Page {
    title: String,
    text: String,
    /// What each link says, and where it goes.
    links: Vec<(String, String)>,
}

/// The page as a worker reads it.
fn describe(page: &Page, url: &reqwest::Url) -> String {
    let text: String = page.text.chars().take(TEXT_CHARS).collect();
    let mut out = format!(
        "Page: {}\nURL: {url}\n\nText:\n{}",
        if page.title.is_empty() {
            "(untitled)"
        } else {
            &page.title
        },
        if text.is_empty() { "(none)" } else { &text },
    );
    if page.text.chars().count() < THIN {
        out.push_str(
            "\n\n(There is little text here: the page may be put together by script, or \
             want a sign-in. Open it in the browser if what you need isn't above.)",
        );
    }
    if !page.links.is_empty() {
        // The site's own pages first: those are what reading it through follows.
        let own = |link: &&(String, String)| {
            reqwest::Url::parse(&link.1).is_ok_and(|to| to.host_str() == url.host_str())
        };
        let ordered = (page.links.iter().filter(own))
            .chain(page.links.iter().filter(|link| !own(link)))
            .take(LINKS);
        out.push_str("\n\nLinks:");
        for (says, goes) in ordered {
            out.push_str(&format!("\n- {says}: {goes}"));
        }
    }
    out
}

/// Tags whose insides are never text to read.
const UNREAD: [&str; 6] = ["script", "style", "noscript", "svg", "template", "head"];
/// Tags that end a line.
const BLOCKS: [&str; 22] = [
    "p",
    "div",
    "br",
    "li",
    "ul",
    "ol",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "tr",
    "table",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "main",
    "blockquote",
    "pre",
];

/// Reads a page's title, visible text and links out of its HTML. Not a browser's reading:
/// enough to know what the page says.
fn read_html(html: &str, base: &reqwest::Url) -> Page {
    let mut page = Page::default();
    let mut text = String::new();
    // The link being read, if inside one: where it goes, and where its words start.
    let mut link: Option<(String, usize)> = None;
    let mut rest = html;
    // A line break in the source is only a space: lines are ended by block tags.
    let words = |source: &str| source.replace(['\n', '\r'], " ");
    while let Some(open) = rest.find('<') {
        text.push_str(&words(&rest[..open]));
        rest = &rest[open..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
            continue;
        }
        let Some(close) = rest.find('>') else {
            break;
        };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        let closing = tag.starts_with('/');
        let name: String = (tag.trim_start_matches('/').chars())
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if name == "title" && !closing {
            let end = find_closing(rest, "title");
            page.title = tidy(&unescape(&rest[..end.0]));
            rest = &rest[end.1..];
        } else if UNREAD.contains(&name.as_str()) && !closing && name != "head" {
            rest = &rest[find_closing(rest, &name).1..];
        } else if name == "a" && !closing {
            if let Some(href) = attribute(tag, "href") {
                link = Some((unescape(&href), text.len()));
            }
        } else if name == "a" {
            // Links set side by side read as separate words.
            text.push(' ');
            if let Some((href, from)) = link.take() {
                let says = tidy(&unescape(&text[from..]));
                let goes = base
                    .join(href.trim())
                    .ok()
                    .filter(|to| matches!(to.scheme(), "http" | "https"));
                if let (false, Some(goes)) = (says.is_empty(), goes) {
                    let goes = goes.to_string();
                    if !page.links.iter().any(|(_, known)| *known == goes) {
                        page.links.push((says, goes));
                    }
                }
            }
        } else if BLOCKS.contains(&name.as_str()) {
            text.push('\n');
        }
    }
    text.push_str(&words(rest));
    // Each line tidied, and no run of empty ones.
    let lines: Vec<String> = (unescape(&text).lines().map(tidy))
        .filter(|line| !line.is_empty())
        .collect();
    page.text = lines.join("\n");
    page
}

/// Where the closing tag `name` starts in `html`, and where what follows it does; the end of
/// `html` for both when there is none.
fn find_closing(html: &str, name: &str) -> (usize, usize) {
    let lower = html.to_ascii_lowercase();
    match lower.find(&format!("</{name}")) {
        Some(start) => {
            let after = lower[start..]
                .find('>')
                .map_or(html.len(), |end| start + end + 1);
            (start, after)
        }
        None => (html.len(), html.len()),
    }
}

/// The value of `name` in a tag's inside (`a href="/x" class=y`).
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let at = from + lower[from..].find(name)?;
        from = at + name.len();
        // The name whole, with `=` after it.
        let before = lower[..at].chars().next_back();
        let value = tag[from..].trim_start();
        if before.is_some_and(|c| !c.is_whitespace()) || !value.starts_with('=') {
            continue;
        }
        let value = value[1..].trim_start();
        return Some(match value.chars().next()? {
            quote @ ('"' | '\'') => value[1..].split(quote).next()?.to_string(),
            _ => value.split_whitespace().next()?.to_string(),
        });
    }
}

/// One line of text, its runs of spaces made one.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The commonest HTML entities, as the characters they stand for.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[..rest.len().min(12)].find(';');
        let named = end.map(|end| &rest[1..end]);
        let read = named.and_then(|name| match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "copy" => Some('©'),
            _ => {
                let code = name.strip_prefix('#')?;
                let number = match code.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => code.parse().ok()?,
                };
                char::from_u32(number)
            }
        });
        match (read, end) {
            (Some(character), Some(end)) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> reqwest::Url {
        "http://hq.home.example/apps/".parse().unwrap()
    }

    #[test]
    fn a_page_is_read_as_its_title_text_and_links() {
        let html = r#"<!doctype html><html><head><title>Bernt &amp; friends</title>
            <style>p { color: red }</style><script>var x = "<p>not text</p>";</script></head>
            <body><!-- <p>hidden</p> --><nav><a href="/">Home</a> <a href='pricing'>Pricing</a></nav>
            <h1>Bernt</h1><p>An iOS&nbsp;app   for
            <b>grilling</b>.</p><p>It costs &#36;4 &mdash; once.</p>
            <a href="https://apps.example/bernt">Get it</a><a href="mailto:me@example.com">Mail</a>
            <a href="/">Home again</a><noscript>Turn on scripts</noscript></body></html>"#;
        let page = read_html(html, &base());
        assert_eq!(page.title, "Bernt & friends");
        assert_eq!(
            page.text,
            "Home Pricing\nBernt\nAn iOS app for grilling.\nIt costs $4 — once.\nGet it Mail Home again"
        );
        assert_eq!(
            page.links,
            [
                ("Home".to_string(), "http://hq.home.example/".to_string()),
                (
                    "Pricing".to_string(),
                    "http://hq.home.example/apps/pricing".to_string()
                ),
                (
                    "Get it".to_string(),
                    "https://apps.example/bernt".to_string()
                ),
            ]
        );
    }

    #[test]
    fn the_sites_own_links_come_first_and_a_thin_page_says_so() {
        let page = Page {
            title: String::new(),
            text: "Loading…".into(),
            links: vec![
                ("Store".into(), "https://apps.example/bernt".into()),
                ("About".into(), "http://hq.home.example/about".into()),
            ],
        };
        let shown = describe(&page, &base());
        assert!(
            shown.starts_with(
                "Page: (untitled)\nURL: http://hq.home.example/apps/\n\nText:\nLoading…"
            )
        );
        assert!(shown.contains("Open it in the browser"));
        assert!(shown.ends_with(
            "Links:\n- About: http://hq.home.example/about\n- Store: https://apps.example/bernt"
        ));
    }

    #[test]
    fn broken_html_is_still_read() {
        let page = read_html("<p>Half a tag <a href=x>link", &base());
        assert_eq!(page.text, "Half a tag link");
        assert_eq!(
            read_html("Just words & more", &base()).text,
            "Just words & more"
        );
        assert_eq!(read_html("<script>never closed", &base()).text, "");
        assert_eq!(
            attribute(r#"a data-href="no" href="/yes""#, "href").as_deref(),
            Some("/yes")
        );
        assert_eq!(attribute("a class=x", "href"), None);
    }

    /// Reads a real page on the user's own network (`TOKIDOKI_LOCAL_PAGE`, an address).
    #[tokio::test]
    #[ignore = "reads a page on the network"]
    async fn a_page_on_the_network_is_read() {
        let address = std::env::var("TOKIDOKI_LOCAL_PAGE").expect("set TOKIDOKI_LOCAL_PAGE");
        let page = FetchPage::new().read(&address).await.unwrap();
        println!("{page}");
        assert!(page.contains("URL: "));
    }
}
