//! What goes in and comes out. How it travels is not tokidoki's concern: whatever carries it
//! (an app, a bot, a terminal) passes these plain serde values on as it sees fit.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who an input came from. Their memory scope follows from it, so the core decides who may
/// speak. The ID is whatever the code passing inputs in uses to tell people apart.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Speaker {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Speaker {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
        }
    }

    /// The one whose tokidoki it is.
    pub fn owner() -> Self {
        Self::new("owner")
    }
}

/// Bytes, inline or where to fetch them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Media {
    Base64(String),
    Url(String),
}

/// One piece of an input. The core takes text today; the other kinds are in the protocol so
/// they can be passed in, and are refused until the core can use them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: Media,
    },
    Audio {
        media_type: String,
        data: Media,
    },
    File {
        name: String,
        media_type: String,
        data: Media,
    },
}

impl Part {
    pub fn kind(&self) -> &'static str {
        match self {
            Part::Text { .. } => "text",
            Part::Image { .. } => "image",
            Part::Audio { .. } => "audio",
            Part::File { .. } => "file",
        }
    }
}

/// What to do with an input to a thread that is busy answering another.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WhenBusy {
    /// Wait for the current answer, then answer this.
    #[default]
    FollowUp,
    /// Join the current work after its tool round.
    Steer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Input {
    pub from: Speaker,
    pub parts: Vec<Part>,
    /// The thread to send it to; `None` lets the router choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The same ID finds the same submission instead of submitting twice (a retried send).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default)]
    pub when_busy: WhenBusy,
}

impl Input {
    pub fn text(from: Speaker, text: impl Into<String>) -> Self {
        Self {
            from,
            parts: vec![Part::Text { text: text.into() }],
            thread: None,
            request_id: None,
            when_busy: WhenBusy::default(),
        }
    }

    pub fn to(mut self, thread: impl Into<String>) -> Self {
        self.thread = Some(thread.into());
        self
    }
}

/// Why an input went where it did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Route {
    /// `rule`, `search`, `classifier` or `model`.
    pub by: String,
    pub reason: String,
    /// A new thread was started for it.
    pub new: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Sent {
    pub thread: String,
    pub submission: String,
    /// `None` when the input named its thread, or its request ID was already submitted.
    pub route: Option<Route>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Placed,
    Done,
    Unanswered,
}

/// An input and what became of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Submission {
    pub id: String,
    pub thread: String,
    pub text: String,
    pub status: Status,
    pub answer: Option<String>,
    /// Why it went unanswered.
    pub reason: Option<String>,
}

