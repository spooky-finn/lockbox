//! Unlock/create form and its background worker; no UI or system-service ownership.
use super::time::unix_now;
use crate::infra::profile::{BootstrapProfile, ProviderConfiguration};
use crate::{
    infra::config::Config,
    infra::touchid::{self, Biometrics},
    infra::vault::{KdfParams, Vault},
};
use secrecy::SecretString;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const MIN_PASSWORD_CHARS: usize = 12;
const SECRET_CAPACITY: usize = 256;
fn secret_buffer() -> Zeroizing<String> {
    Zeroizing::new(String::with_capacity(SECRET_CAPACITY))
}

/// A vault unlocked on a worker thread.
pub(crate) struct Opened {
    pub(crate) vault: Vault,
    pub(crate) with_password: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Open,
    Create,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageKind {
    Local,
    Cloud,
}

pub(crate) use super::cloud_setup::CloudSetup;

pub(crate) struct Locked {
    pub(crate) path: Option<PathBuf>,
    pub(crate) mode: Mode,
    pub(crate) storage_kind: Option<StorageKind>,
    pub(crate) cloud: CloudSetup,
    pub(crate) cloud_password_step: bool,
    pub(crate) password: Zeroizing<String>,
    pub(crate) repeat: Zeroizing<String>,
    pub(crate) recovery_password: Zeroizing<String>,
    pub(crate) recovery_repeat: Zeroizing<String>,
    pub(crate) error: Option<String>,
    /// Argon2id and Touch ID run on a worker thread so the window keeps repainting.
    pub(crate) pending: Option<Receiver<Result<Opened, String>>>,
    /// Vault identifier when Touch ID can open this file.
    pub(crate) touch_id: Option<String>,
    /// Ask for the finger right away: only when the app starts.
    pub(crate) auto_touch_id: bool,
    pub(crate) waiting_touch_id: bool,
    /// Why the vault was locked without the user asking.
    pub(crate) notice: Option<String>,
}

impl Locked {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        let error = path
            .as_ref()
            .filter(|path| !path.exists())
            .map(|path| format!("Файл не найден: {}", path.display()));
        Self {
            path,
            mode: Mode::Open,
            storage_kind: None,
            cloud: CloudSetup::default(),
            cloud_password_step: false,
            password: secret_buffer(),
            repeat: secret_buffer(),
            recovery_password: secret_buffer(),
            recovery_repeat: secret_buffer(),
            error,
            pending: None,
            touch_id: None,
            auto_touch_id: false,
            waiting_touch_id: false,
            notice: None,
        }
    }

    /// Offers Touch ID when it was turned on for this file and the sensor is available.
    pub(crate) fn offer_touch_id(
        mut self,
        config: &Config,
        biometrics: &dyn Biometrics,
        auto: bool,
    ) -> Self {
        self.touch_id = self
            .path
            .as_ref()
            .filter(|path| path.exists())
            .and_then(|path| Vault::peek_id(path).ok())
            .filter(|id| config.touch_id.contains(id))
            .filter(|_| touchid::available(biometrics));
        self.auto_touch_id = auto && self.touch_id.is_some();
        self
    }

