//! Remembers which vault was opened last. Holds only the path, never secrets.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub last_vault: Option<PathBuf>,
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
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path), config);
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
