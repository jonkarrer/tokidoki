//! Jev's judgement on what the user just said, in one request answered in parallel: which of
//! the memories it might need would help to answer it (recall), and whether it tells
//! something lasting about the user worth keeping (retain). Jev decides; it doesn't write:
//! a memory worth keeping is still written by the model, which sees the memories it might
//! correct (`Experience::decide`).
//!
//! It stands in for Attention's pattern detectors on the user's words, which only notice
//! what they have a pattern for: "What should I cook tonight?" recalls nothing by pattern,
//! and "I moved to Porto last month" is kept only if a pattern happens to match.

use std::collections::BTreeMap;

use anyhow::Result;
use jev::{Jev, Question};
use memory::Item;
use serde_json::json;

/// Memories Jev weighs per message: the ones that share words with it, then the most
/// recent, up to this many. Each is one yes-or-no, answered in parallel.
pub(crate) const CANDIDATES: usize = 30;
/// A memory this likely to help is recalled.
pub(crate) const RECALL_MIN: f64 = 0.5;
/// What the user said is considered for keeping when it is this likely to be lasting.
pub(crate) const RETAIN_MIN: f64 = 0.5;
/// Earlier messages shown with the user's, so "yes, that's right" can be understood.
pub(crate) const CONTEXT_MESSAGES: usize = 4;
const CONTEXT_CHARS: usize = 400;

const LASTING: &str = "lasting";
const WORK: &str = "work";
const ANSWER: &str = "answer";
const RESEARCH: &str = "research";
const MISSION: &str = "mission";
const LEARNING: &str = "learning";
/// A message this likely to be research gets the model advice to hand it on.
pub(crate) const RESEARCH_MIN: f64 = 0.6;
/// And one this likely to be a mission.
pub(crate) const MISSION_MIN: f64 = 0.6;
/// And one this likely to be something to go and learn.
pub(crate) const LEARNING_MIN: f64 = 0.6;

/// The kinds of worker there are to hand work to.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Delegation {
    /// Looking something up on the web and writing a report.
    pub research: bool,
    /// Carrying out a long job, in many steps with tools.
    pub missions: bool,
    /// Going to learn something where the user says, to remember it.
    pub learning: bool,
}

impl Delegation {
    fn any(self) -> bool {
        self.research || self.missions || self.learning
    }
}

pub(crate) struct Judged {
    /// The candidates likely to help, likeliest first, with how likely.
    pub recall: Vec<(Item, f64)>,
    /// How likely the message tells something lasting about the user.
    pub lasting: f64,
    /// How likely it is work to hand to a research worker; `None` when not asked.
    pub research: Option<f64>,
    /// How likely it is a long job to send a worker on a mission for; `None` when not asked.
    pub mission: Option<f64>,
    /// How likely it asks for something to be learned and remembered; `None` when not asked.
    pub learning: Option<f64>,
}

