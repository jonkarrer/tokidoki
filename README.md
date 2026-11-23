# tokidoki

A personal assistant that remembers: an agent with durable execution and automatic long-term
memory, as a Rust workspace.

```bash
nix develop --impure   # dev shell (from ~/devjon/configs): just, jq, tursodb, turso
just                   # list the recipes
just chat              # talk to a fresh tokidoki in the terminal
```

The flake doesn't provide Rust; bring your own toolchain (e.g. rustup). `just chat` and the
live tests read `TOKIDOKI_PROVIDER` (`anthropic`, `fireworks` or `baseten`) and that provider's
key (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`) from the environment or
`.env`; the live tests of each provider need its key.

## Layout

`crates/` is the system. Each crate's `src/lib.rs` opens with what it is for. `tokidoki` puts
them together as one library (input in, output out) and `interface` is its protocol, as plain
serde types, with `crates/interface/protocol.ts` generated from them (`just types`).

Shared dependency versions go in the root `Cargo.toml` under `[workspace.dependencies]`.
