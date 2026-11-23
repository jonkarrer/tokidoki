//! Jev, TypeSafe's System One model: fast, cheap, structured decisions. It is sent a `state`
//! (the facts, as text or JSON) and named questions, and answers each with calibrated
//! probabilities instead of text: pick one of some options (`Question::Choice`), yes or no
//! (`Question::Noul`), a level on a scale (`Question::Score`). Questions in one request are
//! answered in parallel, so asking several costs about the time of one.
//!
//! Nothing is set up ahead of time: every request carries its questions whole, so a choice's
//! options can be made per request (the threads an input could go to, say). It is the user's
//! own key, like the model's (`TYPESAFE_API_KEY`).
//!
//! API: <https://docs.typesafe.ai>, `POST https://api.typesafe.ai/v1/systemone`.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BASE_URL: &str = "https://api.typesafe.ai";

/// The model a client asks unless told otherwise: pinned, not `jev-latest`, since thresholds
/// tuned on one version may not hold on the next.
pub const MODEL: &str = "jev-1.14.0";

/// Requests a busy or rate-limited server is sent again, after a doubling wait.
const RETRIES: u32 = 2;
const BACKOFF: Duration = Duration::from_millis(250);

/// A Jev client; cheap to clone.
#[derive(Clone)]
pub struct Jev {
    http: reqwest::Client,
    base: String,
    key: String,
    model: String,
}

/// Never the key.
impl std::fmt::Debug for Jev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jev")
            .field("base", &self.base)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl Jev {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: BASE_URL.into(),
            key: key.into(),
            model: MODEL.into(),
        }
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Another server (a test double, a proxy).
    pub fn base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// The answer to each of `questions`, by name, about `state`.
    pub async fn ask(
        &self,
        state: impl Serialize,
        questions: BTreeMap<String, Question>,
    ) -> Result<Response> {
        let request = Request {
            model: self.model.clone(),
            state: serde_json::to_value(state)?,
            questions,
        };
        let url = format!("{}/v1/systemone", self.base.trim_end_matches('/'));
        let mut attempt = 0;
        loop {
            let res = self
                .http
                .post(&url)
                .bearer_auth(&self.key)
                .json(&request)
                .send()
                .await
                .context("Jev")?;
            let status = res.status();
            if status.is_success() {
                return res.json().await.context("Jev's answer");
            }
            // Rate-limited (429) or overloaded (529): worth another try.
            if matches!(status.as_u16(), 429 | 529) && attempt < RETRIES {
                tokio::time::sleep(BACKOFF * 2u32.pow(attempt)).await;
                attempt += 1;
                continue;
            }
            let body = res.text().await.unwrap_or_default();
            bail!("Jev returned {status}: {}", detail(&body));
        }
    }
}

