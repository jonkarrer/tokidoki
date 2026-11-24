// The bridge to the core (src-tauri/src/lib.rs): one function per command, taking and
// returning the `interface` crate's types.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type {
  Event,
  Memory,
  MemoryChange,
  Note,
  NoteDraft,
  NoteSearch,
  Sent,
  Submission,
  ThreadSummary,
  VoiceStatus,
  Heard,
  Worker,
} from "../../../crates/interface/protocol";

// The protocol's types are generated from the Rust (`just types`), never written here.
export type {
  Change,
  Content,
  Doing,
  Memory,
  MemoryChange,
  Note,
  NoteChange,
  NoteDraft,
  NoteSearch,
  Sent,
  Submission,
  ThreadPhase,
  ThreadSummary,
  VoiceStatus,
  Heard,
  Worker,
} from "../../../crates/interface/protocol";

export type CoreEvent = Event;
export type Checkpoint = Extract<Event, { type: "checkpoint" }>;

/** Where the app is (src-tauri/src/lib.rs). */
export type Status = { state: "needs_keys" } | { state: "ready" } | { state: "failed"; message: string };

export type UpdateStatus =
  | { state: "idle" | "checking" | "up_to_date" }
  | { state: "ready" | "installing"; version: string }
  | { state: "failed"; message: string };

/** A provider the user can choose (src-tauri/src/lib.rs). */
export type ProviderChoice = { id: string; name: string; keys_at: string; model: string; has_key: boolean };
export type Chosen = {
  provider: string;
  model: string | null;
  providers: ProviderChoice[];
  has_jev_key: boolean;
  has_exa_key: boolean;
};

export const core = {
  status: () => invoke<Status>("status"),
  chosen: () => invoke<Chosen>("chosen"),
  /** Runs on `provider` from now on. A `null` key keeps the one there is; a `null` model is the provider's usual.
      `jevKey` (TypeSafe, for Jev) and `exaKey` (for research workers): `null` keeps the one there is. */
  choose: (provider: string, key: string | null, model: string | null, jevKey: string | null, exaKey: string | null) =>
    invoke<Status>("choose", { provider, key, model, jevKey, exaKey }),
  send: (text: string, thread: string | null) =>
    invoke<Sent>("send", { text, thread, steer: false, requestId: crypto.randomUUID() }),
  threads: () => invoke<ThreadSummary[]>("threads"),
  history: (thread: string, since: number) => invoke<Checkpoint[]>("history", { thread, since }),
  /** Every thread's last `turns` turns that started before `before` (ms), as one conversation. */
  timeline: (before: number | null, turns: number) => invoke<Checkpoint[]>("timeline", { before, turns }),
  abort: (thread: string) => invoke<boolean>("abort", { thread }),
  memories: () => invoke<Memory[]>("memories"),
  remember: (text: string) => invoke<Memory>("remember", { text }),
  reviseMemory: (id: number, text: string) => invoke<Memory>("revise_memory", { id, text }),
  forgetMemory: (id: number) => invoke<void>("forget_memory", { id }),
  wipeMemory: () => invoke<void>("wipe_memory"),
  notes: (search: NoteSearch) => invoke<Note[]>("notes", { search }),
  /** A new note, or with `id` a change to one. */
  writeNote: (id: number | null, draft: NoteDraft) => invoke<Note>("write_note", { id, draft }),
  deleteNote: (id: number) => invoke<void>("delete_note", { id }),
  /** Every worker there has been, latest first. */
  workers: () => invoke<Worker[]>("workers"),
  cancelWorker: (id: string) => invoke<boolean>("cancel_worker", { id }),
  /** Where the voice models stand; `null` when there is no voice. */
  voiceStatus: () => invoke<VoiceStatus | null>("voice_status"),
  /** Downloads the voice models; progress comes as `voice` events. */
  prepareVoice: () => invoke<void>("prepare_voice"),
  /** Loads the voice models while the user talks, so hearing them doesn't wait. */
  warmVoice: () => invoke<void>("warm_voice"),
  /** What a WAV recording says. */
  listen: (wav: Uint8Array) => invoke<Heard>("listen", wav),
  /** A reply read out in `language`, as WAV bytes. */
  speak: (text: string, language: string) => invoke<ArrayBuffer>("speak", { text, language }),
  onEvent: (handler: (event: CoreEvent) => void) => listen<CoreEvent>("tokidoki:event", (e) => handler(e.payload)),

  version: () => invoke<string>("app_version"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkForUpdates: () => invoke<void>("check_for_updates"),
  installUpdate: () => invoke<void>("install_update"),
  onUpdate: (handler: (status: UpdateStatus) => void) =>
    listen<UpdateStatus>("update:status", (e) => handler(e.payload)),
};
