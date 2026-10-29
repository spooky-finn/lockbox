//! Remembers which vault was opened last and where Touch ID is turned on.
//! Holds only paths and vault identifiers, never secrets.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub last_vault: Option<PathBuf>,
    /// Vaults (by `Vault::id`) whose owner turned Touch ID on.
    #[serde(default)]
    pub touch_id: Vec<String>,
    /// Lock after this many minutes without input; 0 means never.
    #[serde(default = "default_auto_lock")]
    pub auto_lock_minutes: u32,
}

fn default_auto_lock() -> u32 {
    crate::autolock::DEFAULT_MINUTES
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last_vault: None,
            touch_id: Vec::new(),
            auto_lock_minutes: default_auto_lock(),
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
mod tests {
    use super::*;

    #[test]
    fn remembered_path_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lockbox").join("config.json");
        let config = Config {
            last_vault: Some("/tmp/vault.lbx".into()),
            touch_id: vec!["00ff".into()],
            auto_lock_minutes: 15,
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path), config);
    }

    #[test]
    fn config_from_before_touch_id_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, br#"{"last_vault":"/tmp/vault.lbx"}"#).unwrap();
        let config = Config::load(&path);
        assert_eq!(
            config.last_vault.as_deref(),
            Some(Path::new("/tmp/vault.lbx"))
        );
        assert!(config.touch_id.is_empty());
        assert_eq!(config.auto_lock_minutes, crate::autolock::DEFAULT_MINUTES);
    }

    #[test]
    fn missing_or_broken_config_remembers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(Config::load(&path), Config::default());
        fs::write(&path, b"{not json").unwrap();
        assert_eq!(Config::load(&path), Config::default());
    }

    #[cfg(unix)]
    #[test]
    fn config_file_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        Config::default().save(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