    pub(crate) fn start_touch_id(&mut self, biometrics: Arc<dyn Biometrics>) {
        let (Some(path), Some(id)) = (self.path.clone(), self.touch_id.clone()) else {
            return;
        };
        self.error = None;
        self.waiting_touch_id = true;
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = touchid::unlock(&*biometrics, &id, unix_now())
                .map_err(|error| error.to_string())
                .and_then(|key| Vault::open_with_key(&path, &key).map_err(|e| e.to_string()))
                .map(|vault| Opened {
                    vault,
                    with_password: false,
                });
            let _ = sender.send(result);
        });
        self.pending = Some(receiver);
    }

    pub(crate) fn continue_cloud(&mut self) {
        if self.mode != Mode::Create || self.storage_kind != Some(StorageKind::Cloud) {
            return;
        }
        match self.cloud.validate() {
            Ok(()) => {
                self.error = None;
                self.cloud_password_step = true;
            }
            Err(error) => self.error = Some(error.into()),
        }
    }

    pub(crate) fn edit_cloud_connection(&mut self) {
        self.path = None;
        self.cloud_password_step = false;
        self.password.zeroize();
        self.repeat.zeroize();
        self.recovery_password.zeroize();
        self.recovery_repeat.zeroize();
        self.error = None;
    }

    pub(crate) fn validate_creation(&mut self) -> bool {
        if self.mode != Mode::Create
            || self.storage_kind != Some(StorageKind::Cloud)
            || !self.cloud_password_step
        {
            return false;
        }
        let error = self.cloud.validate().err().map(str::to_owned).or_else(|| {
            if self.password.chars().count() < MIN_PASSWORD_CHARS {
                Some(format!(
                    "Мастер-пароль: не менее {MIN_PASSWORD_CHARS} символов"
                ))
            } else if *self.password != *self.repeat {
                Some("Пароли не совпадают".into())
            } else if self.recovery_password.chars().count() < MIN_PASSWORD_CHARS {
                Some(format!(
                    "Пароль восстановления: не менее {MIN_PASSWORD_CHARS} символов"
                ))
            } else if *self.recovery_password != *self.recovery_repeat {
                Some("Пароли восстановления не совпадают".into())
            } else if *self.password == *self.recovery_password {
                Some("Пароль восстановления должен отличаться от мастер-пароля".into())
            } else {
                None
            }
        });
        self.error = error;
        self.error.is_none()
    }

    pub(crate) fn submit(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let cloud = self.mode == Mode::Create && self.storage_kind == Some(StorageKind::Cloud);
        if self.mode == Mode::Create && self.storage_kind.is_none() {
            self.error = Some("Выберите тип хранилища".into());
            return;
        }
        if cloud && !self.validate_creation() {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.password.is_empty() {
            self.error = Some("Введите мастер-пароль".into());
            return;
        }
        if self.mode == Mode::Create && *self.password != *self.repeat {
            self.error = Some("Пароли не совпадают".into());
            return;
        }
        // Move the Rust-side password buffer to the worker, which wipes it on drop.
        let password = SecretString::from(std::mem::replace(
            &mut *self.password,
            String::with_capacity(SECRET_CAPACITY),
        ));
        self.repeat.zeroize();
        self.error = None;
        self.waiting_touch_id = false;
        let mode = self.mode;
        let cloud_creation = cloud.then(|| {
            let recovery = SecretString::from(std::mem::take(&mut *self.recovery_password));
            self.recovery_repeat.zeroize();
            (
                self.cloud.username.trim().to_owned(),
                self.cloud.space_name.trim().to_owned(),
                ProviderConfiguration {
                    base_url: self.cloud.base_url.clone(),
                    bucket: self.cloud.bucket.clone(),
                    access_key: self.cloud.access_key.to_string(),
                    secret_key: self.cloud.secret_key.to_string(),
                },
                recovery,
            )
        });
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = match mode {
                Mode::Open => Vault::open(&path, &password),
                Mode::Create => {
                    if let Some((username, space_name, provider, recovery)) = cloud_creation {
                        BootstrapProfile::generate(
                            username,
                            space_name,
                            provider,
                            &recovery,
                            KdfParams::default(),
                        )
                        .and_then(|profile| {
                            Vault::create_profile(&path, &password, KdfParams::default(), profile)
                        })
                    } else {
                        Vault::create(&path, &password, KdfParams::default())
                    }
                }
            }
            .map(|vault| Opened {
                vault,
                with_password: true,
            })
            .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.pending = Some(receiver);
    }
}

impl Locked {
    pub(crate) fn poll(&mut self, biometrics: Arc<dyn Biometrics>) -> Option<Opened> {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(Ok(opened)) => return Some(opened),
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.pending = None;
                    self.waiting_touch_id = false;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.error = Some("Разблокировка прервалась".into());
                    self.pending = None;
                    self.waiting_touch_id = false;
                }
            }
        }
        if self.auto_touch_id && self.pending.is_none() {
            self.auto_touch_id = false;
            self.start_touch_id(biometrics);
        }
        None
    }
}
/// Picked files get the `.lbx` extension, so `.gitignore` always covers them.
pub(crate) fn with_vault_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("lbx");
    }
    path
}

#[cfg(test)]
#[path = "tests/auth.rs"]
mod tests;
