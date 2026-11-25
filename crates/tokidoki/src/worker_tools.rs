//! Workers, for the model: the tools that hand work off (`delegate`, `list_workers`,
//! `cancel_worker`) and what the system prompt says of them; the two kinds of worker, each
//! with its prompt and tools; and Jev's review of a report the user has moved on from.
//!
//!   research  finds something out on the web (`web_search`, `read_page`, on Exa) and writes
//!             a report.
//!   mission   a piece of real work to go away and do: sustained, many steps, with something
//!             to show at the end. Making something in one of the user's accounts ("make a
//!             marketing post on X for my app"), or working through a pile of material
//!             there ("read my last ten emails and summarize them"). Not what the
//!             conversation can do in a step or two itself (reading a note), not a single
//!             look-up, and not research (the public web, searched, written up). Everything
//!             it did is its thread, which the app shows as the mission's trace.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use agent::{Agent, Call, Replay, Tool, ToolFuture, Tools, Worker};
use anyhow::{Context as _, bail};
use exa::Exa;
use jev::{Jev, Question};
use serde_json::{Value, json};

/// The agent the tools act on, set once it exists (the tools are made before it).
pub(crate) type Late = Arc<OnceLock<Agent>>;

fn agent(late: &Late) -> anyhow::Result<&Agent> {
    late.get().context("workers aren't running yet")
}

pub(crate) const RESEARCH: &str = "research";
pub(crate) const MISSION: &str = "mission";

/// What the system prompt says of workers, beside the tools: of the `kinds` there are, and
/// of what a mission can do (`mission_tools`, by name), so one isn't sent on a job its tools
/// can't do.
pub fn prompt(kinds: &[&str], mission_tools: &[String]) -> String {
    let mut text = String::from(
        "You can hand work to workers, which run on their own while you stay with the user. \
         Call `delegate` once with the task, written so it stands alone (the worker sees \
         nothing of this conversation: put in what it needs), then tell the user in a \
         sentence that you're on it. Don't do the work yourself, don't guess the result, and \
         don't delegate the same thing twice.",
    );
    if kinds.contains(&RESEARCH) {
        text.push_str(
            " Research (finding current information on the web, comparing options, checking \
             sources, writing up what was found) goes to a research worker.",
        );
    }
    if kinds.contains(&MISSION) {
        text.push_str(&format!(
            " A mission worker takes a piece of real work to go away and do: sustained, many \
             steps, with something to show at the end. Making or doing something in one of \
             the user's accounts (\"make a marketing post on X for my app\", \"order my \
             groceries\"), or working through a pile of material there (\"read my last ten \
             emails and summarize them\"). Not what you can do yourself in a step or two \
             (reading or saving a note, answering from what you know), and not research: \
             what can be found on the public web is research, however it is worded (\"go to \
             this page and tell me what it says\"). A mission is for what needs the user's \
             own access: a site they are signed in to, their accounts, their own network \
             (\"check the health of my apps on fly.io\"). A mission worker has these tools \
             and no others: {}. Something small that only \
             those tools can do (looking one thing up on a site the user is signed in to) \
             also goes to one, since you have no other way. If the job needs something they \
             can't do, tell the user what you can't do yet instead of sending one. The \
             browser tools, when it has them, drive one web browser with the user's saved \
             logins: send one job that needs it at a time.",
            mission_tools.join(", ")
        ));
    }
    text.push_str(
        " A message that starts with <worker-report> is a worker's report, not the user \
         speaking: follow the note under it. `list_workers` shows what is running; \
         `cancel_worker` stops one the user no longer wants.",
    );
    text
}

/// The research worker's system prompt.
pub const RESEARCH_PROMPT: &str = "You are a research worker for a personal assistant. You get one \
task and no conversation. Use `web_search` (and `read_page` for a page worth reading whole) to \
find what the task asks: current, specific, from good sources; search a few ways if the first \
doesn't answer it. Then answer with a short report in Markdown: the answer first, in a few \
sentences, then the details that matter, then \"Sources:\" with each URL you used. Say plainly \
what you couldn't find. Don't ask questions: nobody will answer them.";

/// The mission worker's system prompt.
pub const MISSION_PROMPT: &str = "You are on a mission for a personal assistant's user. You get \
one job and no conversation. Do it with your tools, step by step: say in a sentence what you \
are about to do before each step, check what each step gave you, and try another way when one \
fails. You have no memory of the user beyond the job you were given and what your tools show \
you. Don't ask questions: nobody will answer them; if something only the user can decide or \
do stands in the way, stop there. Never claim a step you didn't do. End with a short report \
in Markdown: whether the job is done, what you did, and what is left for the user, if \
anything. If your tools can't do the job, say so at once and what would be needed.";

