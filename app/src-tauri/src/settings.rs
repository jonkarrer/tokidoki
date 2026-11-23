//! What the user chose that isn't a secret: whose models Tokidoki runs on, and which model
//! answers if not the provider's usual. Kept as `settings.json` beside the databases.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use tokidoki::Vendor;

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// A provider's ID (`Vendor::id`). None until the user has chosen.
    #[serde(default)]
    pub provider: Option<String>,
    /// The model that answers, in the provider's own naming. None for the provider's usual.
    #[serde(default)]
    pub model: Option<String>,
}

impl Settings {
    /// The settings in `data`: the defaults when there are none, or they can't be read.
    pub fn load(data: &Path) -> Self {
        let read = std::fs::read_to_string(data.join(FILE)).ok();
        read.and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data: &Path) -> Result<()> {
        std::fs::create_dir_all(data)?;
        let path = data.join(FILE);
        std::fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The provider to run on: `TOKIDOKI_PROVIDER` (development), else the user's choice,
    /// else Anthropic.
    pub fn provider(&self) -> Vendor {
        let named = std::env::var("TOKIDOKI_PROVIDER").ok();
        let chosen = named.as_deref().or(self.provider.as_deref());
        chosen
            .and_then(|id| id.parse().ok())
            .unwrap_or(Vendor::Anthropic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_come_back_as_saved_and_default_when_missing() {
        let dir = std::env::temp_dir().join(format!("tokidoki-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::load(&dir), Settings::default());
        let chosen = Settings {
            provider: Some("fireworks".into()),
            model: Some("some/model".into()),
        };
        chosen.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir), chosen);
        std::fs::write(dir.join(FILE), "not json").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
