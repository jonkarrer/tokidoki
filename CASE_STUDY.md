# Tokidoki: a case study

Written 2026-11-24, six days after the first commit. It covers what the project set out to
do, the choices that shaped it, what went wrong along the way, and what is still unproven.
It is drawn from the git history, the code, the project's own notes (`CLAUDE.md`,
`docs/RELEASING.md`) and the working sessions that produced them. Where the
record doesn't say why something was done, this says so instead of guessing.

## The brief

The owner's goal, in their words: "an alive agent that grows with the user". Concretely, a
personal assistant with two properties most chat apps lack:

- **Durable execution.** Work survives a crash and resumes where it stopped.
- **Automatic long-term memory.** It learns from conversations without being told to.

It is built as one Rust library, `tokidoki::Tokidoki` (input in, output out), with a macOS
app around it (Tauri v2 and Svelte 5).

## At a glance

| | |
|---|---|
| First commit | 2026-11-19 |
| Commits | 30 |
| Crates | 16 |
| Rust | about 24,000 lines |
| App front end | about 3,700 lines |
| Tests | 190 in the workspace, plus scripted runs of the app |
| Released | not yet; a release dry run passed on 2026-11-24 |

## Timeline

| Date | What happened |
|---|---|
| Nov 19 | Inference: a provider abstraction built around the providers' compatible wire types. |
| Nov 20 | The core in one day: context, state, memory, router, engine, telemetry, the protocol and the library. A first review and simplify pass the same day. |
| Nov 21 | A standalone app, with a signing and self-update pipeline copied from ternt, the owner's browser app. The same day the project was redirected into ternt as a sidecar process, one agent per ternt space, and the app was retired. |
| Nov 22 | Memory can be wiped. |
| Nov 23 | Ternt took its own copy. The ternt-specific parts were removed here "so we can go back to a pure idea", and the app came back. Then, the same day: one conversation over many threads, Jev for quick decisions, workers and web research, notes, and local voice. Two review rounds, twenty findings fixed. |
| Nov 24 | Release preparation: a dry run of the pipeline, and checking for updates from the menu and the settings. |

## Design choices and why

### The log is the truth; the context is rebuilt

A thread is an append-only log of checkpoints. What the model sees is never stored: it is
rebuilt from the log each time. Dropping something from the context to save space therefore
never loses it from history, and compaction is safe to get wrong.

### One line of atomic commits

Thread checkpoints and task states are written together in one SQLite transaction, or not
at all, and nothing is shown before it is stored. All work runs as tasks: state machines
that commit at every step. The design follows pi-durable. The rules for what happens after
a crash are explicit, not left to chance:

- A model request cut off by a crash counts as a failed attempt and is retried after a
  backoff that itself survives a restart. It is not silently sent again.
- A tool call's intent is committed before it runs. If it is cut off, only a tool marked
  safe to replay runs again; any other gives the model an "interrupted" result.
- A task that keeps crashing the process is faulted after a set number of restarts, so one
  bad piece of work can't loop forever.

### Memory in layers, with a cheap trigger in front

Three crates share the memory work, each with one job:

- **Attention** finds triggers. It runs before every model call, so it is plain synchronous
  code (patterns, structure, counts) with no model calls and no I/O.
- **Experience** decides what to do about a trigger and carries it out: recall, retain,
  refine. While a thread is idle it "dreams": summarises episodes, tidies memory, compacts
  the context.
- **Memory** is passive storage: facts and preferences, ways of doing things, and summaries
  of what happened. Items are versioned, so a correction adds a version instead of erasing
  the past.

One rule holds across all of it: nothing in the memory work may break a turn. A decision
that fails becomes "ignore", with the reason recorded.

### Deciding cheaply first

Every input has to be filed into a thread, and a model call on every input would be slow
and expensive. The router decides in tiers and stops at the first one that is sure: pattern
cues, then a fast classifier, then word overlap and keyword search over thread summaries,
and a model call only for what is left. It reads short "cards" describing threads, not the
threads themselves.

### Jev for the quick decisions

Jev is TypeSafe's structured-decision model: it is sent facts and named questions, and
returns probabilities instead of text, in a few hundred milliseconds. Tokidoki uses it to
route inputs, to judge what to recall and what is worth remembering, to check whether a
thread's summary still holds, and to decide whether a worker's report should interrupt.

Three choices keep it safe to depend on:

- **Jev decides, the model writes.** Jev picks and places; the wording of a memory is still
  the model's.
- **The version is pinned,** because thresholds tuned on one version may not hold on the
  next.
- **Everything works without it.** Each use has a fallback, and a request that stalls times
  out after five seconds.

Jev never sees the whole conversation or the whole memory store. Candidates are chosen
first by keyword search and recency (up to 30 memories, the last few messages, up to 10
threads by their cards), and Jev judges those.

### One conversation over many threads

The user sees one continuous conversation. Underneath, messages are filed into threads by
subject, and the threads are a view "under the hood". The reason for the design isn't
written down; the effect is that the user never has to pick or name a chat, while each
subject still gets a focused context.

### Workers report back as messages, but are not the user

Long work such as web research is handed to a worker, which reports when done. A report
arrives in the thread as a message, and Jev decides whether to deliver it now or hold it.
This produced the most instructive bug of the project (see below): a report travels as a
user message, so every place that reasons about "what the user said" had to learn to tell
the two apart.

### Notes are the user's; memory is the agent's

