//! Unlock/create form and its background worker; no UI or system-service ownership.

use crate::{
    access::mnemonic::RecoveryPhrase,
    desktop::time::unix_now,
    platform::{
        jobs::Worker,
        touchid::{self, Biometrics},
    },
    settings::config::Config,
    vault::{KdfParams, Vault, identity::Identity, profile::ProviderConfiguration},
};
use secrecy::{ExposeSecret, SecretString};
use std::{path::PathBuf, sync::Arc};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const MIN_PASSWORD_CHARS: usize = 12;
const SECRET_CAPACITY: usize = 256;
fn secret_buffer() -> Zeroizing<String> {
    Zeroizing::new(String::with_capacity(SECRET_CAPACITY))
}

fn identity_file_present(path: &std::path::Path) -> bool {
    !matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

/// A vault unlocked on a worker thread.
pub(crate) struct Opened {
    pub(crate) vault: Vault,
    pub(crate) with_password: bool,
    pub(crate) recovery_notice: Option<String>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreationStep {
    EncryptionKey,
    Identity,
    Sync,
    Password,
}

pub(crate) use crate::access::cloud_setup::CloudSetup;

pub(crate) struct Locked {
    pub(crate) path: Option<PathBuf>,
    pub(crate) space_name: Option<String>,
    pub(crate) mode: Mode,
    pub(crate) storage_kind: Option<StorageKind>,
    pub(crate) cloud: CloudSetup,
    pub(crate) creation_step: CreationStep,
    pub(crate) joining: bool,
    pub(crate) password: Zeroizing<String>,
    pub(crate) repeat: Zeroizing<String>,
    pub(crate) recovery_words: Zeroizing<String>,
    pub(crate) words_saved: bool,
    pub(crate) show_recovery_words: bool,
    pub(crate) importing_words: bool,
    pub(crate) recovering: bool,
    pub(crate) identity_path: Option<PathBuf>,
    pub(crate) identity_source: Option<PathBuf>,
    pub(crate) identity: Option<Identity>,
    pending_identity: Option<Worker<Result<Identity, String>>>,
    pub(crate) error: Option<String>,
    /// Argon2id and Touch ID run on a worker thread so the window keeps repainting.
    pub(crate) pending: Option<Worker<Result<Opened, String>>>,
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
            .map(|path| format!("File not found: {}", path.display()));
        Self {
            path,
            space_name: None,
            mode: Mode::Open,
            storage_kind: None,
            cloud: CloudSetup::default(),
            creation_step: CreationStep::EncryptionKey,
            joining: false,
            password: secret_buffer(),
            repeat: secret_buffer(),
            recovery_words: secret_buffer(),
            words_saved: false,
            show_recovery_words: false,
            importing_words: false,
            recovering: false,
            identity_path: None,
            identity_source: None,
            identity: None,
            pending_identity: None,
            error,
            pending: None,
            touch_id: None,
            auto_touch_id: false,
            waiting_touch_id: false,
            notice: None,
        }
    }

    pub(crate) fn begin_create() -> Self {
        let mut form = Self::new(None);
        form.mode = Mode::Create;
        form.generate_recovery_words();
        form
    }

    pub(crate) fn begin_join() -> Self {
        let mut form = Self::begin_create();
        form.storage_kind = Some(StorageKind::Cloud);
        form.joining = true;
        // A local staging profile supplies independent keys until admission.
        form.cloud.space_name = "Shared space".into();
        form
    }

    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some() || self.pending_identity.is_some()
    }

    /// A missing identity may be set up once; unreadable files still require unlock.
    pub(crate) fn with_identity_path(
        mut self,
        path: Option<PathBuf>,
        source: Option<PathBuf>,
    ) -> Self {
        self.identity_path = path;
        if self.mode == Mode::Create && self.identity_path.is_some() {
            let saved = self
                .identity_path
                .as_deref()
                .is_some_and(identity_file_present);
            self.identity_source = (!saved).then_some(source).flatten();
            if saved || self.identity_source.is_some() {
                self.recovery_words.zeroize();
                self.creation_step = CreationStep::Identity;
            }
        }
        self
    }

    pub(crate) fn unlocking_identity(&self) -> bool {
        self.creation_step == CreationStep::Identity
            && (self.identity_source.is_some()
                || self
                    .identity_path
                    .as_deref()
                    .is_some_and(identity_file_present)
                    && (self.recovery_words.is_empty() || self.recovering))
    }

    /// Switching between create and join must not replace the selected identity.
    pub(crate) fn switch_creation(&mut self, joining: bool) {
        if self.busy() {
            return;
        }
        self.edit_sync_setup();
        self.joining = joining;
        self.recovering = false;
        self.storage_kind = joining.then_some(StorageKind::Cloud);
        self.cloud = CloudSetup::default();
        if joining {
            self.cloud.space_name = "Shared space".into();
        }
        if self.identity.is_none() {
            self.creation_step = if self.identity_source.is_some()
                || self
                    .identity_path
                    .as_deref()
                    .is_some_and(identity_file_present)
            {
                CreationStep::Identity
            } else {
                CreationStep::EncryptionKey
            };
        }
    }

    /// Offers Touch ID when it was turned on for this file and the sensor is available.
    pub(crate) fn offer_touch_id(
        mut self,
        config: &Config,
        biometrics: &dyn Biometrics,
        auto: bool,
    ) -> Self {
        if self.space_name.is_none() {
            self.space_name = self
                .path
                .as_ref()
                .and_then(|path| config.space_names.get(path))
                .cloned();
        }
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
        self.pending = Some(Worker::spawn(move |_| {
            touchid::unlock(&*biometrics, &id, unix_now())
                .map_err(|error| error.to_string())
                .and_then(|key| Vault::open_with_key(&path, &key).map_err(|e| e.to_string()))
                .map(|vault| Opened {
                    vault,
                    with_password: false,
                    recovery_notice: None,
                })
        }));
    }

    pub(crate) fn continue_cloud(&mut self) {
        if self.mode != Mode::Create
            || self.creation_step != CreationStep::Sync
            || self.storage_kind != Some(StorageKind::Cloud)
            || !self.validate_recovery_key()
        {
            return;
        }

        match self.cloud.validate() {
            Ok(()) => {
                self.error = None;
                self.creation_step = CreationStep::Password;
            }
            Err(error) => self.error = Some(error.into()),
        }
    }

    pub(crate) fn edit_sync_setup(&mut self) {
        self.path = None;
        self.creation_step = CreationStep::Sync;
        self.password.zeroize();
        self.repeat.zeroize();
        self.show_recovery_words = false;
        self.error = None;
    }

    pub(crate) fn edit_recovery_key(&mut self) {
        if self.identity.is_some() {
            return;
        }
        self.edit_sync_setup();
        self.creation_step = CreationStep::EncryptionKey;
    }

    fn validate_recovery_key(&mut self) -> bool {
        if self.identity.is_some() {
            self.error = None;
            return true;
        }
        self.error = if let Err(error) = RecoveryPhrase::parse(&self.recovery_words) {
            Some(error.to_string())
        } else if !self.words_saved {
            Some("Save your recovery words before continuing".into())
        } else {
            None
        };
        self.error.is_none()
    }

    pub(crate) fn continue_key_setup(&mut self) {
        if self.mode == Mode::Create
            && self.creation_step == CreationStep::EncryptionKey
            && self.validate_recovery_key()
        {
            self.show_recovery_words = false;
            self.creation_step = if self.identity_path.is_some() {
                CreationStep::Identity
            } else {
                CreationStep::Sync
            };
        }
    }

    /// Keep the confirmed key when the native save dialog returns.
    pub(crate) fn choose_local_path(&mut self, path: PathBuf) {
        if self.mode == Mode::Create
            && self.creation_step == CreationStep::Sync
            && !self.joining
            && self.validate_recovery_key()
        {
            self.path = Some(with_vault_extension(path));
            self.storage_kind = Some(StorageKind::Local);
            self.creation_step = CreationStep::Password;
        }
    }

    pub(crate) fn generate_recovery_words(&mut self) -> bool {
        if self.identity.is_some()
            || self.identity_source.is_some()
            || self
                .identity_path
                .as_deref()
                .is_some_and(identity_file_present)
        {
            return false;
        }
        match RecoveryPhrase::generate() {
            Ok(phrase) => {
                self.recovery_words = Zeroizing::new(phrase.words().expose_secret().to_owned());
                self.words_saved = false;
                self.show_recovery_words = false;
                self.importing_words = false;
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    pub(crate) fn toggle_recovery(&mut self) {
        self.recovering = !self.recovering;
        self.auto_touch_id = false;
        self.password.zeroize();
        self.repeat.zeroize();
        self.recovery_words.zeroize();
        self.error = None;
    }

    pub(crate) fn validate_creation(&mut self) -> bool {
        if self.mode != Mode::Create
            || self.storage_kind.is_none()
            || self.creation_step != CreationStep::Password
            || (self.identity_path.is_some() && self.identity.is_none())
        {
            return false;
        }

        let error = self
            .storage_kind
            .filter(|kind| *kind == StorageKind::Cloud)
            .and_then(|_| self.cloud.validate().err().map(str::to_owned))
            .or_else(|| self.new_password_error());
        self.error = error;
        self.error.is_none() && self.validate_recovery_key()
    }

    fn new_password_error(&self) -> Option<String> {
        if self.password.chars().count() < MIN_PASSWORD_CHARS {
            Some(format!(
                "Master password: at least {MIN_PASSWORD_CHARS} characters"
            ))
        } else if *self.password != *self.repeat {
            Some("Passwords do not match".into())
        } else {
            None
        }
    }

    pub(crate) fn submit(&mut self) {
        if self.busy() {
            return;
        }

        if self.mode == Mode::Create && self.creation_step == CreationStep::Identity {
            self.submit_identity();
            return;
        }

        let cloud = self.mode == Mode::Create && self.storage_kind == Some(StorageKind::Cloud);

        if self.mode == Mode::Create && self.storage_kind.is_none() {
            self.error = Some("Choose a vault type".into());
            return;
        }

        if self.mode == Mode::Create && !self.validate_creation() {
            return;
        }

        let Some(path) = self.path.clone() else {
            return;
        };

        if self.password.is_empty() {
            self.error = Some("Enter master password".into());
            return;
        }

        if self.recovering
            && let Some(error) = self.new_password_error()
        {
            self.error = Some(error);
            return;
        }
        let identity = self.identity.clone();
        let phrase = if self.mode == Mode::Create
            && let Some(identity) = identity.as_ref()
        {
            match identity.phrase() {
                Ok(phrase) => phrase,
                Err(error) => {
                    self.error = Some(error.to_string());
                    return;
                }
            }
        } else if self.mode == Mode::Create || self.recovering {
            match RecoveryPhrase::parse(&self.recovery_words) {
                Ok(phrase) => Some(phrase),
                Err(error) => {
                    self.error = Some(error.to_string());
                    return;
                }
            }
        } else {
            None
        };
        // Move the Rust-side password buffer to the worker, which wipes it on drop.
        let password = SecretString::from(std::mem::replace(
            &mut *self.password,
            String::with_capacity(SECRET_CAPACITY),
        ));
        self.repeat.zeroize();
        self.error = None;
        self.waiting_touch_id = false;
        let mode = self.mode;
        let recovering = self.recovering;
        let cloud_creation = cloud.then(|| {
            (
                self.cloud.username.trim().to_owned(),
                self.cloud.space_name.trim().to_owned(),
                ProviderConfiguration {
                    base_url: self.cloud.base_url.clone(),
                    bucket: self.cloud.bucket.clone(),
                    access_key: self.cloud.access_key.to_string(),
                    secret_key: self.cloud.secret_key.to_string(),
                },
            )
        });
        self.pending = Some(Worker::spawn(move |_| {
            let mut recovery_notice = None;
            let mut with_password = true;
            if recovering {
                let phrase = phrase.as_ref().expect("validated recovery phrase");
                Vault::open_with_recovery_phrase(&path, phrase).and_then(|vault| {
                    if vault.signing_error().is_some() {
                        with_password = false;
                        recovery_notice = Some("Recovered read-only access. This backup must be admitted as a new device before its password can be reset.".into());
                        Ok(vault)
                    } else {
                        vault.reset_password_with_phrase(phrase, &password)
                    }
                })
            } else {
                match mode {
                    Mode::Open => Vault::open(&path, &password),
                    Mode::Create => {
                        if let Some((username, space_name, provider)) = cloud_creation {
                            crate::vault::ProfileStore::default().create_shared_with_identity(
                                &path,
                                &password,
                                KdfParams::default(),
                                crate::vault::SharedProfile { username, space_name, provider },
                                &identity.unwrap_or_else(|| Identity::from_phrase(phrase.as_ref().expect("validated recovery phrase"))),
                            )
                        } else if let Some(phrase) = phrase.as_ref() {
                            Vault::create_with_recovery_phrase(
                                &path,
                                &password,
                                KdfParams::default(),
                                phrase,
                            )
                        } else {
                            Vault::create(&path, &password, KdfParams::default())
                        }
                    }
                }
            }
            .map(|vault| Opened { vault, with_password, recovery_notice })
            .map_err(|error| error.to_string())
        }));
    }

    fn submit_identity(&mut self) {
        let Some(path) = self.identity_path.clone() else {
            return;
        };
        let unlocking = self.unlocking_identity() && !self.recovering;
        if self.password.is_empty() {
            self.error = Some("Enter master password".into());
            return;
        }
        if !unlocking {
            if let Some(error) = self.new_password_error() {
                self.error = Some(error);
                return;
            }
            if !self.recovering && !self.validate_recovery_key() {
                return;
            }
        }
        let proposed = if unlocking {
            None
        } else {
            match RecoveryPhrase::parse(&self.recovery_words) {
                Ok(phrase) => Some(Identity::from_phrase(&phrase)),
                Err(error) => {
                    self.error = Some(error.to_string());
                    return;
                }
            }
        };
        let source = self.identity_source.clone();
        let recovering = self.recovering;
        let password = SecretString::from(std::mem::take(&mut *self.password));
        self.repeat.zeroize();
        self.error = None;
        self.pending_identity = Some(Worker::spawn(move |_| {
            (|| {
                if identity_file_present(&path) {
                    if recovering {
                        let phrase = proposed
                            .as_ref()
                            .ok_or(crate::vault::VaultError::RecoveryUnavailable)?
                            .phrase()?
                            .ok_or(crate::vault::VaultError::RecoveryUnavailable)?;
                        return Identity::recover(&path, &phrase, &password);
                    }
                    if let Some(proposed) = proposed.as_ref() {
                        proposed.save_new(&path, &password, KdfParams::default())?;
                    }
                    return Identity::open(&path, &password);
                }
                if (unlocking && source.is_none()) || recovering {
                    return Err(crate::vault::VaultError::RecoveryUnavailable);
                }
                let identity = if let Some(source) = source {
                    let vault = Vault::open(&source, &password)?;
                    Identity::from_vault(&vault, &password)?
                } else {
                    proposed.ok_or(crate::vault::VaultError::RecoveryUnavailable)?
                };
                identity.save_new(&path, &password, KdfParams::default())?;
                Ok(identity)
            })()
            .map_err(|error: crate::vault::VaultError| error.to_string())
        }));
    }
}

impl Locked {
    pub(crate) fn poll(&mut self, biometrics: Arc<dyn Biometrics>) -> Option<Opened> {
        if let Some(result) = self.pending_identity.as_mut().and_then(Worker::take) {
            self.pending_identity = None;
            match result.unwrap_or_else(|_| Err("Unlock was interrupted".into())) {
                Ok(identity) => {
                    self.recovery_words =
                        Zeroizing::new(identity.words().unwrap_or_default().to_owned());
                    self.identity = Some(identity);
                    self.identity_source = None;
                    self.words_saved = true;
                    self.recovering = false;
                    self.show_recovery_words = false;
                    self.creation_step = CreationStep::Sync;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self.pending.as_mut().and_then(Worker::take) {
            match result.unwrap_or_else(|_| Err("Unlock was interrupted".into())) {
                Ok(opened) => return Some(opened),
                Err(error) => {
                    self.error = Some(error);
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
