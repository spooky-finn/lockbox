//! Preference persistence. Contains no vault, editor or native handles.

use crate::settings::config::Config;
use std::path::{Path, PathBuf};

pub(crate) struct Settings {
    pub config: Config,
    pub path: Option<PathBuf>,
}

impl Settings {
    pub fn load() -> Self {
        let path = Config::default_path();
        let config = path.as_deref().map(Config::load).unwrap_or_default();
        Self { config, path }
    }

    pub fn save(&self) -> Result<(), String> {
        self.path.as_deref().map_or(Ok(()), |path| {
            self.config
                .save(path)
                .map_err(|error| format!("Could not save settings: {error}"))
        })
    }

    pub(crate) fn identity_path(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .map(|path| path.with_file_name("identity.key"))
    }

    /// Every tab keeps its own copy of the preferences. Reload the file first
    /// and apply only this change, so tabs never revert each other's settings.
    pub fn update(&mut self, change: impl FnOnce(&mut Config)) -> Result<(), String> {
        if let Some(path) = self.path.as_deref() {
            self.config = Config::load(path);
        }

        change(&mut self.config);
        self.save()
    }

    pub fn remember_profile(
        &mut self,
        path: PathBuf,
        space_name: Option<String>,
    ) -> Result<(), String> {
        self.update(|config| {
            config.recent_vaults.retain(|known| known != &path);
            config.recent_vaults.insert(0, path.clone());
            if let Some(name) = space_name {
                config.space_names.insert(path.clone(), name);
            } else {
                config.space_names.remove(&path);
            }
            config.last_vault = Some(path);
        })
    }

    pub(crate) fn forget_local_vault(&mut self, path: &Path, id: &str) -> Result<(), String> {
        self.update(|config| {
            config.recent_vaults.retain(|known| known != path);
            if config.last_vault.as_deref() == Some(path) {
                config.last_vault = None;
            }
            config.space_names.remove(path);
            config.vault_aliases.remove(path);
            config.vault_appearances.remove(id);
            config.touch_id.retain(|known| known != id);
        })
    }

    pub(crate) fn rename_local_vault(
        &mut self,
        source: &Path,
        destination: &Path,
    ) -> Result<(), String> {
        self.update(|config| {
            config.recent_vaults.retain(|path| path != destination);
            if let Some(path) = config.recent_vaults.iter_mut().find(|path| *path == source) {
                *path = destination.to_path_buf();
            } else {
                config.recent_vaults.insert(0, destination.to_path_buf());
            }
            if config
                .last_vault
                .as_deref()
                .is_none_or(|path| path == source)
            {
                config.last_vault = Some(destination.to_path_buf());
            }
            config.space_names.remove(source);
            config.space_names.remove(destination);
            config.vault_aliases.remove(destination);
            if let Some(alias) = config.vault_aliases.remove(source) {
                config
                    .vault_aliases
                    .insert(destination.to_path_buf(), alias);
            }
        })
    }

    pub(crate) fn cache_space_name(&mut self, path: &Path, name: &str) -> Result<(), String> {
        self.update(|config| {
            config
                .space_names
                .insert(path.to_path_buf(), name.to_owned());
        })
    }

    pub(crate) fn set_vault_alias(&mut self, path: &Path, name: &str) -> Result<(), String> {
        let name = name.trim();
        let mut previous = None;
        let saved = self.update(|config| {
            previous = if name.is_empty() {
                config.vault_aliases.remove(path)
            } else {
                config
                    .vault_aliases
                    .insert(path.to_path_buf(), name.to_owned())
            };
        });
        if saved.is_err() {
            if let Some(previous) = previous {
                self.config
                    .vault_aliases
                    .insert(path.to_path_buf(), previous);
            } else {
                self.config.vault_aliases.remove(path);
            }
        }
        saved
    }

    /// Include metadata saved before the recent-profile list was introduced.
    pub(crate) fn known_profiles(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for path in self
            .config
            .last_vault
            .iter()
            .chain(self.config.recent_vaults.iter())
            .chain(self.config.space_names.keys())
        {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        paths
    }

    pub fn set_touch_id(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        self.update(|config| {
            config.touch_id.retain(|known| known != id);

            if enabled {
                config.touch_id.push(id.to_owned());
            }
        })
    }
}

#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;
