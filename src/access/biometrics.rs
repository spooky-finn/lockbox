//! Biometric preference and worker lifecycle, scoped to one unlocked vault.

use crate::{
    desktop::time::unix_now,
    platform::{
        jobs::Worker,
        touchid::{self, Biometrics, TouchIdError},
    },
    settings::state::Settings,
    vault::Vault,
};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct TouchId {
    pub available: bool,
    pub enabled: bool,
    pub pending: Option<Worker<Result<(), String>>>,
}

impl TouchId {
    pub fn open(
        vault: &Vault,
        settings: &Settings,
        biometrics: &dyn Biometrics,
        with_password: bool,
    ) -> (Self, Option<String>) {
        let id = vault.id();
        let enabled = settings.config.touch_id.contains(&id);
        let error = if with_password && enabled {
            match touchid::refresh(biometrics, &id, &vault.export_key(), unix_now()) {
                Ok(()) | Err(TouchIdError::Unavailable) => None,
                Err(error) => Some(error.to_string()),
            }
        } else {
            None
        };
        (
            Self {
                available: touchid::available(biometrics),
                enabled,
                pending: None,
            },
            error,
        )
    }

    pub fn set(
        &mut self,
        vault: &Vault,
        settings: &mut Settings,
        biometrics: Arc<dyn Biometrics>,
        wanted: bool,
    ) -> Option<String> {
        if self.pending.is_some() {
            return None;
        }

        let id = vault.id();

        if wanted {
            let key = vault.export_key();
            self.pending = Some(Worker::spawn(move |_| {
                touchid::enable(&*biometrics, &id, &key, unix_now())
                    .map_err(|error| format!("Could not enable Touch ID: {error}"))
            }));
            None
        } else {
            touchid::disable(&*biometrics, &id);
            self.enabled = false;
            settings.set_touch_id(&id, false).err()
        }
    }

    pub fn poll(&mut self, vault: &Vault, settings: &mut Settings) -> Option<Result<(), String>> {
        let result = self
            .pending
            .as_mut()?
            .take()?
            .unwrap_or_else(|_| Err("Enabling Touch ID was interrupted".into()));

        self.pending = None;
        Some(result.and_then(|()| {
            self.enabled = true;
            settings.set_touch_id(&vault.id(), true)
        }))
    }
}

#[cfg(test)]
#[path = "tests/biometrics.rs"]
mod tests;
