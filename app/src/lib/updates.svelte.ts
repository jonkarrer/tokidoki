import { core, type UpdateStatus } from "./core";

// The updater's status (src-tauri/src/updater.rs), shared by the card in the sidebar and the
// row in the settings.
export const updates = $state<{ status: UpdateStatus }>({
  status: { state: "idle" },
});

let watching = false;

/** Follow the status for the life of the page (asking twice does nothing more). */
export function watchUpdates() {
  if (watching) return;
  watching = true;
  // Subscribe first, then read, so no change slips in between.
  core
    .onUpdate((next) => (updates.status = next))
    .then(() => core.updateStatus())
    .then((now) => (updates.status = now));
}
