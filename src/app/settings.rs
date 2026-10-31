//! Preference persistence. Contains no vault, editor or native handles.
use crate::infra::config::Config;
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
                .map_err(|error| format!("Не удалось сохранить настройки: {error}"))
        })
    }

    pub fn remember(&mut self, path: Option<PathBuf>) -> Result<(), String> {
        self.config.last_vault = path;
        self.save()
    }

    pub fn set_touch_id(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        self.config.touch_id.retain(|known| known != id);
        if enabled {
            self.config.touch_id.push(id.to_owned());
        }
        self.save()
    }
}

pub(crate) fn minutes_label(minutes: u32) -> String {
    match minutes {
        0 => "никогда".to_owned(),
        minutes => format!("{minutes} мин"),
    }
}
