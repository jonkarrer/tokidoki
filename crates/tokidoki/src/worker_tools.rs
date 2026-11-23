//! Workers, for the model: the tools that hand work off (`delegate`, `list_workers`,
//! `cancel_worker`) and what the system prompt says of them; the research worker's own tools
//! (`web_search`, `read_page`, on Exa) and prompt; and Jev's review of a report the user has
//! moved on from.

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

/// The one kind of worker there is, for now.
pub(crate) const RESEARCH: &str = "research";

/// What the system prompt says of workers, beside the tools.
pub const PROMPT: &str = "You can hand work to workers, which run on their own while you stay \
with the user. Research (finding current information on the web, comparing options, checking \
sources, writing up what was found) goes to a research worker: call `delegate` once with the \
task, written so it stands alone (the worker sees nothing of this conversation: put in what it \
needs), then tell the user in a sentence that you're on it. Don't research it yourself, don't \
guess the result, and don't delegate the same thing twice. A message that starts with \
<worker-report> is a worker's report, not the user speaking: follow the note under it. \
`list_workers` shows what is running; `cancel_worker` stops one the user no longer wants.";

/// The research worker's system prompt.
pub const RESEARCH_PROMPT: &str = "You are a research worker for a personal assistant. You get one \
task and no conversation. Use `web_search` (and `read_page` for a page worth reading whole) to \
find what the task asks: current, specific, from good sources; search a few ways if the first \
doesn't answer it. Then answer with a short report in Markdown: the answer first, in a few \
sentences, then the details that matter, then \"Sources:\" with each URL you used. Say plainly \
what you couldn't find. Don't ask questions: nobody will answer them.";

/// Adds the tools that hand work to workers.
pub fn add(tools: &mut Tools, late: &Late) {
    tools
        .add(Delegate(late.clone()))
        .add(ListWorkers(late.clone()))
        .add(CancelWorker(late.clone()));
}

/// The research worker's tools.
pub fn research(exa: &Exa) -> Tools {
    let mut tools = Tools::new();
    tools.add(WebSearch(exa.clone())).add(ReadPage(exa.clone()));
    tools
}

struct Delegate(Late);

impl Tool for Delegate {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "delegate".into(),
            description: "Hands research to a worker that runs on its own; its report comes \
                          back by itself. Once per piece of work."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "What to find out, standing alone: the worker sees \
                                        nothing of this conversation."
                    },
                    "title": {
                        "type": "string",
                        "description": "A few words naming it, for the user's Research list."
                    }
                },
                "required": ["task"]
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
            let worker = (agent(&self.0)?).delegate(
                &call.thread,
                &call.id,
                RESEARCH,
                title.trim(),
                &task,
            )?;
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
    let sentence = task
        .trim()
        .split_inclusive(['.', '?', '!', '\n'])
        .next()
        .unwrap_or(task);
    let sentence = sentence.trim().trim_end_matches('.');
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
