//! Workers: work the main model hands off so it can stay with the user. A worker is a run on a
//! thread of its own (`worker-N`, hidden from the conversation), with its kind's system prompt
//! and tools and no memory work: an ordinary run, so it is as durable as any (a crash resumes
//! it where it was). It is started by a tool call (`Agent::delegate`), once per call however
//! often a crash makes that call run again.
//!
//! When it ends, its report goes back to the thread that asked for it (`Report`, a task on
//! that thread): delivered as a new input, so the model tells the user now, when the user is
//! still there (the thread is the one they were in last, and hot) or a `Review` (Jev) judges
//! it worth interrupting them for; otherwise held, written into the thread's context without
//! a turn, for when the user comes back to it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use context::Message;
use engine::{Next, Runtime, Task, TaskOptions, Tx};
use interface::{Rejected, WhenBusy};
use serde::{Deserialize, Serialize};

use crate::{Draft, LAST_THREAD, Settings, Settled, inbox};

pub(crate) const REPORT: &str = "agent.report";
const PREFIX: &str = "worker-";
const INDEX: &str = "agent/workers";
const NEXT: &str = "agent/next-worker";

/// Whether `thread` is a worker's: kept out of the conversation, routing and memory.
pub fn is_worker(thread: &str) -> bool {
    thread.starts_with(PREFIX)
}

fn record_key(worker: &str) -> String {
    format!("agent/worker/{worker}")
}

fn delegated_key(thread: &str, call: &str) -> String {
    format!("agent/delegated/{thread}/{call}")
}

/// A worker, as stored and shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Worker {
    /// Its thread.
    pub id: String,
    /// The thread that asked for it, and the turn it asked in.
    pub parent: String,
    pub turn: u32,
    pub kind: String,
    /// A few words naming it, for the user's list.
    #[serde(default)]
    pub title: String,
    /// What it was asked, in full: its brief.
    pub task: String,
    pub status: WorkerStatus,
    /// Ms since the epoch.
    pub started_at: u64,
    pub ended_at: Option<u64>,
    /// What it found, or why it failed.
    pub report: Option<String>,
    /// What became of the report.
    pub delivery: Option<Delivery>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Given to the parent thread as an input: the model told the user.
    Delivered,
    /// Written into the parent thread's context for later.
    Held,
}

/// The worker a written document holds, if it is one: for reading `Engine::subscribe`.
pub fn worker_in(key: &str, value: Option<&serde_json::Value>) -> Option<Worker> {
    key.strip_prefix("agent/worker/")?;
    serde_json::from_value(value?.clone()).ok()
}

pub(crate) fn load(tx: &mut Tx, worker: &str) -> Result<Worker> {
    tx.doc(&record_key(worker))?
        .with_context(|| format!("no worker {worker}"))
}

fn save(tx: &mut Tx, worker: &Worker) -> Result<()> {
    tx.put_doc(&record_key(&worker.id), worker)
}

/// Starts a worker of `kind` on `task` for the tool call `call` in `thread`, set up with
/// `setup`; the same call again finds the worker it started. A second worker of the same kind
/// in the same turn is refused: the model asked twice for one piece of work.
pub(crate) fn delegate(
    tx: &mut Tx,
    setup: &crate::ThreadSetup,
    thread: &str,
    call: &str,
    kind: &str,
    title: &str,
    task: &str,
) -> Result<Worker> {
    if let Some(id) = tx.doc::<String>(&delegated_key(thread, call))? {
        return load(tx, &id);
    }
    let turn = tx.thread(thread)?.turn();
    let index: Vec<String> = tx.doc(INDEX)?.unwrap_or_default();
    for id in &index {
        let other = load(tx, id)?;
        // A cancelled one has no report coming: the work can be asked for again.
        let coming = other.status != WorkerStatus::Cancelled;
        if coming && other.parent == thread && other.turn == turn && other.kind == kind {
            return Err(Rejected(format!(
                "{id} is already on this turn's {kind}: wait for its report instead"
            ))
            .into());
        }
    }
    let n: u64 = tx.doc(NEXT)?.unwrap_or(1);
    tx.put_doc(NEXT, &(n + 1))?;
    let id = format!("{PREFIX}{n}");
    setup.apply(tx, &id)?;
    let draft = Draft {
        text: task.to_string(),
        request_id: None,
        when_busy: WhenBusy::FollowUp,
    };
    inbox::submit(tx, &id, draft)?;
    let worker = Worker {
        id: id.clone(),
        parent: thread.to_string(),
        turn,
        kind: kind.to_string(),
        title: title.to_string(),
        task: task.to_string(),
        status: WorkerStatus::Running,
        started_at: tx.now(),
        ended_at: None,
        report: None,
        delivery: None,
    };
    save(tx, &worker)?;
    tx.put_doc(&delegated_key(thread, call), &id)?;
    let mut index = index;
    index.push(id.clone());
    tx.put_doc(INDEX, &index)?;
    tracing::info!(worker = %id, parent = thread, kind, task, "worker started");
    Ok(worker)
}

