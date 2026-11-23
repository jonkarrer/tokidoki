# Tokidoki

A personal-assistant agent in Rust: durable execution (work survives a crash and resumes) and
automatic long-term memory (it learns from conversations without being told to). The owner's
goal is "an alive agent that grows with the user". It is one library, `tokidoki::Tokidoki`:
input in, output out, and a macOS app around it (`app/`, Tauri v2 + Svelte 5).

## History worth knowing

- From 2026-11-21 to 2026-11-23 this repo was shaped to run inside **ternt**, the owner's
  browser app, as a sidecar process with one agent per ternt space. Ternt then took its own
  copy (at `4d3b18b`, in the ternt repo) and develops it there. Nothing is synced. The
  ternt-specific parts were removed here afterwards: the sidecar (`tokidoki-serve`), `Spaces`,
  per-space configuration, the space name told to new threads, and the handoff docs. They are
  in history at `ed91e80` if ever wanted.
- The app (signed and self-updating the way ternt is) was first built on 2026-11-21, retired
  when ternt took over, and brought back on 2026-11-23 with a provider picker. Its release
  pipeline (`scripts/publish.sh`, `docs/RELEASING.md`) has never run end to end.

## Standing decisions

- **Bring your own key, macOS only.** The user picks one provider (Anthropic, Fireworks or
  Baseten) and gives its key; that provider answers, does the memory work and routes. Keys
  never go in a file in the repo.
- **Without a key it opens asleep** (`Tokidoki::open_asleep`): threads, history and memory can
  be read and memory edited, unfinished work waits, and only `send` is refused.
- **Tools come later.** It is a chat with memory for now.

## Working with the owner

Step by step: one piece, reported, then the next. Expect a review and a simplify pass after a
chunk of work. Commit only when asked. Secrets come from Doppler or the Keychain; never print
a key or put one on a command line.

## Tests that call a model

Always on a very cheap model (the owner's rule): `claude-haiku-5-5`, `deepseek-v4p1-flash`,
`DeepSeek-V4.1-Flash`, which `Config::cheap_models` names and `TOKIDOKI_MODELS=cheap` selects.
A live test never names a provider's default model.

## Style

Every `lib.rs` opens with what the crate is for. Comments are plain prose saying why. Test
names are sentences (`a_reply_is_shown_as_it_is_written`). `cargo fmt` the files you touch
(edition 2024).

## Commands

```sh
just check     # fmt, clippy -D warnings, protocol.ts check
just test      # all tests; `just test inference --live` hits real APIs
just types     # regenerate crates/interface/protocol.ts
just chat      # talk to a fresh agent in the terminal
just inspect recent   # read what the agent did
just app       # the app, with hot reload
e2e/run.sh     # look at the app: runs e2e/tests/*.js in its page, pictures in e2e/.out/
```

## Looking at the app

There is no screen to capture from this shell, so the app photographs its own page:
`e2e/run.sh <test>` starts the debug build on an empty data folder, runs the script with `t`
(`app/src/lib/debug.ts`: `click`, `type`, `see`, `until`, `shot`, `check`) and writes
`e2e/.out/<name>.png`, which can be read as images. After changing the interface, run a test
and look at the pictures. A test that needs a model gets its key from Doppler:
`doppler run --project ternt --config dev -- e2e/run.sh conversation` (cheapest models).