Memory is what Tokidoki learns and tidies on its own. A note is kept exactly as the user
wrote it, saved only when they ask, and changed only by them. Keeping the two apart means
the agent's housekeeping can never rewrite something the user meant to keep verbatim.

### Voice stays on the Mac

Speech to text (Whisper large-v3-turbo) and text to speech (Kokoro) both run locally;
nothing is sent anywhere. The models are downloaded once, on first use.

### Bring your own key, macOS only

The user picks one provider (Anthropic, Fireworks or Baseten) and pastes its key, which is
kept in the Keychain. Without a key the app opens "asleep": conversations and memory can be
read and memory edited, and only sending is refused. Keys never go in a file in the repo,
are never printed, and never appear on a command line.

### Built to be debugged

Every model call (the exact body sent and the raw reply), routing decision, memory decision
and task change is recorded to a journal beside the database. A moment described in words
("it forgot I'm vegetarian") can be found, laid out step by step, and its model calls sent
again. The journal can also stand in for the provider, so a run can be repeated without
calling a model.

### The app photographs itself

There is no screen to capture from a shell, so a debug build runs a script inside its own
page that clicks and types as a user would, and the web view takes pictures of itself. This
is how interface changes were checked throughout, without a person watching.

## Obstacles

### The detour through ternt

Two days in, the project was redirected to run inside ternt as a sidecar. That work was
real: a line protocol, one agent per space, per-space provider configuration, and a review
pass that found a request left unanswered after a panic and a space that could reopen
during shutdown. Then ternt took its own copy and all of it was removed here. What
survived the round trip was worth having: streaming for all three providers, opening
without a key, the memory surface, and the generated TypeScript protocol. The removed parts
remain in history.

### A default model that didn't exist

The first Fireworks default returned 404 on every call, because the model name wasn't in
the account's list. The fix came with a live test that checks the name, and with the
owner's rule that followed: tests that call a model always run on the cheapest one.

### Reports mistaken for the user

Because a worker's report is stored as a user message, the first version judged and learned
from reports as if the user had said them, and a held report could be lost when its thread
settled. The fix was one shared definition of what a report is, used by every place that
asks what the user said.

### A microphone that could stay on

The first voice version could open a second audio stream if the key was pressed again while
permission was being asked, and treated a held key as many presses. Recordings longer than
Whisper hears at once were cut off; they are now split at the quietest moment near each
boundary and transcribed in pieces. Replies are no longer read out over the user talking.

### A test that failed by the millisecond

A timeline test failed on the main branch because two turns were started in the same
millisecond and tied. The test was spaced out; the ordering rule itself was left alone.

### Nix compilers and release builds

A release binary must link only system libraries, and a Nix toolchain links libraries other
Macs don't have. The publish script forces Apple's compiler and linker whatever shell it
runs from, then checks the result.

### What can't be checked from here

Some things were built without being seen working, and are recorded as such instead of
assumed: voice end to end (the models aren't on the development Mac), Baseten against a
real key, light mode, and the native menu item, which the test script can't click.

## How the work was done

- **One piece, reported, then the next.** The owner changes direction often, so nothing was
  built ahead.
- **A review and a simplify pass after each chunk.** Two review rounds on Nov 23 found ten
  faults each; all twenty were fixed.
- **Commit only when asked.**
- **Prose over jargon in the code.** Every crate opens by saying what it is for, comments
  say why, and test names are sentences (`a_cancelled_worker_reports_nothing`).
- **Verified, or said not to be.** Each change lists what was checked and what wasn't.

## Releasing

The pipeline follows ternt's: a universal app (Apple Silicon and Intel), signed with a
Developer ID and notarized, in a DMG for new installs, with a signed update archive and a
`latest.json` on GitHub Releases for installed copies. Installed copies check shortly after
launch, every 15 minutes, and on returning to the window; an update downloads in the
background and installs on quit, with a card in the sidebar offering to restart now. A
check can also be asked for from the app menu, the settings, or the version in the sidebar.

On 2026-11-24 the dry run built the universal app with voice included (about 96 MB; a 39 MB
DMG and a 35 MB update archive), linking only system libraries. Signing, notarization and
upload have not run yet: they wait on the repo being made public and a token to upload with.

A storage bucket was considered in place of GitHub Releases and set aside for now: it would
work, but needs tools and credentials this Mac doesn't have and a rewrite of the publish
step, and it gives no release page.

## Still open

- **The first real release,** and an update from one release to the next.
- **Voice, heard working** on a Mac with the models.
- **Crash-safe update downloads and required updates,** which ternt has and Tokidoki
  doesn't; deliberately left until a first release works.
- **Baseten live, light mode, a narrow window.**
- **The older roadmap:** approvals for tools, the agent speaking first, a menu-bar presence,
  inputs other than text and voice.
## What this project suggests

- **Durability is cheaper designed in than added.** Committing a tool's intent before it
  runs, and deciding up front what a crash means for each kind of work, cost little on day
  two and would be a rewrite on day twenty.
- **A cheap decision layer changes what is affordable.** Judging every message for recall
  and retention is only reasonable because the judge costs a fraction of a cent and has a
  fallback.
- **A shortcut in the data model is paid for everywhere it is read.** Storing reports as
  user messages was simple to write and wrong in five places.
- **A detour can pay if the general parts are kept.** The ternt work was removed, but the
  app that came back was better for it.
- **A tool that can see its own interface removes a person from the loop,** and makes it
  honest to say which parts have and haven't been looked at.