/// Every worker, oldest first.
pub(crate) fn all(engine: &engine::Engine) -> Result<Vec<Worker>> {
    let index: Vec<String> = engine.doc(INDEX)?.unwrap_or_default();
    let mut workers = Vec::new();
    for id in index {
        if let Some(worker) = engine.doc(&record_key(&id))? {
            workers.push(worker);
        }
    }
    Ok(workers)
}

/// The worker's run ended (`inbox::finish`): records how, and unless it was stopped, starts
/// taking its report back to the thread that asked for it.
pub(crate) fn ended(tx: &mut Tx, id: &str, settled: &Settled, aborted: bool) -> Result<()> {
    let mut worker = load(tx, id)?;
    if worker.status != WorkerStatus::Running {
        return Ok(());
    }
    worker.ended_at = Some(tx.now());
    (worker.status, worker.report) = match (settled, aborted) {
        (_, true) => (WorkerStatus::Cancelled, None),
        (Settled::Done { answer }, _) => (WorkerStatus::Done, Some(answer.clone())),
        (Settled::Unanswered { reason }, _) => (WorkerStatus::Failed, Some(reason.clone())),
    };
    tracing::info!(worker = id, status = ?worker.status, "worker ended");
    if worker.status != WorkerStatus::Cancelled {
        let input = serde_json::json!({ "worker": id });
        tx.create_task_named(REPORT, &worker.parent, input, TaskOptions::default())?;
    }
    save(tx, &worker)
}

pub type ReviewFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// Whether a report is worth giving the user now, though they have moved on from the thread
/// that asked for it (Jev, in `tokidoki`). Asked only when the rule says hold.
pub trait Review: Send + Sync + 'static {
    /// `now`: the user's latest messages, where they are now, oldest first.
    fn deliver<'a>(&'a self, worker: &'a Worker, now: &'a [String]) -> ReviewFuture<'a>;
}

/// Takes a worker's report back to the thread that asked for it, on that thread.
pub struct Report {
    review: Option<Arc<dyn Review>>,
    settings: Settings,
}

