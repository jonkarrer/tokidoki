//! Exa, web search for models: a query finds pages, and each comes back with its text, so
//! finding and reading are one call. It is the user's own key, like the model's
//! (`EXA_API_KEY`).
//!
//! API: <https://docs.exa.ai>, `POST https://api.exa.ai/search` and `/contents`.

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const BASE_URL: &str = "https://api.exa.ai";

/// An Exa client; cheap to clone.
#[derive(Clone)]
pub struct Exa {
    http: reqwest::Client,
    base: String,
    key: String,
}

/// Never the key.
impl std::fmt::Debug for Exa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Exa")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// What a search or a read returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub results: Vec<Page>,
    /// Everything else Exa says (its request ID, the search type it chose, what it cost…).
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// A page found or read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "publishedDate")]
    pub published: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// The page's text, cut to what was asked for.
    #[serde(default)]
    pub text: Option<String>,
    /// Everything else Exa says of it (its ID, score, image…).
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Exa {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: BASE_URL.into(),
            key: key.into(),
        }
    }

    /// Another server (a test double).
    pub fn base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// Up to `results` pages for `query`, each with up to `chars` characters of its text.
    pub async fn search(&self, query: &str, results: usize, chars: usize) -> Result<Response> {
        self.post(
            "search",
            json!({
                "query": query,
                "type": "auto",
                "numResults": results,
                "contents": { "text": { "maxCharacters": chars } },
            }),
        )
        .await
    }

    /// The text of the pages at `urls`, up to `chars` characters each.
    pub async fn read(&self, urls: &[String], chars: usize) -> Result<Response> {
        self.post(
            "contents",
            json!({ "urls": urls, "text": { "maxCharacters": chars } }),
        )
        .await
    }

    async fn post(&self, endpoint: &str, body: Value) -> Result<Response> {
        let url = format!("{}/{endpoint}", self.base.trim_end_matches('/'));
        let res = self
            .http
            .post(&url)
            .header("x-api-key", &self.key)
            .json(&body)
            .send()
            .await
            .context("Exa")?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            let hint = match status.as_u16() {
                401 | 403 => " (the Exa key was refused)",
                402 => " (the Exa account is out of credit)",
                429 => " (too many searches at once; try again shortly)",
                _ => "",
            };
            bail!("Exa returned {status}{hint}: {body}");
        }
        res.json().await.context("Exa's answer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_response_keeps_what_it_doesnt_name() {
        let response: Response = serde_json::from_value(json!({
            "requestId": "r1",
            "results": [{
                "id": "https://example.com/a",
                "url": "https://example.com/a",
                "title": "A",
                "publishedDate": "2026-11-01T00:00:00.000Z",
                "text": "Hello",
                "score": 0.5
            }]
        }))
        .unwrap();
        assert_eq!(response.rest["requestId"], "r1");
        assert_eq!(response.results[0].text.as_deref(), Some("Hello"));
        assert_eq!(response.results[0].rest["score"], 0.5);
    }

    /// Searches the real thing (`EXA_API_KEY`).
    #[tokio::test]
    #[ignore = "calls Exa"]
    async fn exa_finds_pages_with_their_text() {
        let exa = Exa::new(std::env::var("EXA_API_KEY").expect("set EXA_API_KEY"));
        let started = std::time::Instant::now();
        let found = exa
            .search("Rust programming language release notes", 3, 500)
            .await
            .unwrap();
        println!("{} results in {:?}", found.results.len(), started.elapsed());
        for page in &found.results {
            println!(
                "- {} {:?} ({} chars)",
                page.url,
                page.title,
                page.text.as_deref().unwrap_or("").len()
            );
        }
        assert!(!found.results.is_empty());
        assert!(found.results.iter().all(|p| p.text.is_some()));
        let read = exa
            .read(&[found.results[0].url.clone()], 300)
            .await
            .unwrap();
        assert!(read.results[0].text.is_some());
    }
}
