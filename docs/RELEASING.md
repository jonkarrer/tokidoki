# Releasing Tokidoki

The app (`app/`) is released the way ternt is: built on a Mac that has an Apple **Developer
ID**, so it is signed and notarized and opens without warnings on any Mac. GitHub Releases
hosts the downloads and the update manifest; installed copies update themselves (see
`app/src-tauri/src/updater.rs`).

```sh
just release 0.2.0   # bump the app's version, commit, tag v0.2.0 (any machine)
just upload          # on the signing Mac: build, sign, notarize, push, publish
```

## Where releases live

On the source repo itself, `jonkarrer/tokidoki`, which is public. It has to be: the updater
sends no credentials, and a private repo's release files can't be downloaded without them.
The repo is named once, in the updater endpoint in `app/src-tauri/tauri.conf.json`:

```json
"endpoints": ["https://github.com/jonkarrer/tokidoki/releases/latest/download/latest.json"]
```

`scripts/publish.sh` reads the repo from that line. The endpoint is built into every copy of
the app, so changing it later strands the copies already installed (the old address would
have to keep serving a `latest.json`). Every update is signed with the updater key, so a
copy only ever installs what this project published, whoever can read the repo.

## What `just upload` needs

| Needed | Used for | Where it comes from |
| --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` (+`_PASSWORD`) | signs the update archive | env, else `~/.tauri/tokidoki.key` (+`.password`) |
| `GITHUB_ACCESS_TOKEN` or `GITHUB_TOKEN` | creates the release, uploads files | env, else keychain item `tokidoki-github` |
| `APPLE_CLI_UPLOAD_KEY_ID` + `_ISSUER_ID` | App Store Connect API key for notarization (the one ternt uses; the `.p8` is in `~/devjon/ids`) | env: required |
| a *Developer ID Application* certificate | signs the app | the keychain (ternt's signing keychain is unlocked if present) |

The Apple values and the GitHub token are in Doppler, project `general`, config
`app_tokidoki`:

```sh
doppler run --project general --config app_tokidoki -- just upload
```

Don't run it under another app's config: one that holds that app's
`TAURI_SIGNING_PRIVATE_KEY` signs the update with the wrong key (0.1.1's was signed with
ternt's this way; `publish.sh` now refuses to go on when the key isn't this app's).

**The updater key** (`~/.tauri/tokidoki.key`, its password beside it) signs every update;
installed copies only accept updates signed with it, and its public half is in
`tauri.conf.json`. Back it up somewhere that outlives this Mac (the same Doppler config, as
`TAURI_SIGNING_PRIVATE_KEY` and `_PASSWORD`).
**Never commit it**: losing it means installed copies can't update.

**The GitHub token** is a fine-grained token with *Contents: Read and write* on this
repo only. Store it without it touching shell history:

```sh
security add-generic-password -s tokidoki-github -a jonkarrer -w
```

Release binaries must link only system libraries, so `publish.sh` compiles and links C with
Apple's toolchain whatever shell it runs from, and checks the result. Both Rust targets are
needed:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

## Each release

1. On `main`, with everything committed: `just upload`. HEAD isn't tagged yet, so it asks
   whether this is a patch, minor or major release (or `just upload minor`), bumps the
   version in `tauri.conf.json`, `package.json` and the app's `Cargo.toml`, commits and tags.
2. The same run checks the setup, builds a universal app, signs and notarizes it (a few
   minutes, waiting on Apple), verifies the signature and Gatekeeper assessment, signs the
   update archive, writes `latest.json`, pushes `main` and the tag to the source repo, and
   creates the release (a draft until every file is uploaded) with:
   - `Tokidoki_<version>_universal.dmg`: for new installs. This is the link to hand out.
   - `Tokidoki_<version>_universal.app.tar.gz`: the update installed copies download
   - `latest.json`: what installed copies check
3. Within a few seconds of launch, every 15 minutes after, and when their window comes
   forward after a couple of minutes away, installed copies download the update in the
   background and install it when they quit; meanwhile a card in the sidebar says it's ready
   (*Restart now* installs it right away). *Check for Updates…* in the Tokidoki menu, the
   button under *Model and key*, and the version in the sidebar all check on the spot.

If something fails halfway, nothing is published until the last step. The bump commit and tag
stay local and the built, notarized files stay in `target/universal-apple-darwin/release/bundle/publish`:
fix the cause (deleting any leftover draft release) and run `just upload-resume`.

`just upload-dry-run` builds and verifies the tag at HEAD with no Apple signing and no upload.
