//! Remembers appearance, language, the last vault and where Touch ID is turned on.
//! Holds only local preferences and vault metadata, never secrets.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub language: crate::settings::language::Language,
    #[serde(default)]
    pub appearance: crate::settings::appearance::Appearance,
    pub last_vault: Option<PathBuf>,
    /// Display names for cloud profiles, available before password unlock.
    #[serde(default)]
    pub space_names: std::collections::BTreeMap<PathBuf, String>,
    /// Vaults (by `Vault::id`) whose owner turned Touch ID on.
    #[serde(default)]
    pub touch_id: Vec<String>,
    /// Lock after this many minutes without input; 0 means never.
    #[serde(default = "default_auto_lock")]
    pub auto_lock_minutes: u32,
    /// Opt-in network lookups disclose the selected website to that website.
    #[serde(default)]
    pub website_icons: bool,
}

fn default_auto_lock() -> u32 {
    crate::platform::autolock::DEFAULT_MINUTES
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: crate::settings::language::Language::default(),
            appearance: crate::settings::appearance::Appearance::default(),
            last_vault: None,
            space_names: Default::default(),
            touch_id: Vec::new(),
            auto_lock_minutes: default_auto_lock(),
            website_icons: false,
        }
    }
}

impl Config {
    /// `~/Library/Application Support/lockbox/config.json` on macOS,
    /// `~/.config/lockbox/config.json` on Linux.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("lockbox").join("config.json"))
    }

    /// A missing or unreadable config just means no vault is remembered.
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            create_private_dir(dir)?;
        }

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(path)?;
        file.write_all(&serde_json::to_vec_pretty(self).expect("config always serializes"))?;
        file.sync_all()
    }
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;
