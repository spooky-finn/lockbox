//! Identity setup, unlock, migration and recovery workers.

use super::{CreationStep, Locked, identity_file_present};
use crate::{
    access::mnemonic::RecoveryPhrase,
    platform::jobs::Worker,
    vault::{KdfParams, Vault, identity::Identity},
};
use secrecy::SecretString;
use zeroize::{Zeroize, Zeroizing};

impl Locked {
    pub(super) fn submit_identity(&mut self) {
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

    pub(super) fn poll_identity(&mut self) {
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
    }
}