/// Adds the tools that hand work to workers of `kinds`.
pub fn add(tools: &mut Tools, late: &Late, kinds: &[&'static str]) {
    tools
        .add(Delegate(late.clone(), kinds.to_vec()))
        .add(ListWorkers(late.clone()))
        .add(CancelWorker(late.clone()));
}

/// The research worker's tools.
pub fn research(exa: &Exa) -> Tools {
    let mut tools = Tools::new();
    tools.add(WebSearch(exa.clone())).add(ReadPage(exa.clone()));
    tools
}

/// What a mission works with: the user's notes to read, the web with Exa, and a browser
/// where there is one to drive.
pub fn mission(
    notes: &Arc<notes::Notes>,
    exa: Option<&Exa>,
    browser: Option<&browser::Browser>,
) -> Tools {
    let mut tools = crate::note_tools::reading(notes);
    if let Some(exa) = exa {
        tools.extend(research(exa));
    }
    if let Some(browser) = browser {
        tools.extend(crate::browser_tools::tools(browser));
    }
    tools
}

/// The mission worker's system prompt: with a browser, also how to use it.
pub fn mission_prompt(browser: bool) -> String {
    match browser {
        true => format!("{MISSION_PROMPT}\n\n{}", crate::browser_tools::PROMPT),
        false => MISSION_PROMPT.to_string(),
    }
}

/// The agent, and the kinds of worker there are.
struct Delegate(Late, Vec<&'static str>);

impl Tool for Delegate {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "delegate".into(),
            description: "Hands work to a worker that runs on its own; its report comes back \
                          by itself. Once per piece of work."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "What to find out or do, standing alone: the worker \
                                        sees nothing of this conversation."
                    },
                    "kind": {
                        "type": "string",
                        "enum": self.1,
                        "description": "research finds something out and reports; mission \
                                        goes and does a job."
                    },
                    "title": {
                        "type": "string",
                        "description": "A few words naming it, for the user's list."
                    }
                },
                "required": ["task", "kind"]
            }),
        }
    }

    // The same call finds the worker it started.
    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let task = text(&call.arguments, "task");
            if task.trim().is_empty() {
                bail!("delegate needs a task");
            }
            let title = Some(text(&call.arguments, "title"))
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| first_sentence(&task));
            // A model that names no kind, with one kind to choose from, means that one.
            let named = text(&call.arguments, "kind");
            let kind = match (named.trim(), self.1.as_slice()) {
                ("", [only]) => only,
                (named, kinds) => match kinds.iter().find(|kind| **kind == named) {
                    Some(kind) => kind,
                    None => bail!("delegate needs a kind: one of {}", kinds.join(", ")),
                },
            };
            let worker =
                (agent(&self.0)?).delegate(&call.thread, &call.id, kind, title.trim(), &task)?;
            Ok(format!(
                "Started {} on it. Its report comes back on its own: tell the user you're on it.",
                worker.id
            ))
        })
    }
}

struct ListWorkers(Late);

impl Tool for ListWorkers {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "list_workers".into(),
            description: "The workers there have been, latest first, and how each is doing.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, _: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let workers = agent(&self.0)?.workers()?;
            if workers.is_empty() {
                return Ok("No workers yet.".into());
            }
            let lines: Vec<String> = workers.iter().rev().take(20).map(described).collect();
            Ok(lines.join("\n"))
        })
    }
}

fn described(worker: &Worker) -> String {
    format!(
        "{} ({}, {:?}): {}",
        worker.id, worker.kind, worker.status, worker.task
    )
}

struct CancelWorker(Late);

impl Tool for CancelWorker {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "cancel_worker".into(),
            description: "Stops a running worker the user no longer wants; its report never \
                          comes."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Like worker-3." } },
                "required": ["id"]
            }),
        }
    }

    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let id = text(&call.arguments, "id");
            match agent(&self.0)?.cancel_worker(id.trim())? {
                true => Ok(format!("Stopped {id}.")),
                false => Ok(format!("{id} wasn't running.")),
            }
        })
    }
}

/// Pages a search returns, and the characters of each.
const SEARCH_RESULTS: u64 = 5;
const SEARCH_CHARS: usize = 2_500;
/// Characters of a page read whole.
const PAGE_CHARS: usize = 12_000;

struct WebSearch(Exa);

impl Tool for WebSearch {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "web_search".into(),
            description: "Searches the web; each result comes with the start of its page's text."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "results": { "type": "integer", "description": "1 to 8; 5 if not given." }
                },
                "required": ["query"]
            }),
        }
    }

    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let query = text(&call.arguments, "query");
            let n =
                (call.arguments.get("results").and_then(Value::as_u64)).unwrap_or(SEARCH_RESULTS);
            let found = self
                .0
                .search(&query, n.clamp(1, 8) as usize, SEARCH_CHARS)
                .await?;
            if found.results.is_empty() {
                return Ok(format!("Nothing found for \"{query}\"."));
            }
            let pages: Vec<String> = found
                .results
                .iter()
                .enumerate()
                .map(|(i, page)| {
                    let title = page.title.as_deref().unwrap_or("(untitled)");
                    let date = page
                        .published
                        .as_deref()
                        .map(|d| format!(" ({})", &d[..d.len().min(10)]));
                    format!(
                        "[{}] {title}\n{}{}\n{}",
                        i + 1,
                        page.url,
                        date.unwrap_or_default(),
                        page.text.as_deref().unwrap_or("").trim()
                    )
                })
                .collect();
            Ok(pages.join("\n\n"))
        })
    }
}