impl Report {
    pub fn new(review: Option<Arc<dyn Review>>, settings: Settings) -> Self {
        Self { review, settings }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportInput {
    pub worker: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ReportState {
    /// Decide, and deliver or hold, once the thread isn't answering.
    Review,
    /// Decided: carry it out once the thread isn't answering.
    Decided { deliver: bool },
}

/// How long a report waits for the thread to stop answering before looking again.
const BUSY_RETRY_MS: u64 = 2_000;
/// The user's latest messages a review is shown.
const REVIEW_MESSAGES: usize = 4;

impl Task for Report {
    type Input = ReportInput;
    type State = ReportState;
    type Output = Delivery;
    const NAME: &'static str = REPORT;

    fn initial(&self, _: &ReportInput) -> ReportState {
        ReportState::Review
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        let thread = rt.thread_id().to_string();
        let id = rt.input().worker.clone();
        let deliver = match *rt.state() {
            ReportState::Decided { deliver } => deliver,
            ReportState::Review => {
                // Read in a commit that writes nothing.
                let mut read = None;
                rt.commit(|tx| {
                    let worker = load(tx, &id)?;
                    let last: Option<String> = tx.doc(LAST_THREAD)?;
                    let active = tx.thread(&thread)?.checkpoints().last().map(|c| c.at);
                    let hot = active.is_some_and(|at| {
                        let idle = at.elapsed().unwrap_or_default().as_millis();
                        idle < u128::from(self.settings.dream_after_ms)
                    });
                    let now = match &last {
                        Some(last) => user_messages(tx.thread(last)?, REVIEW_MESSAGES),
                        None => Vec::new(),
                    };
                    let still_there = last.as_deref() == Some(thread.as_str()) && hot;
                    read = Some((worker, still_there, now));
                    Ok(None)
                })?;
                let (worker, still_there, now) = read.context("nothing read")?;
                match (still_there, &self.review) {
                    (true, _) => true,
                    (false, Some(review)) => review.deliver(&worker, &now).await,
                    (false, None) => false,
                }
            }
        };
        rt.commit(|tx| {
            // Not in the middle of an answer: a report waits for the thread to be free.
            if inbox::load(tx, &thread)?
                .run
                .is_some_and(|run| tx.is_live(run))
            {
                return Ok(Some(Next::Sleep {
                    until: tx.now() + BUSY_RETRY_MS,
                    state: ReportState::Decided { deliver },
                }));
            }
            let mut worker = load(tx, &id)?;
            let text = report_message(&worker, deliver);
            let delivery = match deliver {
                true => {
                    let draft = Draft {
                        text,
                        request_id: None,
                        when_busy: WhenBusy::FollowUp,
                    };
                    inbox::submit(tx, &thread, draft)?;
                    Delivery::Delivered
                }
                false => {
                    tx.thread(&thread)?.push(Message::user(text))?;
                    Delivery::Held
                }
            };
            worker.delivery = Some(delivery);
            save(tx, &worker)?;
            tracing::info!(worker = %id, thread = %thread, ?delivery, "worker report");
            Ok(Some(Next::Complete(delivery)))
        })
    }
}

/// The user's last `n` messages in `thread`, oldest first.
fn user_messages(thread: &state::Thread, n: usize) -> Vec<String> {
    let mut said: Vec<String> = thread
        .checkpoints()
        .iter()
        .rev()
        .filter_map(|c| match &c.event {
            state::Event::Message(m) if m.role == context::Role::User => {
                Some(m.text()).filter(|t| !t.trim().is_empty() && !t.starts_with(REPORT_OPEN))
            }
            _ => None,
        })
        .take(n)
        .collect();
    said.reverse();
    said
}

pub use experience::REPORT_OPEN;

/// A worker's report as the parent thread is given it, with what to do about it.
fn report_message(worker: &Worker, delivered: bool) -> String {
    let status = match worker.status {
        WorkerStatus::Done => "done",
        WorkerStatus::Failed => "failed",
        WorkerStatus::Cancelled | WorkerStatus::Running => "stopped",
    };
    let note = match (delivered, worker.status) {
        (true, WorkerStatus::Done) => {
            "Tell the user what came back: the answer first, briefly, in their language. They \
             asked for this; it isn't something they just said."
        }
        (true, _) => "Tell the user briefly that this couldn't be done, and why.",
        (false, _) => {
            "The user has moved on. Don't bring this up now; use it when they come back to it."
        }
    };
    format!(
        "{REPORT_OPEN} worker=\"{}\" kind=\"{}\" status=\"{status}\">\nTask: {}\n\n{}\n</worker-report>\n{note}",
        worker.id,
        worker.kind,
        // On one line, so where the task ends and the report starts is plain.
        worker.task.split_whitespace().collect::<Vec<_>>().join(" "),
        worker.report.as_deref().unwrap_or("(no report)"),
    )
}
