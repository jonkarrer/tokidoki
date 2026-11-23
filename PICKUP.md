# Pickup

Written 2026-11-23, for whoever opens this repo next with no memory of the work so far.
`CLAUDE.md` holds the standing rules; this file is the state of play. Where it and the code
disagree, the code is right.

## Where things are

- **Branch:** `standalone-app`, pushed, ahead of `main` (`ed91e80`) by two commits of work and
  the one that adds this file.
  - `d7d9acb` removes everything built for ternt.
  - `d35b7d5` brings the macOS app back.
- **No pull request is open** and `main` hasn't been touched. The owner hasn't said whether
  to merge.
- **Nothing is uncommitted.**

## What tokidoki is

A personal-assistant agent in Rust with durable execution (work survives a crash and
resumes) and automatic long-term memory (it learns from conversations unprompted). The
owner's goal: "an alive agent that grows with the user". It is one library,
`tokidoki::Tokidoki`, plus a macOS app around it (`app/`, Tauri v2 + Svelte 5) that runs the
library in-process.

## How it got here

1. **2026-11-21:** a standalone app was built (shell, streaming, a memory view, a signing
   and self-update pipeline copied from ternt, the owner's browser app).
2. **2026-11-21 to 11-23:** the owner redirected tokidoki into ternt, as a sidecar process
   with one agent per ternt space. The app was retired.
3. **2026-11-23:** ternt took its own copy of the repo (at `4d3b18b`) and develops it there.
   Nothing is synced. The owner asked for the ternt parts to be removed here "so we can go
   back to a pure idea", then for the standalone app to come back. Both are done.

The removed ternt work (sidecar `tokidoki-serve`, `Spaces`, per-space configuration) is in
history at `ed91e80` if ever wanted.

## What works, and how it was checked

- **The core:** 152 workspace tests pass; `clippy -D warnings` is clean; the generated
  TypeScript protocol is current.
- **The app, seen running:**
  - A first launch asks for a provider and key.
  - A wrong key is refused in words by the provider.
  - A message is answered as it streams and its thread is listed.
  - Two memories were learned from one message and showed in the memory view.
  - The settings screen shows which providers have a key saved.
- **A real model:** the conversation above ran on Fireworks' cheapest model with the key
  from Doppler. Anthropic was verified live earlier, during the ternt work.

## How to look at the app

The shell can't capture the screen, so the app photographs its own page (debug builds only).

```sh
e2e/run.sh first-run                                               # no key needed
doppler run --project ternt --config dev -- e2e/run.sh conversation   # calls a model, cheapest
```

Each test is a script in `e2e/tests/` run inside the window's page with `t`
(`app/src/lib/debug.ts`: `click`, `type`, `see`, `until`, `shot`, `check`). It starts from an
empty data folder and ignores the Keychain. Pictures land in `e2e/.out/<name>.png` and can
be read as images. After changing the interface, run a test and look at the pictures.

Two things to know about the pictures: they show the page, not the window's title bar, and
only dark mode has been looked at.

## How the app is put together

- `app/src-tauri/src/lib.rs`: one Tauri command per library method; events go to the page as
  `tokidoki:event`.
- `keys.rs`: one key per provider in the macOS Keychain (service `com.jokar.tokidoki`,
  `.dev` in debug builds). A key in the environment wins.
- `settings.rs`: the chosen provider and an optional model name, as `settings.json` beside
  the databases.
- `debug.rs`: the test hook above.
- `updater.rs`: self-update from GitHub Releases.
- `app/src/lib/`: `Keys.svelte` (the provider picker), `Conversation.svelte`,
  `Memory.svelte`, `state.svelte.ts` (the page's state, kept in step by events).
- Data lives in `~/Library/Application Support/com.jokar.tokidoki` (`…/dev` for `just app`).

## Decisions already made

- **Bring your own key, macOS only.**
- **One provider does everything:** it answers, does the memory work and routes messages to
  threads. The user picks Anthropic, Fireworks or Baseten.
- **Without a key the core opens asleep:** conversations and memory can be read, memory can
  be edited, and only sending is refused.
- **Fireworks runs the flash model by default**, cheap by the owner's choice.
- **Tests that call a model use the cheapest one**, always (`TOKIDOKI_MODELS=cheap`).
- **It is a chat with memory for now.** Tools come later.

## Not done

- **Wipe memory** has a command (`wipe_memory`) and no button in the app.
- **Light mode and a narrow window** haven't been looked at.
- **Baseten is untested live:** there is no Baseten key in Doppler. Its default model name
  came from public docs, and its key check accepts anything Baseten doesn't plainly refuse.
- **The release pipeline has never run end to end.** `scripts/publish.sh` signs, notarizes
  and uploads to `jonkarrer/tokidoki-dist`, which didn't exist when last checked. The
  updater's signing key is at `~/.tauri/tokidoki.key`; Apple's notarization secrets are in
  ternt's Doppler project. See `docs/RELEASING.md`.
- **The journal still supports several agents in one process**, left over from ternt's
  spaces. It works unchanged with one. The owner was offered its removal and hasn't answered.
- **The older roadmap,** none of it started: tools with approvals, proactivity (the agent
  speaking first), a menu-bar presence, non-text inputs.

## What I'd do next

Confirm each with the owner first; they work one step at a time.

1. Merge `standalone-app` into `main`, if they want it there.
2. Add the wipe-memory button, and look at light mode and a narrow window.
3. Run the release pipeline once, end to end, to a real notarized build.
4. Then the roadmap: tools with approvals is the next real feature.

## Working with the owner

- One piece, reported, then the next. They change direction often, so don't build ahead.
- Expect a review and a simplify pass after a chunk of work.
- Commit and push only when asked. Work on a branch, not `main`.
- Never print a key or put one on a command line; keys come from Doppler or the Keychain.