/// Asks Jev about `message`, said after `earlier` (oldest first, as "who: text"); with
/// `delegation` (the workers there are to hand work to), also whether it is work for one.
pub(crate) async fn judge(
    jev: &Jev,
    message: &str,
    earlier: &[String],
    candidates: Vec<Item>,
    delegation: Delegation,
) -> Result<Judged> {
    let started = std::time::Instant::now();
    let earlier: Vec<String> = earlier
        .iter()
        .map(|line| line.chars().take(CONTEXT_CHARS).collect())
        .collect();
    let state = json!({ "message": message, "earlier_in_the_conversation": earlier });
    let mut questions = BTreeMap::new();
    questions.insert(
        LASTING.to_string(),
        Question::noul(
            "Does `message`, from the user, tell something lasting about them worth remembering \
             in future conversations: a fact about them, their life, work, people or things, a \
             preference, how they want things done, or a change to any of these? A one-off \
             request, a question, or small talk is not.",
            "It tells something lasting about the user",
            "It is a one-off request, a question, or small talk",
        ),
    );
    for item in &candidates {
        questions.insert(
            recall_key(item.id),
            Question::noul(
                &format!(
                    "Tokidoki remembers this about the user: \"{}\". Would knowing it help \
                     to answer `message` well?",
                    item.body.text()
                ),
                "It is relevant to the answer",
                "It has nothing to do with it",
            ),
        );
    }
    if delegation.any() {
        let mut work = BTreeMap::new();
        work.insert(
            ANSWER.to_string(),
            json!(
                "Tokidoki can answer it now, from what it knows and the conversation: talk, \
                   advice, writing, explaining, or a quick fact"
            ),
        );
        if delegation.research {
            work.insert(
                RESEARCH.to_string(),
                json!(
                    "It needs looking into: finding current information on the web, comparing \
                     options, checking several sources, or a written report"
                ),
            );
        }
        if delegation.missions {
            work.insert(
                MISSION.to_string(),
                json!(
                    "It is a piece of real work to go away and do for the user: sustained, \
                     many steps, with something to show at the end. Making or doing something \
                     in one of their accounts (writing and publishing a post, placing an \
                     order, booking), or working through a pile of material there (reading \
                     their recent emails and summarizing them, checking the health of \
                     their apps on a hosting service or their own network). It needs their \
                     own access: what anyone could find on the public web is not this. Not a \
                     single look-up, and not reading their notes or memory"
                ),
            );
        }
        if delegation.learning {
            work.insert(
                LEARNING.to_string(),
                json!(
                    "The user wants Tokidoki to go and learn about something and keep it in \
                     memory: read a site, a page or a source they name and remember what it \
                     says (\"learn about my app from its site\", \"study this and commit it \
                     to memory\"). Building what it knows, not a report to read"
                ),
            );
        }
        questions.insert(
            WORK.to_string(),
            Question::choice("What does `message` ask of Tokidoki?", work),
        );
    }
    let response = jev.ask(state, questions).await?;
    let lasting = response.noul(LASTING).unwrap_or(0.0);
    let work = |kind: &str, asked: bool| {
        (response.choice(WORK))
            .filter(|_| asked)
            .map(|work| work.probabilities.get(kind).copied().unwrap_or(0.0))
    };
    let research = work(RESEARCH, delegation.research);
    let mission = work(MISSION, delegation.missions);
    let learning = work(LEARNING, delegation.learning);
    let candidates_weighed = candidates.len();
    let mut recall: Vec<(Item, f64)> = candidates
        .into_iter()
        .filter_map(|item| {
            let p = response.noul(&recall_key(item.id))?;
            (p >= RECALL_MIN).then_some((item, p))
        })
        .collect();
    recall.sort_by(|a, b| b.1.total_cmp(&a.1));
    tracing::info!(
        model = %response.model,
        lasting,
        recall = ?recall.iter().map(|(i, p)| (i.id, format!("{p:.2}"), i.body.text())).collect::<Vec<_>>(),
        research,
        mission,
        learning,
        weighed = candidates_weighed,
        input_tokens = response.usage.input_tokens,
        ms = started.elapsed().as_millis() as u64,
        "jev judged"
    );
    Ok(Judged {
        recall,
        lasting,
        research,
        mission,
        learning,
    })
}

fn recall_key(id: i64) -> String {
    format!("memory_{id}")
}

/// Pairs of memories Jev is asked about per request; the requests run together.
const PAIRS_PER_REQUEST: usize = 50;
/// A pair this likely to overlap goes to the model to merge or correct.
pub(crate) const OVERLAP_MIN: f64 = 0.5;

/// Which `pairs` of memories say the same thing, or where one contradicts or updates the
/// other: each pair with how likely, those at `OVERLAP_MIN` or above.
pub(crate) async fn overlapping(jev: &Jev, pairs: &[(Item, Item)]) -> Result<Vec<(i64, i64, f64)>> {
    let started = std::time::Instant::now();
    let mut requests = tokio::task::JoinSet::new();
    for batch in pairs.chunks(PAIRS_PER_REQUEST) {
        let mut questions = BTreeMap::new();
        for (a, b) in batch {
            questions.insert(
                pair_key(a.id, b.id),
                Question::noul(
                    &format!(
                        "Tokidoki remembers both of these about the user. A: \"{}\" B: \"{}\" \
                         Do they say the same thing, or does one contradict, update or \
                         include the other, so they should become one memory?",
                        a.body.text(),
                        b.body.text()
                    ),
                    "They overlap: one memory would do, or one is out of date",
                    "They are separate things worth keeping apart",
                ),
            );
        }
        let jev = jev.clone();
        let state = json!({ "task": "tidying the memories an assistant keeps about its user" });
        requests.spawn(async move { jev.ask(state, questions).await });
    }
    let mut flagged = Vec::new();
    let mut tokens = 0;
    while let Some(answered) = requests.join_next().await {
        let response = answered??;
        tokens += response.usage.input_tokens;
        for (a, b) in pairs {
            if let Some(p) = response.noul(&pair_key(a.id, b.id))
                && p >= OVERLAP_MIN
            {
                flagged.push((a.id, b.id, p));
            }
        }
    }
    tracing::info!(
        pairs = pairs.len(),
        flagged = ?flagged,
        input_tokens = tokens,
        ms = started.elapsed().as_millis() as u64,
        "jev screened memories"
    );
    Ok(flagged)
}

