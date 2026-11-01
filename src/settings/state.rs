//! Preference persistence. Contains no vault, editor or native handles.

use crate::settings::config::Config;
use std::path::PathBuf;

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

    pub fn remember(&mut self, path: Option<PathBuf>) -> Result<(), String> {
        self.update(|config| {
            if path.is_none()
                && let Some(previous) = &config.last_vault
            {
                config.space_names.remove(previous);
            }
            config.last_vault = path;
        })
    }

    pub fn remember_profile(
        &mut self,
        path: PathBuf,
        space_name: Option<String>,
    ) -> Result<(), String> {
        self.update(|config| {
            if let Some(name) = space_name {
                config.space_names.insert(path.clone(), name);
            } else {
                config.space_names.remove(&path);
            }
            config.last_vault = Some(path);
        })
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
