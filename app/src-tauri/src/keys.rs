//! The user's own API keys: one per provider, and optional ones for Jev (TypeSafe) and Exa. They
//! live in the macOS Keychain, never in a file; a key in the environment wins over it
//! (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`, `TYPESAFE_API_KEY`, `EXA_API_KEY`:
//! development, where the repo's `.env` already has them).

use anyhow::{Context as _, Result};
use keyring::Entry;
use tokidoki::Vendor;

/// The Keychain service the keys are filed under, each under its provider's ID. Development
/// builds keep their own, so trying a key in `just app` never replaces the one the installed
/// app uses.
const SERVICE: &str = if cfg!(debug_assertions) {
    "com.jokar.tokidoki.dev"
} else {
    "com.jokar.tokidoki"
};

/// Where Jev's key is filed, and the variable that overrides it.
const JEV: &str = "typesafe";
const JEV_VAR: &str = "TYPESAFE_API_KEY";
/// Where Exa's key is filed, and the variable that overrides it.
const EXA: &str = "exa";
const EXA_VAR: &str = "EXA_API_KEY";

/// The user's key for `provider`, if they have given one.
pub fn get(provider: Vendor) -> Result<Option<String>> {
    read(provider.id(), provider.key_var())
}

/// Stores the key for `provider`, in place of the one there was.
pub fn set(provider: Vendor, key: &str) -> Result<()> {
    write(provider.id(), key)
}

/// The user's TypeSafe key, for Jev, if they have given one.
pub fn jev() -> Result<Option<String>> {
    read(JEV, JEV_VAR)
}

pub fn set_jev(key: &str) -> Result<()> {
    write(JEV, key)
}

fn read(account: &str, var: &str) -> Result<Option<String>> {
    let set = |key: &String| !key.is_empty();
    if let Some(key) = std::env::var(var).ok().filter(set) {
        return Ok(Some(key));
    }
    // Tests run on the environment's keys alone, whatever this Mac's Keychain holds.
    if std::env::var_os("TOKIDOKI_NO_KEYCHAIN").is_some() {
        return Ok(None);
    }
    match Entry::new(SERVICE, account)?.get_password() {
        Ok(key) if !key.is_empty() => Ok(Some(key)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error).context("reading the Keychain"),
    }
}

fn write(account: &str, key: &str) -> Result<()> {
    Entry::new(SERVICE, account)?
        .set_password(key)
        .context("writing to the Keychain")
}

/// The user's Exa key, for research workers, if they have given one.
pub fn exa() -> Result<Option<String>> {
    read(EXA, EXA_VAR)
}

pub fn set_exa(key: &str) -> Result<()> {
    write(EXA, key)
}
