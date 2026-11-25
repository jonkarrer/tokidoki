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
- `CASE_STUDY.md`: how it came to be built this way.

Shared dependency versions go in the root `Cargo.toml` under `[workspace.dependencies]`.

## The app

It runs on the user's own key. They choose a provider (Anthropic, Fireworks or Baseten) and
paste its key, which is kept in the macOS Keychain; the choice itself is `settings.json`
beside the databases. Two more keys are optional: Jev's (TypeSafe), for quicker and cheaper
decisions about where a message belongs and what to remember, and Exa's, for research on
the web. Voice (talking and being read to) runs on the Mac, with models it downloads on
first use. Without a key the app still opens what it has, and only sending waits
for one. Its data lives in `~/Library/Application Support/com.jokar.tokidoki` (`…/dev` for
`just app`); point `just inspect --data <dir>` at it to read what the agent did.

Closing the window leaves it running, so idle conversations still dream; quitting stops it,
and the next launch resumes whatever was cut off.

## What to know before using it

- **Your words go to the provider you choose.** Messages, and the memories and notes brought
  into a conversation, are sent to Anthropic, Fireworks or Baseten, whichever you picked.
  With a Jev key, short excerpts go to TypeSafe for the quick decisions; with an Exa key,
  research queries go to Exa. Each handles them under its own terms.
- **Usage is billed to your keys.** Tokidoki works in the background too (remembering,
  tidying memory, research), and that also calls the models.
- **Everything else stays on the Mac.** Conversations, memory and notes are files in the
  data folder above; keys are in the Keychain; voice is heard and spoken locally. The app
  contacts GitHub to look for updates and, the first time voice is used, to download its
  models.
- **It can be wrong.** What it says, remembers or finds on the web may be mistaken or out of
  date. It is not a source of medical, legal or financial advice.
- **No warranty.** It is provided as is; see the licence.

## License

Tokidoki's own code is MIT: see `LICENSE`. The app is built from other open-source work,
listed with its licences in `THIRD_PARTY_LICENSES.md` (written by `just licenses`, and
bundled in the app beside `LICENSE`). One of them is not permissive: speaking uses
espeak-ng, which is GPL-3.0-or-later and is linked into the released app through sherpa-onnx.
