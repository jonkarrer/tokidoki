//! Procedures from work done: how a job was carried out, kept so the next one like it starts
//! from the way that worked and not from nothing. This is what procedural memory is for.
//!
//!   learn   when a worker comes back from a job (a mission), the model reads the job, the
//!           steps it took and how it ended, and writes the procedure: where to go straight
//!           away, what to act on, what didn't work, where it had to stop. A procedure there
//!           already is for that kind of job is corrected, not doubled.
//!   recall  when a job is about to be handed to a worker, the procedures that fit it are
//!           put in its brief. With Jev, each candidate is judged; without, the closest by
//!           words.
//!
//! A procedure is advice, not a script: sites change, and the worker is told to check each
//! page as it goes.

use std::collections::BTreeMap;

use anyhow::Result;
use inference::providers::Api;
use jev::Question;
use memory::{Body, Item, Source, Write};
use serde_json::{Value, json};

use crate::Experience;

/// Procedures weighed for a job, and how many at most go in its brief.
const CANDIDATES: usize = 8;
const IN_A_BRIEF: usize = 3;
/// A procedure this likely to help is put in the brief.
const HELPS_MIN: f64 = 0.5;
/// Without Jev: a procedure sharing this share of the job's longer words is close enough.
const SHARED_WORDS_MIN: f64 = 0.4;
/// How much of the steps a worker took the model is shown.
const DID_CHARS: usize = 14_000;

impl<A: Api> Experience<A> {
    /// The procedures that fit `job`, the likeliest first.
    pub async fn procedures_for(&self, job: &str) -> Result<Vec<Item>> {
        let hits = (self.memory).search(job, &self.scopes.searched(), CANDIDATES * 3)?;
        let candidates: Vec<Item> = (hits.into_iter().map(|hit| hit.item))
            .filter(|item| matches!(item.body, Body::Procedural { .. }))
            .take(CANDIDATES)
            .collect();
        if candidates.is_empty() {
            return Ok(candidates);
        }
        let Some(jev) = &self.jev else {
            return Ok((candidates.into_iter())
                .filter(|item| close(job, &item.body.text()))
                .take(IN_A_BRIEF)
                .collect());
        };
        let mut questions = BTreeMap::new();
        for item in &candidates {
            questions.insert(
                item.id.to_string(),
                Question::noul(
                    &format!(
                        "An assistant is about to do `job`. It once wrote down how a job was \
                         done: \"{}\". Is that the same kind of job, on the same site or \
                         service, so that following it would help?",
                        item.body.text()
                    ),
                    "The same kind of job in the same place",
                    "A different job, or a different site",
                ),
            );
        }
        let response = match jev.ask(json!({ "job": job }), questions).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "jev couldn't weigh procedures; none used");
                return Ok(Vec::new());
            }
        };
        let mut helps: Vec<(Item, f64)> = (candidates.into_iter())
            .filter_map(|item| {
                let p = response.noul(&item.id.to_string())?;
                (p >= HELPS_MIN).then_some((item, p))
            })
            .collect();
        helps.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(helps
            .into_iter()
            .take(IN_A_BRIEF)
            .map(|(item, _)| item)
            .collect())
    }

    /// Writes down how `job` was done, from the steps taken (`did`) and how it ended
    /// (`report`, and whether it was `done`): a new procedure, a correction to one there is,
    /// or nothing when there is nothing worth keeping. `source` is where it was done.
    pub async fn learn_procedure(
        &self,
        job: &str,
        did: &str,
        report: &str,
        done: bool,
        source: Source,
    ) -> Result<Option<Item>> {
        let known = self.procedures_for(job).await.unwrap_or_default();
        let did: String = did.chars().take(DID_CHARS).collect();
        let input = json!({
            "job": job,
            "ended": if done { "done" } else { "not finished" },
            "steps_taken": did,
            "report": report,
            "procedures_already_known": known.iter().map(|item| json!({
                "id": item.id,
                "procedure": item.body.text(),
            })).collect::<Vec<_>>(),
        });
        let reply = self.ask(LEARN_PROMPT, input.to_string()).await?;
        let Some(raw) = json_object(&reply) else {
            anyhow::bail!("the model didn't answer with a procedure: {reply}");
        };
        let text = |key: &str| {
            raw[key]
                .as_str()
                .map(str::trim)
                .unwrap_or_default()
                .to_string()
        };
        let steps: Vec<String> = (raw["steps"].as_array().into_iter().flatten())
            .filter_map(|step| step.as_str().map(|s| s.trim().to_string()))
            .filter(|step| !step.is_empty())
            .collect();
        let (name, trigger) = (text("name"), text("trigger"));
        if raw["action"] == "none" || name.is_empty() || trigger.is_empty() || steps.is_empty() {
            tracing::info!(job, reason = %text("reason"), "nothing learned from the job");
            return Ok(None);
        }
        // Only a procedure it was shown can be the one it corrects.
        let corrects = (raw["id"].as_i64()).and_then(|id| known.iter().find(|item| item.id == id));
        let mut write = Write::procedure(self.scopes.write.clone(), name, trigger, steps);
        write.source = Some(source);
        let item = match corrects {
            Some(old) => {
                write.scope = old.scope.clone();
                write.key = old.key.clone();
                self.memory.replace(old.id, write)?
            }
            None => self.memory.write(write)?,
        };
        tracing::info!(job, id = item.id, corrected = corrects.is_some(), procedure = %item.body.text(), "learned a procedure");
        Ok(Some(item))
    }
}