impl Submission {
    pub fn is_settled(&self) -> bool {
        matches!(self.status, Status::Done | Status::Unanswered)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThreadSummary {
    pub id: String,
    /// The start of its first message.
    pub title: String,
    pub turns: u32,
    /// What it is about, in tokidoki's words: written while the thread is idle, so a new
    /// thread has none yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Ms since the epoch.
    pub created_at: u64,
    /// The last message, ms since the epoch.
    pub active_at: u64,
    /// How warm it is, which is how readily a new input goes to it.
    pub phase: ThreadPhase,
    /// Where it left off, written when it last settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
}

/// A thread is hot while the user talks in it, cool once idle long enough to dream (a new
/// input then goes to it only when it plainly belongs there), and settled after a long idle,
/// when its context is reset to a short note of where it left off. Talking in it again makes
/// it hot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ThreadPhase {
    Hot,
    Cool,
    Settled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
    Thinking {
        text: String,
    },
}

/// A change to a thread, as recorded: every checkpoint is one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    TurnStarted,
    StepStarted,
    Message {
        role: Role,
        content: Vec<Content>,
    },
    TurnEnded,
    Routed {
        by: String,
        reason: String,
    },
    /// Memories brought into the conversation.
    Recalled {
        memories: Vec<String>,
    },
    /// Messages replaced: compaction, or a long-idle thread settled into its resume card.
    Compacted {
        replaced: usize,
        with: usize,
    },
    System {
        text: Option<String>,
    },
    ToolAdded {
        name: String,
    },
    ToolRemoved {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A thread changed. `seq` counts a thread's checkpoints from 0 with no gaps: a client
    /// that sees one out of order, or reconnects, catches up with `Tokidoki::history(thread,
    /// seq)`.
    Checkpoint {
        thread: String,
        seq: usize,
        /// Ms since the epoch.
        at: u64,
        turn: u32,
        change: Change,
    },
    /// An input's status changed.
    Submission { submission: Submission },
    /// Work started on a thread.
    Activity { thread: String, doing: Doing },
    /// Part of a reply, as the model writes it. Never recorded, so `history` has none: the
    /// reply arrives whole as a `Checkpoint` afterwards, and replaces what its deltas built.
    /// So does a new `Activity` `Thinking` on the thread: the request failed partway and is
    /// being made again.
    Delta { thread: String, delta: Delta },
    /// What tokidoki remembers changed: it learned something, corrected or dropped a memory
    /// (while answering, or tidying up while idle), or the user did.
    Memory { change: MemoryChange },
    /// A note was saved, changed or deleted: by the user, or by the model when asked.
    Note { change: NoteChange },
    /// A worker started, ended, or had its report delivered or held.
    Worker { worker: Worker },
    /// Events were missed (the client fell behind): re-read what it shows.
    Lagged,
}

/// Something tokidoki remembers, across every thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Memory {
    /// Names this version: a memory that is revised gets a new ID.
    pub id: i64,
    pub kind: MemoryKind,
    /// The memory, as one line.
    pub text: String,
    /// 1 when first learned; each revision adds one.
    pub version: u32,
    /// When this version was learned, ms since the epoch.
    pub at: u64,
    /// Where it was learned, when it came out of a conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// What is true: a fact or a preference.
    Fact,
    /// How something is done: steps worth repeating.
    Procedure,
}

/// The checkpoint a memory came from (`Event::Checkpoint::seq`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemorySource {
    pub thread: String,
    pub seq: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryChange {
    Learned {
        memory: Memory,
    },
    /// `memory` took the place of the memory `replaces`, which said `was`.
    Revised {
        memory: Memory,
        replaces: i64,
        was: String,
    },
    Forgotten {
        ids: Vec<i64>,
    },
    /// A thread's `ThreadSummary::about` was written.
    Described {
        thread: String,
        about: String,
    },
}

/// One of the user's notes: kept as written, apart from memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Note {
    pub id: i64,
    pub title: String,
    /// Markdown.
    pub body: String,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub people: Vec<String>,
    /// The thread it was saved from, when the model saved it there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// Ms since the epoch.
    pub created_at: u64,
    pub updated_at: u64,
}

/// A note as the user writes it. An empty title takes the body's first line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NoteDraft {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub people: Vec<String>,
}

/// Which notes to list. An empty query lists the most recently changed first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NoteSearch {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NoteChange {
    /// New, or changed.
    Saved {
        note: Note,
    },
    Deleted {
        id: i64,
    },
}

/// Work the model handed to a worker, which runs on its own and reports back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Worker {
    /// Like `worker-3`: its own thread, kept out of the conversation.
    pub id: String,
    /// The thread that asked for it.
    pub parent: String,
    /// What kind of worker: `research`.
    pub kind: String,
    /// What it was asked to do.
    pub task: String,
    pub status: WorkerStatus,
    /// Ms since the epoch.
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// What it found (Markdown), or why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// What became of the report: told to the user now, or kept for when they come back to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Delivered,
    Held,
}

/// What a reply grew by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Delta {
    Text { text: String },
    Thinking { text: String },
}

/// What tokidoki started doing on a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Doing {
    /// Recalling and retaining memories before answering.
    Remembering,
    /// Waiting on the model.
    Thinking,
    /// Running a tool the model asked for.
    Tool { name: String },
}

/// "That was wrong": recorded with the thread (and turn), for investigating later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Feedback {
    pub from: Speaker,
    pub thread: String,
    /// The turn it is about; `None` for the latest.
    #[serde(default)]
    pub turn: Option<u32>,
    pub text: String,
}