struct ReadPage(Exa);

impl Tool for ReadPage {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "read_page".into(),
            description: "A web page's text, for one worth reading whole.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }),
        }
    }

    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let url = text(&call.arguments, "url");
            let read = self.0.read(std::slice::from_ref(&url), PAGE_CHARS).await?;
            match read.results.first().and_then(|page| page.text.as_deref()) {
                Some(text) if !text.trim().is_empty() => Ok(text.trim().to_string()),
                _ => bail!("couldn't read {url}"),
            }
        })
    }
}

/// A title for a brief that came without one: its first sentence, cut short.
fn first_sentence(task: &str) -> String {
    // A sentence ends at a mark with a space after it: not inside "Node.js" or "1.2".
    let task = task.trim();
    let end = task
        .char_indices()
        .find(|&(at, c)| {
            c == '\n'
                || (matches!(c, '.' | '?' | '!')
                    && task[at + 1..]
                        .chars()
                        .next()
                        .is_none_or(char::is_whitespace))
        })
        .map_or(task.len(), |(at, c)| at + c.len_utf8());
    let sentence = task[..end].trim().trim_end_matches('.');
    match sentence.char_indices().nth(TITLE_CHARS) {
        Some((cut, _)) => format!("{}…", sentence[..cut].trim_end()),
        None => sentence.to_string(),
    }
}

/// How long a title taken from a brief may be.
const TITLE_CHARS: usize = 80;

/// A string argument, or empty.
fn text(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Jev's call on a report the user has moved on from: worth telling them now?
pub struct JevReview(pub Jev);

/// A report this likely to be worth interrupting for is delivered.
const INTERRUPT_MIN: f64 = 0.5;

impl agent::Review for JevReview {
    fn deliver<'a>(&'a self, worker: &'a Worker, now: &'a [String]) -> agent::ReviewFuture<'a> {
        Box::pin(async move {
            let mut questions = BTreeMap::new();
            questions.insert(
                "interrupt".to_string(),
                Question::noul(
                    "Earlier the user asked their assistant for `task`; it has just come back \
                     with `report`. They are now talking about `now`. Is it worth telling them \
                     now, interrupting what they're doing: it is urgent, or bears on what they \
                     are talking about now?",
                    "Tell them now",
                    "It can wait until they come back to it",
                ),
            );
            let report: String = (worker.report.as_deref().unwrap_or(""))
                .chars()
                .take(4_000)
                .collect();
            let state = json!({ "task": worker.task, "report": report, "now": now });
            match self.0.ask(state, questions).await {
                Ok(response) => {
                    let p = response.noul("interrupt").unwrap_or(0.0);
                    tracing::info!(worker = %worker.id, p, "jev reviewed a report");
                    p >= INTERRUPT_MIN
                }
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "jev couldn't review; holding the report");
                    false
                }
            }
        })
    }
}

/// A worker as the protocol shows it.
pub(crate) fn shown(worker: Worker) -> interface::Worker {
    use interface::{Delivery as D, WorkerStatus as S};
    interface::Worker {
        id: worker.id,
        parent: worker.parent,
        kind: worker.kind,
        title: worker.title,
        task: worker.task,
        status: match worker.status {
            agent::WorkerStatus::Running => S::Running,
            agent::WorkerStatus::Done => S::Done,
            agent::WorkerStatus::Failed => S::Failed,
            agent::WorkerStatus::Cancelled => S::Cancelled,
        },
        started_at: worker.started_at,
        ended_at: worker.ended_at,
        report: worker.report,
        delivery: worker.delivery.map(|delivery| match delivery {
            agent::Delivery::Delivered => D::Delivered,
            agent::Delivery::Held => D::Held,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_from_a_brief_is_its_first_sentence() {
        for (task, title) in [
            (
                "Compare Node.js 22 and Bun 1.2 startup times. Use recent benchmarks.",
                "Compare Node.js 22 and Bun 1.2 startup times",
            ),
            (
                "Is Lisbon bigger than Porto? Give numbers.",
                "Is Lisbon bigger than Porto?",
            ),
            ("Find flights\nto Tokyo", "Find flights"),
            ("Population of Lisbon", "Population of Lisbon"),
        ] {
            assert_eq!(first_sentence(task), title);
        }
    }
}
