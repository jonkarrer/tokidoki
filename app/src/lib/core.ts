// The bridge to the core (src-tauri/src/lib.rs): one function per command, taking and
// returning the `interface` crate's types.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Event, Memory, MemoryChange, Sent, Submission, ThreadSummary } from "../../../crates/interface/protocol";

// The protocol's types are generated from the Rust (`just types`), never written here.
export type {
  Change,
  Content,
  Doing,
  Memory,
  MemoryChange,
  Sent,
  Submission,
  ThreadSummary,
} from "../../../crates/interface/protocol";

export type CoreEvent = Event;
export type Checkpoint = Extract<Event, { type: "checkpoint" }>;

/** Where the app is (src-tauri/src/lib.rs). */
export type Status = { state: "needs_keys" } | { state: "ready" } | { state: "failed"; message: string };

export type UpdateStatus =
  | { state: "idle" | "up_to_date" }
  | { state: "ready" | "installing"; version: string }
  | { state: "failed"; message: string };

/** A provider the user can choose (src-tauri/src/lib.rs). */
export type ProviderChoice = { id: string; name: string; keys_at: string; model: string; has_key: boolean };
export type Chosen = { provider: string; model: string | null; providers: ProviderChoice[] };

export const core = {
  status: () => invoke<Status>("status"),
  chosen: () => invoke<Chosen>("chosen"),
  /** Runs on `provider` from now on. A `null` key keeps the one there is; a `null` model is the provider's usual. */
  choose: (provider: string, key: string | null, model: string | null) =>
    invoke<Status>("choose", { provider, key, model }),
  send: (text: string, thread: string | null) =>
    invoke<Sent>("send", { text, thread, steer: false, requestId: crypto.randomUUID() }),
  threads: () => invoke<ThreadSummary[]>("threads"),
  history: (thread: string, since: number) => invoke<Checkpoint[]>("history", { thread, since }),
  abort: (thread: string) => invoke<boolean>("abort", { thread }),
  memories: () => invoke<Memory[]>("memories"),
  remember: (text: string) => invoke<Memory>("remember", { text }),
  reviseMemory: (id: number, text: string) => invoke<Memory>("revise_memory", { id, text }),
  forgetMemory: (id: number) => invoke<void>("forget_memory", { id }),
  wipeMemory: () => invoke<void>("wipe_memory"),
  onEvent: (handler: (event: CoreEvent) => void) => listen<CoreEvent>("tokidoki:event", (e) => handler(e.payload)),

  version: () => invoke<string>("app_version"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkForUpdates: () => invoke<void>("check_for_updates"),
  installUpdate: () => invoke<void>("install_update"),
  onUpdate: (handler: (status: UpdateStatus) => void) =>
    listen<UpdateStatus>("update:status", (e) => handler(e.payload)),
};