/// The `detail` of an error body (a message, or a list of what failed validation), else the body.
fn detail(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return body.to_string();
    };
    match &value["detail"] {
        Value::String(text) => text.clone(),
        Value::Null => body.to_string(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
struct Request {
    model: String,
    state: Value,
    questions: BTreeMap<String, Question>,
}

/// A question about the state. `instructions` say what to decide; in them, refer to parts of
/// the state by their keys in backticks (`message`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    /// One of 1–255 options, by name, each with a description (`Value::Null` for none: the
    /// name alone). Descriptions are what tell the options apart.
    Choice {
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<Value>,
        criteria: BTreeMap<String, Value>,
    },
    /// Yes or no, as the probability of yes.
    Noul {
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    /// A level on an ordered scale of 1–10, lowest first.
    Score {
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<Value>,
        criteria: Vec<Value>,
    },
}

impl Question {
    pub fn choice(instructions: &str, criteria: BTreeMap<String, Value>) -> Self {
        Self::Choice {
            instructions: Some(instructions.into()),
            criteria,
        }
    }

    /// Yes or no, with what each means.
    pub fn noul(instructions: &str, yes: &str, no: &str) -> Self {
        Self::Noul {
            instructions: Some(instructions.into()),
            criteria: Some(NoulCriteria {
                yes: Some(yes.into()),
                no: Some(no.into()),
            }),
        }
    }
}

/// What a yes and a no mean.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoulCriteria {
    #[serde(rename = "true", skip_serializing_if = "Option::is_none")]
    pub yes: Option<Value>,
    #[serde(rename = "false", skip_serializing_if = "Option::is_none")]
    pub no: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The version that answered (an alias asked for names its current version).
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

impl Response {
    pub fn choice(&self, name: &str) -> Option<&Choice> {
        match self.answers.get(name)? {
            Answer::Choice(choice) => Some(choice),
            _ => None,
        }
    }

    /// The probability of yes.
    pub fn noul(&self, name: &str) -> Option<f64> {
        match self.answers.get(name)? {
            Answer::Noul { noul } => Some(*noul),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    Choice(Choice),
    /// The probability of yes, 0–1.
    Noul {
        noul: f64,
    },
    Score {
        /// The levels averaged by probability: may fall between two.
        score: f64,
        confidence: f64,
        /// Each level ("0", "1", …) and its criterion.
        legend: BTreeMap<String, Value>,
        probabilities: BTreeMap<String, f64>,
    },
    Box {
        #[serde(rename = "box")]
        region: Region,
        confidence: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    /// The likeliest option.
    pub choice: String,
    pub confidence: f64,
    /// Every option's probability; they add up to about 1.
    pub probabilities: BTreeMap<String, f64>,
}

/// Part of an image, each side 0–1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// What is billed.
    pub input_tokens: u64,
    /// Free, for now.
    pub output_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn questions_are_sent_as_the_api_reads_them() {
        let mut criteria = BTreeMap::new();
        criteria.insert("returns".to_string(), json!("Sending something back"));
        criteria.insert("billing".to_string(), Value::Null);
        let choice = Question::choice("Which team handles `ticket`?", criteria);
        assert_eq!(
            serde_json::to_value(&choice).unwrap(),
            json!({
                "type": "choice",
                "instructions": "Which team handles `ticket`?",
                "criteria": { "billing": null, "returns": "Sending something back" }
            })
        );
        let noul = Question::noul(
            "Is `ticket` urgent?",
            "It must be handled today",
            "It can wait",
        );
        assert_eq!(
            serde_json::to_value(&noul).unwrap(),
            json!({
                "type": "noul",
                "instructions": "Is `ticket` urgent?",
                "criteria": { "true": "It must be handled today", "false": "It can wait" }
            })
        );
    }

    #[test]
    fn every_kind_of_answer_is_read() {
        let response: Response = serde_json::from_value(json!({
            "model": "jev-1.14.0",
            "answers": {
                "team": {
                    "type": "choice",
                    "choice": "returns",
                    "confidence": 0.93,
                    "probabilities": { "returns": 0.95, "billing": 0.05 }
                },
                "urgent": { "type": "noul", "noul": 0.12 },
                "mood": {
                    "type": "score",
                    "score": 1.4,
                    "confidence": 0.7,
                    "legend": { "0": "calm", "1": "annoyed", "2": "angry" },
                    "probabilities": { "0": 0.1, "1": 0.4, "2": 0.5 }
                },
                "logo": {
                    "type": "box",
                    "box": { "top": 0.1, "bottom": 0.2, "left": 0.3, "right": 0.4 },
                    "confidence": null
                }
            },
            "usage": { "input_tokens": 328, "output_tokens": 34 }
        }))
        .unwrap();
        assert_eq!(response.choice("team").unwrap().choice, "returns");
        assert_eq!(response.noul("urgent"), Some(0.12));
        assert_eq!(response.noul("team"), None);
        assert!(matches!(
            response.answers["logo"],
            Answer::Box {
                confidence: None,
                ..
            }
        ));
        assert_eq!(response.usage.input_tokens, 328);
    }

    #[test]
    fn an_error_says_what_was_wrong() {
        assert_eq!(detail(r#"{"detail":"bad key"}"#), "bad key");
        assert_eq!(
            detail(r#"{"detail":[{"loc":["body","model"],"msg":"required"}]}"#),
            r#"[{"loc":["body","model"],"msg":"required"}]"#
        );
        assert_eq!(detail("Bad gateway"), "Bad gateway");
    }

    /// Asks the real thing a choice and a yes-or-no (`TYPESAFE_API_KEY`).
    #[tokio::test]
    #[ignore = "calls Jev"]
    async fn jev_answers() {
        let jev = Jev::new(std::env::var("TYPESAFE_API_KEY").expect("set TYPESAFE_API_KEY"));
        let mut options = BTreeMap::new();
        options.insert(
            "shipping".to_string(),
            json!("Where an order is, or when it arrives"),
        );
        options.insert(
            "returns".to_string(),
            json!("Sending something back for a refund"),
        );
        let mut questions = BTreeMap::new();
        questions.insert(
            "team".to_string(),
            Question::choice("Which team should handle `ticket`?", options),
        );
        questions.insert(
            "angry".to_string(),
            Question::noul("Is the customer angry?", "They are angry", "They are calm"),
        );
        let started = std::time::Instant::now();
        let response = jev
            .ask(
                json!({ "ticket": "This kettle broke on day one. I want my money back!" }),
                questions,
            )
            .await
            .unwrap();
        println!("{response:#?} in {:?}", started.elapsed());
        assert_eq!(response.choice("team").unwrap().choice, "returns");
        assert!(response.noul("angry").unwrap() > 0.5);
    }
}
