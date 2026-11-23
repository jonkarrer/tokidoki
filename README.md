# tokidoki

A personal assistant that remembers: an agent with durable execution and automatic long-term
memory, as a Rust workspace, and a macOS app that packages it.

```bash
nix develop            # dev shell (nix/devshell.nix): just, jq, tursodb, turso, bun
just                   # list the recipes
just chat              # talk to a fresh tokidoki in the terminal
just app               # run the app with hot reload
```

The flake doesn't provide Rust; bring your own toolchain (e.g. rustup). `just chat` and the
live tests read `TOKIDOKI_PROVIDER` (`anthropic`, `fireworks` or `baseten`) and that provider's
key (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`) from the environment or
`.env`; the live tests of each provider need its key.

## Layout

- `crates/`: the system. Each crate's `src/lib.rs` opens with what it is for. `tokidoki` puts
  them together as one library (input in, output out) and `interface` is its protocol, as
  plain serde types, with `crates/interface/protocol.ts` generated from them (`just types`).
- `app/`: the macOS app, Tauri v2 + Svelte 5. `app/src-tauri` is a workspace member that runs
  the `tokidoki` library in-process; the page calls one command per library method and
  follows the same `interface` events any other client would.
- `e2e/`: looks at the app without a person. `e2e/run.sh` runs each script in `e2e/tests`
  inside the window's page and saves pictures of it to `e2e/.out/`.
- `scripts/`, `docs/RELEASING.md`: building, signing, notarizing and publishing the app.

Shared dependency versions go in the root `Cargo.toml` under `[workspace.dependencies]`.

## The app

It runs on the user's own key. They choose a provider (Anthropic, Fireworks or Baseten) and
paste its key, which is kept in the macOS Keychain; the choice itself is `settings.json`
beside the databases. Without a key the app still opens what it has, and only sending waits
for one. Its data lives in `~/Library/Application Support/com.jokar.tokidoki` (`…/dev` for
`just app`); point `just inspect --data <dir>` at it to read what the agent did.

Closing the window leaves it running, so idle conversations still dream; quitting stops it,
and the next launch resumes whatever was cut off.
