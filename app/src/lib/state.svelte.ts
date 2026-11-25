// What the window shows, kept in step with the core by its events.
//
// There is one conversation. Under it the core keeps threads, one per subject, and files
// each message into one (the router); the conversation is every thread's turns as one, in
// the order they started (`log`, the core's timeline). Everything shown is derived from those
// checkpoints, so what is on screen is what is stored. The one exception is a reply being
// written (`live`), which the reply's own checkpoint replaces when it arrives.
import {
  core,
  type Checkpoint,
  type CoreEvent,
  type Doing,
  type Memory,
  type MemoryChange,
  type Note,
  type NoteChange,
  type Status,
  type Submission,
  type ThreadSummary,
  type Worker,
} from "./core";

/** What Tokidoki is doing on a thread right now. `writing` is a reply arriving. */
export type Presence = Doing | { kind: "writing" } | null;

/** Turns read at a time, now and each time the user scrolls back for more. */
const PAGE = 40;

export const app = $state({
  status: null as Status | null,
  /** What the main area shows. */
  view: "conversation" as
    | "conversation"
    | "memory"
    | "notes"
    | "research"
    | "missions"
    | "threads"
    | "keys",
  /** The threads under the conversation, most recently active first. */
  threads: [] as ThreadSummary[],
  /** The thread the Threads view shows. */
  thread: null as string | null,
  /** Everything Tokidoki remembers, newest first. */
  memories: [] as Memory[],
  /** Memories learned or revised since the memory view was last open. */
  unseen: 0,
  /** The user's notes, most recently changed first. */
  notes: [] as Note[],
  /** The conversation: every thread's checkpoints, turn by turn. */
  log: [] as Checkpoint[],
  /** Whether there are earlier turns than `log` holds. */
  earlier: false,
  /** Per thread: what it's doing, and the reply so far. */
  live: {} as Record<string, { doing: Presence; text: string }>,
  /** Inputs waiting for the answer under way. */
  queued: [] as Submission[],
  /** Why the last input got no answer. */
  unanswered: null as string | null,
  /** Work handed to workers, latest first: running, or come back. */
  workers: [] as Worker[],
  /** A worker's own thread, by its id, while its page is open: every step it took. */
  traces: {} as Record<string, Checkpoint[]>,
  /** Why a worker's thread couldn't be read, by its id. */
  traceErrors: {} as Record<string, string>,
  /** Why the last send didn't go through. */
  sendError: null as string | null,
});

/** Per thread: the last checkpoint `log` holds, to tell a missed one from the next. */
let seen: Record<string, number> = {};
let listening = false;

// Hears each reply as it arrives (its line's key and text): the voice reads them out.
let replied: ((key: string, text: string) => void) | null = null;
export function whenReplied(hear: (key: string, text: string) => void) {
  replied = hear;
}

/** Asks the core where things stand (it opens if it can), and follows it from then on. */
export async function start() {
  if (!listening) {
    listening = true;
    await core.onEvent(onEvent);
  }
  app.status = await core.status();
  // Without a key it is asleep, and what it has can still be read.
  if (app.status.state !== "failed")
    await Promise.all([
      loadThreads(),
      loadMemories(),
      loadNotes(),
      loadConversation(),
      loadWorkers(),
    ]);
  if (app.status.state === "needs_keys") app.view = "keys";
}

/** Whether there is anything to show yet: conversations, memories or notes from before. */
export function hasHistory() {
  return (
    app.threads.length > 0 || app.memories.length > 0 || app.notes.length > 0
  );
}

export async function loadMemories() {
  app.memories = await core.memories();
}

export async function loadNotes() {
  app.notes = await core.notes({ query: "", tag: null, project: null });
}

export async function loadWorkers() {
  app.workers = await core.workers();
}

/** Reads a worker's thread from the start; what it does from now on follows (`receive`). */
export async function trace(worker: string) {
  try {
    const read = await core.history(worker, 0);
    app.traces[worker] = read;
    delete app.traceErrors[worker];
  } catch (error) {
    // Said on the mission's page: an empty trace must never look like "it did nothing".
    app.traceErrors[worker] = String(error);
  }
}

/** Stops a worker: what it was looking into never comes back. */
export function cancelWorker(id: string) {
  return core.cancelWorker(id);
}

export async function loadThreads() {
  app.threads = await core.threads();
}

export function show(view: typeof app.view) {
  app.view = view;
  if (view === "memory") app.unseen = 0;
}

