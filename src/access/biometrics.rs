//! Biometric preference and worker lifecycle, scoped to one unlocked vault.

use crate::{
    desktop::time::unix_now,
    platform::{
        jobs::Worker,
        touchid::{self, Biometrics, TouchIdError},
    },
    settings::{config::Config, state::Settings},
    vault::Vault,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Default)]
pub(crate) struct TouchId {
    pub available: bool,
    pub enabled: bool,
    pub pending: Option<Enrolment>,
}

/// Who answers for a key stored by an enrolment: whichever side of the
/// hand-off comes last cleans up, so a lock never leaves it untracked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Stored,
    Delivered,
    Abandoned,
}

/// An enable in flight. Dropping it (lock, profile switch) abandons it.
pub(crate) struct Enrolment {
    worker: Worker<Result<(), String>>,
    phase: Arc<Mutex<Phase>>,
    owner: KeyOwner,
}

/// What an abandoned enrolment needs to remove its key, unless the
/// saved preferences already list the vault.
#[derive(Clone)]
struct KeyOwner {
    biometrics: Arc<dyn Biometrics>,
    id: String,
    config: Option<PathBuf>,
    listed: bool,
}

impl KeyOwner {
    fn discard(&self) {
        if !self.referenced() {
            touchid::disable(&*self.biometrics, &self.id);
        }
    }

    /// Anything other than a readable config without this vault keeps the key.
    fn referenced(&self) -> bool {
        let Some(path) = &self.config else {
            return self.listed;
        };
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<Config>(&bytes)
                .map_or(true, |config| config.touch_id.contains(&self.id)),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    }
}

fn phase(phase: &Mutex<Phase>) -> std::sync::MutexGuard<'_, Phase> {
    phase.lock().unwrap_or_else(|p| p.into_inner())
}

/// Lets the worker store the key only while the enrolment is still wanted.
struct Gate {
    owner: KeyOwner,
    phase: Arc<Mutex<Phase>>,
}

impl Biometrics for Gate {
    fn domain_state(&self) -> Option<Vec<u8>> {
        self.owner.biometrics.domain_state()
    }
    fn authenticate(&self, reason: &str) -> Result<Vec<u8>, String> {
        self.owner.biometrics.authenticate(reason)
    }
    fn load(&self, account: &str, reason: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        self.owner.biometrics.load(account, reason)
    }
    fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
        if *phase(&self.phase) == Phase::Abandoned {
            return Err("the vault was locked".into());
        }
        let stored = self.owner.biometrics.store(account, data);
        let abandoned = {
            let mut phase = phase(&self.phase);
            if *phase != Phase::Abandoned && stored.is_ok() {
                *phase = Phase::Stored;
            }
            *phase == Phase::Abandoned
        };
        if abandoned {
            if stored.is_ok() {
                self.owner.discard();
            }
            return Err("the vault was locked".into());
        }
        stored
    }
    fn delete(&self, account: &str) {
        self.owner.biometrics.delete(account)
    }
}

impl Drop for Enrolment {
    fn drop(&mut self) {
        let mut phase = phase(&self.phase);
        let stored = *phase == Phase::Stored;
        if *phase != Phase::Delivered {
            *phase = Phase::Abandoned;
        }
        drop(phase);
        // A key stored but never recorded in the preferences would linger.
        if stored {
            self.owner.discard();
        }
    }
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
        if self.pending.is_some() && wanted {
            return None;
        }

        let id = vault.id();

        if wanted {
            let key = vault.export_key();
            let phase = Arc::new(Mutex::new(Phase::Running));
            let owner = KeyOwner {
                biometrics,
                listed: settings.config.touch_id.contains(&id),
                config: settings.path.clone(),
                id: id.clone(),
            };
            let gate = Gate {
                owner: owner.clone(),
                phase: phase.clone(),
            };
            let worker = Worker::spawn(move |_| {
                touchid::enable(&gate, &id, &key, unix_now())
                    .map_err(|error| format!("Could not enable Touch ID: {error}"))
            });
            self.pending = Some(Enrolment {
                worker,
                phase,
                owner,
            });
            None
        } else {
            // Abandon an enable still in flight so it cannot finish after this.
            self.pending = None;
            touchid::disable(&*biometrics, &id);
            self.enabled = false;
            settings.set_touch_id(&id, false).err()
        }
    }

    pub fn poll(&mut self, vault: &Vault, settings: &mut Settings) -> Option<Result<(), String>> {
        let enrolment = self.pending.as_mut()?;
        let result = enrolment
            .worker
            .take()?
            .unwrap_or_else(|_| Err("Enabling Touch ID was interrupted".into()));

        if result.is_ok() {
            *phase(&enrolment.phase) = Phase::Delivered;
        }
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
