//! Passport: the sites the user is signed in to, for missions that need to be.
//!
//! Tokidoki holds no passwords. The user signs in to a site themselves, in the browser's own
//! window (`sign_in_account` opens it at the site's sign-in page), and the window keeps the
//! session: a mission just finds itself signed in. That survives codes, CAPTCHAs and
//! passkeys, and leaves nothing here worth stealing. When a session ends, the mission says
//! so and the user signs in again.
//!
//! What is kept is a list, `passport.json` beside the databases: each site, what the user
//! calls it, and the page to sign in on. One entry per site.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::{Said, data_dir, say};

const FILE: &str = "passport.json";

/// An account, as kept: nothing secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// The site it signs in to, as a host with no `www.`: `amazon.com`. Also what names it.
    pub site: String,
    /// What the user calls it: "Amazon".
    pub name: String,
    /// The page to sign in on.
    pub sign_in_url: String,
}

/// An account as the page shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Shown {
    #[serde(flatten)]
    account: Account,
    /// The browser holds cookies from the site.
    visited: bool,
}

/// What the page sends to save one.
#[derive(Debug, Deserialize)]
pub struct Draft {
    site: String,
    name: String,
    sign_in_url: String,
}

/// A site as a bare host: `https://www.Amazon.com/ap/signin` is `amazon.com`.
fn site_of(text: &str) -> Option<String> {
    let text = text.trim().to_lowercase();
    let with_scheme = match text.contains("://") {
        true => text,
        false => format!("https://{text}"),
    };
    let host = browser::host(&with_scheme)?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    // A name on the web: letters, digits, dots and hyphens, with a dot in it (or localhost).
    let named = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    (named && (host.contains('.') || host == "localhost")).then(|| host.to_string())
}

fn load(data: &Path) -> Vec<Account> {
    let read = std::fs::read_to_string(data.join(FILE)).ok();
    read.and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(data: &Path, accounts: &[Account]) -> Result<()> {
    std::fs::create_dir_all(data)?;
    let path = data.join(FILE);
    std::fs::write(&path, serde_json::to_string_pretty(accounts)?)
        .with_context(|| format!("writing {}", path.display()))
}

/// The accounts in `data`, with `draft` put in place of the one for its site, or added.
fn with(mut accounts: Vec<Account>, draft: &Draft) -> Result<(Vec<Account>, Account)> {
    let Some(site) = site_of(&draft.site) else {
        bail!(
            "“{}” isn't a site: give its address, like amazon.com",
            draft.site.trim()
        );
    };
    let sign_in_url = match draft.sign_in_url.trim() {
        "" => format!("https://{site}/"),
        url if url.starts_with("https://") || url.starts_with("http://") => url.to_string(),
        url => format!("https://{url}"),
    };
    // The page the user is sent to to sign in is the site's own, not a look-alike's.
    let there = browser::host(&sign_in_url).unwrap_or_default();
    if !browser::on_site(there, &site) {
        bail!("the sign-in page ({there}) isn't on {site}");
    }
    let account = Account {
        name: match draft.name.trim() {
            "" => site.clone(),
            name => name.to_string(),
        },
        sign_in_url,
        site,
    };
    match accounts.iter_mut().find(|a| a.site == account.site) {
        Some(kept) => *kept = account.clone(),
        None => accounts.push(account.clone()),
    }
    Ok((accounts, account))
}

/// Every account, as the page shows it.
#[tauri::command]
pub async fn passport(app: AppHandle) -> Said<Vec<Shown>> {
    let data = data_dir(&app).map_err(say)?;
    let shown = load(&data).into_iter().map(|account| Shown {
        visited: crate::browser::has_cookies(&app, &account.site),
        account,
    });
    Ok(shown.collect())
}

/// Saves an account, in place of the one for its site if there is one.
#[tauri::command]
pub async fn save_account(app: AppHandle, draft: Draft) -> Said<()> {
    let data = data_dir(&app).map_err(say)?;
    let (accounts, _) = with(load(&data), &draft).map_err(say)?;
    save(&data, &accounts).map_err(say)
}

/// Removes the account for `site`. The browser's session there is left: signing out is done
/// on the site.
#[tauri::command]
pub async fn delete_account(app: AppHandle, site: String) -> Said<()> {
    let data = data_dir(&app).map_err(say)?;
    let mut accounts = load(&data);
    accounts.retain(|account| account.site != site);
    save(&data, &accounts).map_err(say)
}

/// Opens the browser's window at the account's sign-in page, for the user to sign in.
#[tauri::command]
pub async fn sign_in_account(app: AppHandle, site: String) -> Said<()> {
    let data = data_dir(&app).map_err(say)?;
    let accounts = load(&data);
    let account = (accounts.iter().find(|account| account.site == site))
        .ok_or_else(|| format!("there is no account for {site}"))?;
    crate::browser::show_at(&app, &account.sign_in_url).map_err(say)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(site: &str, sign_in_url: &str) -> Draft {
        Draft {
            site: site.into(),
            name: String::new(),
            sign_in_url: sign_in_url.into(),
        }
    }

    #[test]
    fn a_site_is_named_by_its_bare_host() {
        assert_eq!(
            site_of("https://www.Amazon.com/ap/signin").as_deref(),
            Some("amazon.com")
        );
        assert_eq!(site_of(" reddit.com ").as_deref(), Some("reddit.com"));
        assert_eq!(site_of("localhost:1431").as_deref(), Some("localhost"));
        assert_eq!(site_of("amazon"), None);
        assert_eq!(site_of("not a site"), None);
        assert_eq!(site_of(""), None);
    }

    #[test]
    fn an_account_is_kept_once_per_site_and_signs_in_on_its_own_site() {
        let (accounts, account) = with(Vec::new(), &draft("www.amazon.com", "")).unwrap();
        assert_eq!(account.site, "amazon.com");
        assert_eq!(account.name, "amazon.com");
        assert_eq!(account.sign_in_url, "https://amazon.com/");
        let again = draft("amazon.com", "www.amazon.com/ap/signin");
        let (accounts, account) = with(accounts, &again).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(account.sign_in_url, "https://www.amazon.com/ap/signin");
        // A sign-in page off the site would send the user to a look-alike.
        let elsewhere = draft("amazon.com", "https://amazon.com.evil.example/signin");
        assert!(with(accounts, &elsewhere).is_err());
    }

    #[test]
    fn accounts_come_back_as_saved() {
        let dir = std::env::temp_dir().join(format!("tokidoki-passport-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&dir).is_empty());
        let (accounts, _) = with(Vec::new(), &draft("reddit.com", "")).unwrap();
        save(&dir, &accounts).unwrap();
        assert_eq!(load(&dir), accounts);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