/** The thread `id`, under the hood: its own history, in the Threads view. */
export function showThread(id: string) {
  app.thread = id;
  show("threads");
}

/** The latest turns, as the conversation starts. */
async function loadConversation() {
  const log = await core.timeline(null, PAGE);
  app.log = log;
  app.earlier = turns(log) === PAGE;
  seen = {};
  for (const checkpoint of log)
    seen[checkpoint.thread] = Math.max(
      seen[checkpoint.thread] ?? -1,
      checkpoint.seq,
    );
  // A checkpoint that arrived while that was read went to the log it replaced, and no later
  // one may come to show the gap: ask each thread for what follows.
  await Promise.all(Object.keys(seen).map(catchUp));
}

/** The turns before the ones shown. */
export async function loadEarlier() {
  const first = app.log.find(
    (checkpoint) => checkpoint.change.kind === "turn_started",
  );
  if (!first) return;
  const older = await core.timeline(first.at, PAGE);
  app.earlier = turns(older) === PAGE;
  app.log = [...older, ...app.log];
}

function turns(log: Checkpoint[]) {
  return log.filter((checkpoint) => checkpoint.change.kind === "turn_started")
    .length;
}

/** Takes a thread's checkpoint if it is the next one; after a gap, reads what was missed. */
function receive(checkpoint: Checkpoint) {
  // A worker's own thread isn't the conversation: only its report comes back to it.
  if (checkpoint.thread.startsWith("worker-")) {
    // Its page, when open, shows each step as it is taken.
    const steps = app.traces[checkpoint.thread];
    const next = (steps?.at(-1)?.seq ?? -1) + 1;
    if (steps && checkpoint.seq === next) steps.push(checkpoint);
    else if (steps && checkpoint.seq > next) trace(checkpoint.thread);
    return;
  }
  const last = seen[checkpoint.thread];
  // A thread not shown yet starts here: what it said before is earlier than the conversation.
  if (last === undefined || checkpoint.seq === last + 1) {
    app.log.push(checkpoint);
    seen[checkpoint.thread] = checkpoint.seq;
  } else if (checkpoint.seq > last + 1) catchUp(checkpoint.thread);
}

async function catchUp(thread: string) {
  const more = await core.history(thread, (seen[thread] ?? -1) + 1);
  // Events kept arriving meanwhile: take only what still comes next.
  for (const checkpoint of more)
    if (checkpoint.seq === (seen[thread] ?? -1) + 1) receive(checkpoint);
}

/** Sends a message; the core files it with the thread it belongs to. */
export async function send(text: string) {
  app.sendError = null;
  try {
    await core.send(text, null);
    app.unanswered = null;
    return true;
  } catch (error) {
    app.sendError = String(error);
    return false;
  }
}

/** Stops whatever is being answered. */
export function stop() {
  for (const thread of answering(app.log)) core.abort(thread);
}

function live(thread: string) {
  return (app.live[thread] ??= { doing: null, text: "" });
}

function onEvent(event: CoreEvent) {
  switch (event.type) {
    case "checkpoint": {
      const { thread, change } = event;
      if (change.kind === "message" && change.role === "assistant") {
        live(thread).text = "";
        const text = change.content
          .flatMap((part) => (part.type === "text" ? [part.text] : []))
          .join("\n\n")
          .trim();
        if (text && !thread.startsWith("worker-"))
          replied?.(`${thread}:${event.seq}`, text);
      }
      if (change.kind === "turn_ended") {
        app.live[thread] = { doing: null, text: "" };
        loadThreads();
      }
      if (change.kind === "turn_started") loadThreads();
      receive(event);
      break;
    }
    case "activity": {
      const now = live(event.thread);
      now.doing = event.doing;
      // A new request: whatever an earlier attempt wrote is gone.
      if (event.doing.kind === "thinking") now.text = "";
      break;
    }
    case "delta": {
      if (event.delta.kind !== "text") break;
      const now = live(event.thread);
      now.doing = { kind: "writing" };
      now.text += event.delta.text;
      break;
    }
    case "submission": {
      const { submission } = event;
      // A worker's task isn't the user's input: how it went comes back as its report.
      if (submission.thread.startsWith("worker-")) break;
      app.queued = app.queued.filter((queued) => queued.id !== submission.id);
      if (submission.status === "queued") app.queued.push(submission);
      // Stopping an answer is the user's doing, not a failure to report.
      if (submission.status === "unanswered" && submission.reason !== "aborted")
        app.unanswered = submission.reason ?? "";
      break;
    }
    case "memory":
      onMemory(event.change);
      break;
    case "worker": {
      const { worker } = event;
      app.workers = [
        worker,
        ...app.workers.filter((w) => w.id !== worker.id),
      ].sort((a, b) => b.started_at - a.started_at);
      break;
    }
    case "note":
      onNote(event.change);
      break;
    case "lagged":
      loadConversation();
      loadThreads();
      loadMemories();
      loadNotes();
      loadWorkers();
      break;
  }
}

