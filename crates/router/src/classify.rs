//! Jev as the router's classifier: the threads an input could go to become the options of one
//! choice, decided in a fraction of what the model tier costs. Jev's docs advise against a
//! "none of these" option, so whether the input continues any of them at all is a separate
//! yes-or-no in the same request (answered in parallel), and the choice counts only on a yes.

use std::collections::BTreeMap;

use jev::{Jev, Question};
use serde_json::{Value, json};

use crate::{Candidate, Classifier, ClassifyFuture, Input, ago};

pub struct JevClassifier {
    jev: Jev,
}

impl JevClassifier {
    pub fn new(jev: Jev) -> Self {
        Self { jev }
    }
}

const CONTINUES: &str = "continues";
const THREAD: &str = "thread";

impl Classifier for JevClassifier {
    fn classify<'a>(
        &'a self,
        input: &'a Input,
        candidates: &'a [Candidate<'a>],
    ) -> ClassifyFuture<'a> {
        Box::pin(async move {
            if candidates.is_empty() {
                return None;
            }
            let started = std::time::Instant::now();
            let (state, questions) = request(input, candidates);
            let response = match self.jev.ask(state, questions).await {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "jev couldn't route; the model will");
                    return None;
                }
            };
            let decided = decide(response.noul(CONTINUES)?, response.choice(THREAD)?);
            tracing::info!(
                model = %response.model,
                continues = response.noul(CONTINUES),
                probabilities = ?response.choice(THREAD).map(|c| &c.probabilities),
                decided = ?decided,
                input_tokens = response.usage.input_tokens,
                ms = started.elapsed().as_millis() as u64,
                "jev route"
            );
            Some(decided)
        })
    }
}

/// What Jev is asked: whether the message continues any of the conversations, and which.
fn request(input: &Input, candidates: &[Candidate]) -> (Value, BTreeMap<String, Question>) {
    let described = |c: &Candidate| {
        let when = ago(c.idle);
        let last = if c.last {
            "; the user was last here"
        } else {
            ""
        };
        format!("{} (last active {when}{last})", c.about)
    };
    let conversations: BTreeMap<String, Value> = candidates
        .iter()
        .map(|c| (c.view.info.id.clone(), described(c).into()))
        .collect();
    let state = json!({
        "message": input.text,
        "conversations": conversations,
    });
    let mut questions = BTreeMap::new();
    questions.insert(
        CONTINUES.to_string(),
        Question::noul(
            "Does `message` continue one of `conversations`: the same subject, a follow-up, an \
             answer to what was asked there, or a reference back to it?",
            "It continues one of them",
            "It starts a subject none of them covers",
        ),
    );
    questions.insert(
        THREAD.to_string(),
        Question::choice(
            "Which of `conversations` does `message` continue?",
            conversations,
        ),
    );
    (state, questions)
}

/// The thread the input continues (`None` for a new one), and how sure that is.
fn decide(continues: f64, choice: &jev::Choice) -> (Option<String>, f32) {
    if continues < 0.5 {
        return (None, (1.0 - continues) as f32);
    }
    // Sure only as far as both answers are.
    let sure = continues.min(choice.confidence);
    (Some(choice.choice.clone()), sure as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(thread: &str, confidence: f64) -> jev::Choice {
        jev::Choice {
            choice: thread.into(),
            confidence,
            probabilities: BTreeMap::new(),
        }
    }

    #[test]
    fn a_no_is_a_new_thread_and_a_yes_takes_the_choice() {
        assert_eq!(decide(0.1, &choice("kyoto", 0.9)), (None, 0.9));
        assert_eq!(
            decide(0.95, &choice("kyoto", 0.9)),
            (Some("kyoto".into()), 0.9)
        );
        // A yes is only as sure as the choice it picked.
        assert_eq!(
            decide(0.97, &choice("kyoto", 0.4)),
            (Some("kyoto".into()), 0.4)
        );
    }
}