fn pair_key(a: i64, b: i64) -> String {
    format!("pair_{a}_{b}")
}

/// A card this likely to still hold after the new turns isn't rewritten.
pub(crate) const STILL_HOLDS_MIN: f64 = 0.5;
/// Turns this likely to be worth remembering are summarized into an episode.
pub(crate) const WORTH_MIN: f64 = 0.5;

/// How likely `card`, written about a conversation, is still true and complete enough after
/// `since`: the turns that followed it.
pub(crate) async fn still_holds(jev: &Jev, card: &str, since: &str) -> Result<f64> {
    let mut questions = BTreeMap::new();
    questions.insert(
        "still_holds".to_string(),
        Question::noul(
            "`card` describes a conversation as it was; `since` is what was said after. Is \
             `card` still accurate and complete enough, with nothing in `since` it should add \
             or change: no new subject, plan, decision, name, place or result?",
            "The card still holds",
            "The card should be updated",
        ),
    );
    let response = jev
        .ask(json!({ "card": card, "since": since }), questions)
        .await?;
    let p = response.noul("still_holds").unwrap_or(0.0);
    tracing::info!(
        p,
        input_tokens = response.usage.input_tokens,
        "jev judged a card"
    );
    Ok(p)
}

/// How likely `turns` hold something worth remembering about the conversation later.
pub(crate) async fn worth_remembering(jev: &Jev, turns: &str) -> Result<f64> {
    let mut questions = BTreeMap::new();
    questions.insert(
        "worth_remembering".to_string(),
        Question::noul(
            "Does `turns` hold anything worth remembering later about this conversation: a \
             decision, plan, result, piece of work, or something learned about the user? \
             Greetings, small talk and quick lookups are not.",
            "It is worth remembering",
            "It is small talk or a quick lookup",
        ),
    );
    let response = jev.ask(json!({ "turns": turns }), questions).await?;
    let p = response.noul("worth_remembering").unwrap_or(0.0);
    tracing::info!(
        p,
        input_tokens = response.usage.input_tokens,
        "jev judged turns"
    );
    Ok(p)
}

/// Where something worth keeping goes among what is already remembered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Placement {
    /// Nothing remembered says it: a new memory.
    New,
    /// Already remembered: nothing to do.
    Known,
    /// It changes or corrects the memory `id`.
    Changes(i64),
    /// It takes back the memory `id` and says nothing in its place.
    Withdraws(i64),
}

/// A placement Jev is at least this sure of is acted on; a less sure one is left to the model.
pub(crate) const PLACEMENT_MIN: f64 = 0.5;
/// A changed memory this likely to be taken back, rather than replaced, is forgotten.
pub(crate) const WITHDRAWS_MIN: f64 = 0.5;

const NEW: &str = "new";
const KNOWN: &str = "known";