/** Keeps the list in step with the store: each memory once, newest first. */
function onMemory(change: MemoryChange) {
  switch (change.kind) {
    case "learned":
    case "revised": {
      const gone =
        change.kind === "revised" ? change.replaces : change.memory.id;
      app.memories = [
        change.memory,
        ...app.memories.filter(
          (m) => m.id !== gone && m.id !== change.memory.id,
        ),
      ];
      if (app.view !== "memory") app.unseen += 1;
      break;
    }
    case "forgotten":
      app.memories = app.memories.filter((m) => !change.ids.includes(m.id));
      break;
    case "described": {
      const thread = app.threads.find((t) => t.id === change.thread);
      if (thread) thread.about = change.about;
      break;
    }
  }
}

/** Keeps the list in step with the store: each note once, most recently changed first. */
function onNote(change: NoteChange) {
  const others = app.notes.filter(
    (n) => n.id !== (change.kind === "saved" ? change.note.id : change.id),
  );
  app.notes = change.kind === "saved" ? [change.note, ...others] : others;
}

/** A line of the conversation, as shown, and the thread it is in. A user's line that starts
    another thread than the one before it is `moved`: the talk changed subject there. A
    worker's report is given to its thread as if said there (agent's `workers.rs`), but is
    shown as what it is. */
export type Item = { key: string; thread: string } & (
  | { kind: "user"; text: string; moved: boolean }
  | { kind: "reply"; text: string }
  | { kind: "tool"; name: string }
  | { kind: "recalled"; memories: string[] }
  | { kind: "learned"; memory: Memory }
  | { kind: "compacted" }
  | { kind: "report"; report: Report }
);

/** A worker's report, as its thread was given it. */
export type Report = {
  worker: string;
  kind: string;
  status: string;
  task: string;
  body: string;
};

/** The report a message holds, if it is one. */
export function report(text: string): Report | null {
  const found = text.match(
    /^<worker-report worker="([^"]*)" kind="([^"]*)" status="([^"]*)">\nTask: ([\s\S]*?)\n\n([\s\S]*?)\n<\/worker-report>/,
  );
  if (!found) return null;
  const [, worker, kind, status, task, body] = found;
  return { worker, kind, status, task, body };
}

/** The conversation `log` makes, with what was learned in it (`learned`) where it was learned. */
export function items(log: Checkpoint[], learned: Memory[]): Item[] {
  const out: Item[] = [];
  let last: string | null = null;
  for (const { thread, seq: n, change } of log) {
    const key = `${thread}:${n}`;
    if (change.kind === "recalled" && change.memories.length) {
      out.push({ kind: "recalled", key, thread, memories: change.memories });
    } else if (change.kind === "compacted") {
      out.push({ kind: "compacted", key, thread });
    } else if (change.kind === "message") {
      const text = change.content
        .flatMap((part) => (part.type === "text" ? [part.text] : []))
        .join("\n\n")
        .trim();
      const came = change.role === "user" ? report(text) : null;
      if (came) {
        out.push({ kind: "report", key, thread, report: came });
      } else if (text && change.role === "user") {
        out.push({
          kind: "user",
          key,
          thread,
          text,
          moved: last !== null && last !== thread,
        });
        last = thread;
      } else if (text) out.push({ kind: "reply", key, thread, text });
      for (const part of change.content)
        if (part.type === "tool_call")
          out.push({
            kind: "tool",
            key: `${key}-${part.id}`,
            thread,
            name: part.name,
          });
    }
    for (const memory of learned)
      if (memory.source?.thread === thread && memory.source.seq === n)
        out.push({ kind: "learned", key: `m${memory.id}`, thread, memory });
  }
  return out;
}

/** The threads answering now: their last turn hasn't ended. */
export function answering(log: Checkpoint[]): string[] {
  const open = new Map<string, boolean>();
  for (const { thread, change } of log) {
    if (change.kind === "turn_started") open.set(thread, true);
    if (change.kind === "turn_ended") open.set(thread, false);
  }
  return [...open].filter(([, busy]) => busy).map(([thread]) => thread);
}