impl<A: Api> Experience<A> {
    /// Puts what a worker went to learn into memory: `asked` is what the user wanted
    /// learned, `found` what the worker brought back. Each thing worth keeping becomes a
    /// fact, or corrects one already there; `source` is where it was learned.
    pub async fn keep_what_was_learned(
        &self,
        asked: &str,
        found: &str,
        source: Source,
    ) -> Result<Vec<Item>> {
        let found: String = found.chars().take(FOUND_CHARS).collect();
        // What memory already says on the subject, to correct and not repeat.
        let known: Vec<Item> = (self.memory)
            .search(
                &format!("{asked}\n{found}"),
                &self.scopes.searched(),
                KNOWN_FACTS * 2,
            )?
            .into_iter()
            .map(|hit| hit.item)
            .filter(|item| matches!(item.body, Body::Semantic { .. }))
            .take(KNOWN_FACTS)
            .collect();
        let input = json!({
            "asked_to_learn": asked,
            "found": found,
            "already_in_memory": known.iter().map(|item| json!({
                "id": item.id,
                "fact": item.body.text(),
            })).collect::<Vec<_>>(),
        });
        let reply = self.ask(KEEP_PROMPT, input.to_string()).await?;
        let mut kept = Vec::new();
        for raw in super::json_array(&reply)?.into_iter().take(FACTS_AT_ONCE) {
            let Some(text) = raw["text"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            else {
                continue;
            };
            // Only a fact it was shown can be the one it corrects.
            let corrects = (raw["id"].as_i64()).and_then(|id| known.iter().find(|i| i.id == id));
            let mut write = Write::fact(self.scopes.write.clone(), text);
            write.source = Some(source.clone());
            let item = match corrects {
                Some(old) => {
                    write.scope = old.scope.clone();
                    write.key = old.key.clone();
                    self.memory.replace(old.id, write)?
                }
                None => self.memory.write(write)?,
            };
            kept.push(item);
        }
        tracing::info!(asked, facts = kept.len(), "kept what was learned");
        Ok(kept)
    }
}

/// How much of what a worker found the model reads, and how many facts it may keep from it.
const FOUND_CHARS: usize = 24_000;
const FACTS_AT_ONCE: usize = 40;
/// Facts already in memory shown with it, to correct instead of repeating.
const KNOWN_FACTS: usize = 20;

const KEEP_PROMPT: &str = r#"You keep an assistant's long-term memory. Its user asked it to go and learn something, and a worker
has come back with what it found. You get what was asked, what was found, and the facts already in memory on the subject.

Write what is worth remembering as facts, so the assistant can use them months from now without the source:
- each fact one standalone sentence that names its subject in full ("Bernt, the user's iOS app, ..."), never "it" or "the app" alone;
- specific: names, numbers, prices, dates, features, who it is for, as the source gives them; nothing guessed or added;
- one fact per thing; several short facts, not one long one; skip navigation, boilerplate and anything the source doesn't say;
- only what bears on what was asked to be learned;
- in the language of what was found.
When a fact already in memory says the same, leave it out. When one is out of date or less complete, correct it.

Answer with ONLY a JSON array, [] when nothing is worth keeping:
  {"action":"retain","text":"<the fact>"}
  {"action":"refine","id":<id of a fact already in memory>,"text":"<the fact as it should read now>"}"#;

/// Procedures as a worker's brief shows them.
pub fn brief(procedures: &[Item]) -> Option<String> {
    let listed: Vec<String> = (procedures.iter())
        .filter_map(|item| match &item.body {
            Body::Procedural {
                name,
                trigger,
                steps,
            } => Some(format!(
                "{name} (when {trigger}):\n{}",
                (steps.iter().enumerate())
                    .map(|(i, step)| format!("  {}. {step}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
            Body::Semantic { .. } => None,
        })
        .collect();
    (!listed.is_empty()).then(|| {
        format!(
            "<procedures>\nHow jobs like this were done before, from memory. Start from these \
             and skip what they already settle (which page to open, what to click); they may \
             be out of date, so check each page as you go and do what the page needs.\n\n{}\n\
             </procedures>",
            listed.join("\n\n")
        )
    })
}

/// Whether a procedure (`text`) is close enough to `job` to offer, going by words alone: a
/// job that names a site wants a procedure for that site and something else in common; one
/// that names none, a good share of its words.
fn close(job: &str, text: &str) -> bool {
    let shared = shared_words(job, text);
    let sites: Vec<String> = (words(job).into_iter())
        .filter(|word| word.contains('.') && word.chars().any(char::is_alphabetic))
        .collect();
    match sites.is_empty() {
        true => shared >= SHARED_WORDS_MIN,
        false => {
            let text = text.to_lowercase();
            sites.iter().any(|site| text.contains(site)) && shared >= SHARED_WORDS_MIN / 2.0
        }
    }
}

/// The longer words of a text, lowercase.
fn words(text: &str) -> Vec<String> {
    (text.to_lowercase())
        // A site's name is one word: `x.com`, not `x` and `com`.
        .split(|c: char| !(c.is_alphanumeric() || c == '.'))
        .map(|word| word.trim_matches('.'))
        .filter(|word| word.len() > 3)
        .map(str::to_string)
        .collect()
}

/// The share of `job`'s longer words that `text` also has.
fn shared_words(job: &str, text: &str) -> f64 {
    let (job, text) = (words(job), words(text));
    if job.is_empty() {
        return 0.0;
    }
    job.iter().filter(|word| text.contains(word)).count() as f64 / job.len() as f64
}

/// The first JSON object in a reply, past any words or fence around it.
fn json_object(reply: &str) -> Option<Value> {
    let (start, end) = (reply.find('{')?, reply.rfind('}')?);
    serde_json::from_str(reply.get(start..=end)?).ok()
}

const LEARN_PROMPT: &str = r#"You keep an assistant's procedural memory: how jobs are done, so the next one like it is quick.
A worker has just come back from a job. You get the job, whether it ended done, the steps it took (each tool it used,
with what it was given and the start of what came back), its report, and the procedures already known for this kind of job.

Write the procedure a worker should follow next time for this KIND of job, as the shortest way that works:
- the address to open straight away: the deepest one that was reached (the page where the work is done), not the front page and the clicks that led there;
- what to act on, by the words on the page ("click Compose", "type into the What's happening box"), never by element number: numbers change every time;
- what to check before going on, what went wrong and how it was got around, and what never works;
- where it has to stop for the user (signing in, a step that needs their say-so), so the next one stops there at once and doesn't try around it.
Leave out what was particular to this one job (the text of this post, this order's items): say "the text to post", "each item on the list".
Leave out anything about the user. No passwords or codes, ever.

Answer with ONLY one JSON object:
  {"action":"keep","name":"<a few words>","trigger":"<when it applies: the kind of job and the site>","steps":["...","..."]}
  {"action":"refine","id":<id of a known procedure>,"name":"...","trigger":"...","steps":["..."]}   when a known one is for this kind of job: its steps corrected and completed by what was learned now
  {"action":"none","reason":"..."}   when nothing was learned worth keeping: it used no tools, or what happened says nothing about how to do it
A job that didn't finish still teaches: where it stops, and what to do first next time."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brief_lists_procedures_as_numbered_steps() {
        let scope = crate::tests::me();
        let write = Write::procedure(
            scope,
            "Post on X",
            "asked to post on x.com",
            vec![
                "Open https://x.com/compose/post".into(),
                "Type the text".into(),
            ],
        );
        let memory = memory::Memory::in_memory().unwrap();
        let item = memory.write(write).unwrap();
        let brief = brief(&[item]).unwrap();
        assert!(brief.starts_with("<procedures>"));
        assert!(brief.contains("Post on X (when asked to post on x.com):\n  1. Open https://x.com/compose/post\n  2. Type the text"));
        assert!(super::brief(&[]).is_none());
    }

    #[test]
    fn a_procedure_is_close_by_its_site_and_the_words_it_shares_with_the_job() {
        let procedure = "Post on X (when asked to post on x.com): open compose → type the text";
        assert!(close(
            "Make a marketing post on x.com about my app Lernt",
            procedure
        ));
        assert!(!close("Order groceries from the market", procedure));
        // The same words on another site aren't the same job.
        assert!(!close("Make a marketing post on reddit.com", procedure));
        // With no site named, the words carry it.
        assert!(close("Write and post the text", procedure));
    }

    /// On a real model (the cheapest; `ANTHROPIC_API_KEY`): a mission that got as far as a
    /// refused step teaches a procedure, the next job like it finds it, another kind of job
    /// doesn't, and doing it again corrects the procedure instead of writing a second.
    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn a_mission_teaches_a_procedure_the_next_one_finds() {
        let key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
        let experience = crate::tests::experience(&key);
        let source = |thread: &str| Source {
            thread: thread.into(),
            checkpoint: 0,
        };
        let job = "Post this on x.com for my app Bernt: 'Bernt 2.0 is out today.'";
        let did = r#"used browser_open {"url":"x.com"}
browser_open gave: Page: Home / X URL: https://x.com/home Elements: [4] link "Post" [9] textbox "Search"
used browser_click {"ref":4}
browser_click gave: Page: Compose new post / X URL: https://x.com/compose/post Elements: [2] textbox "Post text" [7] button "Post"
used browser_type {"ref":2,"text":"Bernt 2.0 is out today."}
browser_type gave: Page: Compose new post / X URL: https://x.com/compose/post Elements: [2] textbox "Post text" value="Bernt 2.0 is out today." [7] button "Post"
used browser_click {"ref":7}
browser_click refused or failed: Not done: clicking "Post" on x.com needs the user's say-so
said: The post is written and ready; pressing Post is left for you."#;
        let learned = experience
            .learn_procedure(
                job,
                did,
                "Ready, not posted: pressing Post is left for you.",
                true,
                source("worker-1"),
            )
            .await
            .unwrap()
            .expect("a procedure");
        println!("{}", learned.body.text());
        let Body::Procedural { steps, .. } = &learned.body else {
            panic!("not a procedure");
        };
        let all = steps.join(" ");
        assert!(
            all.contains("x.com/compose/post"),
            "goes straight to the compose page"
        );
        assert!(
            !all.contains("Bernt"),
            "nothing particular to this one post"
        );
        assert!(
            !all.contains("[7]") && !all.contains("\"ref\""),
            "no element numbers"
        );

        let next = "Make a marketing post on x.com about my app Lernt.";
        let found = experience.procedures_for(next).await.unwrap();
        assert_eq!(found.len(), 1, "the next post finds it");
        let brief = brief(&found).unwrap();
        println!("{brief}");
        let other = experience
            .procedures_for("Order my groceries from the market.")
            .await;
        assert!(other.unwrap().is_empty(), "another kind of job doesn't");

        // Done again, with something new found out: the same procedure, corrected.
        let did = "used browser_open {\"url\":\"x.com/compose/post\"}\nbrowser_open gave: Page: Log in to X URL: https://x.com/i/flow/login\nsaid: X wants a login; the user needs to sign in.";
        let again = experience
            .learn_procedure(
                next,
                did,
                "Not done: you need to sign in to X.",
                false,
                source("worker-2"),
            )
            .await
            .unwrap();
        let listed = experience.memory().list(&[crate::tests::me()], 10).unwrap();
        println!(
            "{:?}",
            listed
                .iter()
                .map(|item| item.body.text())
                .collect::<Vec<_>>()
        );
        assert_eq!(listed.len(), 1, "one procedure for posting on X, not two");
        if let Some(again) = again {
            assert!(again.version > 1);
            assert!(again.body.text().to_lowercase().contains("sign"));
        }
    }

    /// On a real model (the cheapest; `ANTHROPIC_API_KEY`): what a worker learned about an
    /// app becomes facts that stand alone, and learning it again corrects them, not doubles.
    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn what_was_learned_is_kept_as_facts_and_corrected_the_second_time() {
        let key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
        let experience = crate::tests::experience(&key);
        let source = |thread: &str| Source {
            thread: thread.into(),
            checkpoint: 0,
        };
        let asked =
            "Learn what my iOS app Kettle does from https://kettle.example, to help market it.";
        let found = "Learned:\n- Kettle is an iOS app that times tea steeping.\n- It has presets for green, black and oolong tea.\n- It costs $2.99 once, with no subscription.\n- It is made for people who brew loose-leaf tea.\n\nSources: https://kettle.example";
        let kept = experience
            .keep_what_was_learned(asked, found, source("worker-1"))
            .await
            .unwrap();
        for item in &kept {
            println!("{}", item.body.text());
        }
        assert!((3..=6).contains(&kept.len()), "{} facts", kept.len());
        let all: Vec<String> = kept.iter().map(|item| item.body.text()).collect();
        assert!(
            all.iter().all(|fact| fact.contains("Kettle")),
            "each names its subject"
        );
        assert!(all.iter().any(|fact| fact.contains("2.99")));

        // The price changed: the same facts, the price corrected, nothing doubled.
        let found = found.replace("$2.99 once", "$4.99 once");
        let again = experience
            .keep_what_was_learned(asked, &found, source("worker-2"))
            .await
            .unwrap();
        let now = experience.memory().list(&[crate::tests::me()], 20).unwrap();
        let now: Vec<String> = now.iter().map(|item| item.body.text()).collect();
        println!("after learning again ({} written): {now:#?}", again.len());
        assert_eq!(now.len(), kept.len(), "no fact twice");
        assert!(now.iter().any(|fact| fact.contains("4.99")));
        assert!(!now.iter().any(|fact| fact.contains("2.99")));
    }

    #[test]
    fn a_procedure_is_read_out_of_a_fenced_reply() {
        let reply = "Here it is:\n```json\n{\"action\":\"keep\",\"name\":\"Post\"}\n```";
        assert_eq!(json_object(reply).unwrap()["name"], "Post");
        assert!(json_object("nothing").is_none());
    }
}