/// Where `message`, which tells something lasting, goes among `related`: the facts it might
/// repeat, change or take back. `None` when Jev isn't sure enough.
pub(crate) async fn placement(
    jev: &Jev,
    message: &str,
    earlier: &[String],
    related: &[Item],
) -> Result<Option<Placement>> {
    let mut options = BTreeMap::new();
    options.insert(
        NEW.to_string(),
        json!("It tells something none of `memories` says"),
    );
    options.insert(
        KNOWN.to_string(),
        json!("`memories` already say it: nothing to add or change"),
    );
    let mut memories = BTreeMap::new();
    for item in related {
        let text = item.body.text();
        options.insert(
            recall_key(item.id),
            json!(format!(
                "It changes, corrects or takes back this memory: \"{text}\""
            )),
        );
        memories.insert(recall_key(item.id), text);
    }
    let mut questions = BTreeMap::new();
    questions.insert(
        "about".to_string(),
        Question::choice(
            "`message` tells something lasting about the user. What does it do to `memories`, \
             what Tokidoki already remembers about them?",
            options,
        ),
    );
    questions.insert(
        "withdraws".to_string(),
        Question::noul(
            "Does `message` take back something about the user, or ask Tokidoki to forget it, \
             without saying what is true instead?",
            "It takes something back and puts nothing in its place",
            "It states or changes a fact",
        ),
    );
    let state = json!({
        "message": message,
        "earlier_in_the_conversation": earlier,
        "memories": memories,
    });
    let response = jev.ask(state, questions).await?;
    let Some(about) = response.choice("about") else {
        return Ok(None);
    };
    let withdraws = response.noul("withdraws").unwrap_or(0.0);
    let placed = match about.choice.as_str() {
        _ if about.confidence < PLACEMENT_MIN => None,
        NEW => Some(Placement::New),
        KNOWN => Some(Placement::Known),
        key => related
            .iter()
            .find(|item| recall_key(item.id) == key)
            .map(|item| match withdraws >= WITHDRAWS_MIN {
                true => Placement::Withdraws(item.id),
                false => Placement::Changes(item.id),
            }),
    };
    tracing::info!(
        probabilities = ?about.probabilities,
        confidence = about.confidence,
        withdraws,
        placed = ?placed,
        input_tokens = response.usage.input_tokens,
        "jev placed a memory"
    );
    Ok(placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How Jev judges cards and turns (`TYPESAFE_API_KEY`): small talk keeps a card and
    /// isn't worth an episode; a change of plan updates it and is.
    #[tokio::test]
    #[ignore = "calls Jev"]
    async fn jev_tells_small_talk_from_what_changes() {
        let jev = Jev::new(std::env::var("TYPESAFE_API_KEY").expect("set TYPESAFE_API_KEY"));
        let card = "Planning a Kyoto trip in April: staying in Gion, looking at ryokans.";
        let thanks = "User: thanks, that's great!\nAssistant: Any time, enjoy the planning.";
        let osaka = "User: Actually, let's do Osaka in May instead.\nAssistant: Sure: Osaka in \
                     May is warm and less crowded.";
        let booked = "User: Let's book Gion Hatanaka for three nights from April 3.\nAssistant: \
                      Done: three nights from April 3, confirmation 4471.";
        let sum = "User: what's 17 times 23?\nAssistant: 391.";
        for (since, holds) in [(thanks, true), (osaka, false), (booked, false)] {
            let p = still_holds(&jev, card, since).await.unwrap();
            println!("still holds {p:.2}: {since:?}");
            assert_eq!(p >= STILL_HOLDS_MIN, holds, "{since}");
        }
        for (turns, worth) in [(thanks, false), (sum, false), (osaka, true), (booked, true)] {
            let p = worth_remembering(&jev, turns).await.unwrap();
            println!("worth remembering {p:.2}: {turns:?}");
            assert_eq!(p >= WORTH_MIN, worth, "{turns}");
        }
    }

    /// How Jev tells research and a mission from what Tokidoki can answer itself, with both
    /// kinds of worker to hand work to (`TYPESAFE_API_KEY`).
    #[tokio::test]
    #[ignore = "calls Jev"]
    async fn jev_tells_research_and_a_mission_from_an_answer() {
        let jev = Jev::new(std::env::var("TYPESAFE_API_KEY").expect("set TYPESAFE_API_KEY"));
        let both = Delegation {
            research: true,
            missions: true,
            learning: true,
        };
        for (message, work) in [
            (
                "Could you look into the current population of Lisbon, with a source?",
                RESEARCH,
            ),
            (
                "Compare the best e-bikes under 2000 euros this year.",
                RESEARCH,
            ),
            (
                "What flights go from Lisbon to Kyoto in April, and roughly what do they cost?",
                RESEARCH,
            ),
            ("Go order my usual groceries.", MISSION),
            ("Post this on X: shipping Tokidoki 0.2 today.", MISSION),
            (
                "Book a table for two at Taberna on Friday at eight.",
                MISSION,
            ),
            ("Go make a marketing post on X for my Bernt app.", MISSION),
            ("Go read my last 10 emails and summarize them.", MISSION),
            ("Tell me the health of all of my apps on fly.io.", MISSION),
            (
                "Go to example.com and tell me what the page says.",
                RESEARCH,
            ),
            (
                "Go learn about my app Bernt from bernt.app and commit what it does to memory.",
                LEARNING,
            ),
            (
                "Study headquarters.jokar.dev so you know my projects.",
                LEARNING,
            ),
            ("I run an iOS app called Bernt.", ANSWER),
            ("What's on my grocery list in my notes?", ANSWER),
            ("Write me a haiku about autumn.", ANSWER),
            ("What's 17 times 23?", ANSWER),
            ("Thanks, that's helpful!", ANSWER),
            ("How do I reverse a list in Python?", ANSWER),
            ("What should I post on X about the release?", ANSWER),
        ] {
            let judged = judge(&jev, message, &[], Vec::new(), both).await.unwrap();
            let (research, mission) = (judged.research.unwrap(), judged.mission.unwrap());
            println!("research {research:.2}, mission {mission:.2}: {message}");
            assert_eq!(research >= RESEARCH_MIN, work == RESEARCH, "{message}");
            assert_eq!(mission >= MISSION_MIN, work == MISSION, "{message}");
            let learning = judged.learning.unwrap();
            assert_eq!(learning >= LEARNING_MIN, work == LEARNING, "{message}");
        }
    }
}
