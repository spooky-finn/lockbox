//! Owner cloud discovery and restore workers. No network work runs in the view.

use super::{CreationStep, Locked, Opened, StorageKind};
use crate::{
    access::mnemonic::RecoveryPhrase,
    platform::jobs::Worker,
    sync::transport::storj::Storj,
    vault::{
        CloudRecoveryTrust, CloudRestore, CloudSpace, KdfParams, ProfileStore,
        ProviderConfiguration,
    },
};
use ed25519_dalek::SigningKey;
use secrecy::SecretString;
use zeroize::Zeroize;

#[derive(Default)]
pub(crate) struct CloudRecoverySetup {
    pub restoring: bool,
    pub spaces: Vec<CloudSpace>,
    pub selected: Option<usize>,
    pub trust_snapshot: bool,
    pub pending: Option<Worker<Result<Vec<CloudSpace>, String>>>,
}

impl Locked {
    pub(crate) fn begin_cloud_restore() -> Self {
        let mut form = Self::begin_create();
        form.recovery_words.zeroize();
        form.importing_words = true;
        form.storage_kind = Some(StorageKind::Cloud);
        form.cloud_recovery.restoring = true;
        form.cloud.username = "Recovery".into();
        form.cloud.space_name = "Recovery".into();
        form
    }

    fn cloud_root(&self) -> Result<SigningKey, String> {
        self.identity
            .as_ref()
            .map(|identity| Ok(identity.signing_key()))
            .unwrap_or_else(|| {
                RecoveryPhrase::parse(&self.recovery_words)
                    .map(|phrase| phrase.signing_key())
                    .map_err(|error| error.to_string())
            })
    }

    fn cloud_provider(&self) -> ProviderConfiguration {
        ProviderConfiguration {
            base_url: self.cloud.base_url.clone(),
            bucket: self.cloud.bucket.clone(),
            access_key: self.cloud.access_key.to_string(),
            secret_key: self.cloud.secret_key.to_string(),
        }
    }

    pub(super) fn discover_cloud(&mut self) {
        let root = match self.cloud_root() {
            Ok(root) => root,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let provider = self.cloud_provider();
        self.cloud_recovery.spaces.clear();
        self.cloud_recovery.selected = None;
        self.cloud_recovery.trust_snapshot = false;
        self.cloud_recovery.pending = Some(Worker::spawn(move |cancel| {
            (|| {
                let mut storage = Storj::new(
                    &provider.base_url,
                    &provider.bucket,
                    "Recovery",
                    &provider.access_key,
                    &provider.secret_key,
                )?;
                ProfileStore::discover_cloud_spaces(&root, &mut storage, &cancel)
            })()
            .map_err(|error: crate::vault::VaultError| error.to_string())
        }));
    }

    pub(super) fn poll_cloud_recovery(&mut self) {
        if let Some(result) = self.cloud_recovery.pending.as_mut().and_then(Worker::take) {
            self.cloud_recovery.pending = None;
            match result.unwrap_or_else(|_| Err("Cloud recovery was interrupted".into())) {
                Ok(spaces) => {
                    self.cloud_recovery.spaces = spaces;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    pub(crate) fn choose_cloud_recovery(&mut self, index: usize) {
        if self.busy()
            || !self.cloud_recovery.restoring
            || index >= self.cloud_recovery.spaces.len()
        {
            return;
        }
        self.cloud_recovery.selected = Some(index);
        self.cloud_recovery.trust_snapshot = false;
        self.creation_step = CreationStep::Password;
        self.error = None;
    }

    pub(super) fn submit_cloud_restore(&mut self) {
        if !self.validate_creation() {
            return;
        }
        if !self.cloud_recovery.trust_snapshot {
            self.error = Some("Confirm that you trust the available cloud snapshot before replacing the lost controller".into());
            return;
        }
        let Some(space) = self
            .cloud_recovery
            .selected
            .and_then(|i| self.cloud_recovery.spaces.get(i))
            .cloned()
        else {
            self.error = Some("Select a cloud space to recover".into());
            return;
        };
        let Some(path) = self.path.clone() else {
            return;
        };
        let root = match self.cloud_root() {
            Ok(root) => root,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let phrase = if let Some(identity) = &self.identity {
            identity.phrase()
        } else {
            RecoveryPhrase::parse(&self.recovery_words)
                .map(Some)
                .map_err(Into::into)
        };
        let phrase = match phrase {
            Ok(phrase) => phrase,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let provider = self.cloud_provider();
        let password = SecretString::from(std::mem::take(&mut *self.password));
        self.repeat.zeroize();
        self.error = None;
        self.pending = Some(Worker::spawn(move |cancel| {
            (|| {
                cancel.check()?;
                if path.exists() {
                    let mut vault = crate::vault::Vault::open(&path, &password)?;
                    cancel.check()?;
                    if vault.space_descriptor().as_ref() != Some(&space.anchor().descriptor)
                        || vault.device_certificate().is_none_or(|certificate| {
                            certificate.body.root_key != root.verifying_key().to_bytes()
                        })
                    {
                        return Err(crate::vault::VaultError::IdentityMismatch);
                    }
                    if vault.pending_control_approval()
                        == Some(crate::sync::protocol::ControlTransition::Recovery)
                    {
                        vault.approve_controller_recovery(&password)?;
                    }
                    if vault.signing_error().is_some()
                        || !vault
                            .sync_replay_state()?
                            .is_some_and(|replay| replay.ready)
                    {
                        return Err(crate::vault::VaultError::RecoveryFailed);
                    }
                    return Ok(Opened {
                        vault,
                        with_password: true,
                        recovery_notice: None,
                    });
                }
                let mut storage = Storj::new(
                    &provider.base_url,
                    &provider.bucket,
                    space.name(),
                    &provider.access_key,
                    &provider.secret_key,
                )?;
                let vault = ProfileStore::default().restore_cloud_space(
                    &path,
                    &password,
                    KdfParams::default(),
                    CloudRestore {
                        root: &root,
                        space,
                        provider,
                        trust: CloudRecoveryTrust::TrustProviderSnapshot,
                        phrase: phrase.as_ref(),
                        cancel,
                    },
                    &mut storage,
                )?;
                Ok(Opened {
                    vault,
                    with_password: true,
                    recovery_notice: None,
                })
            })()
            .map_err(|error: crate::vault::VaultError| error.to_string())
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_requires_explicit_snapshot_trust_before_starting_any_worker() {
        let mut form = Locked::begin_cloud_restore();
        form.recovery_words = zeroize::Zeroizing::new(
            secrecy::ExposeSecret::expose_secret(&RecoveryPhrase::generate().unwrap().words())
                .to_owned(),
        );
        form.words_saved = true;
        form.creation_step = CreationStep::Password;
        form.cloud.base_url = "https://gateway.storjshare.io".into();
        form.cloud.bucket = "fictional-bucket".into();
        form.cloud.access_key.push_str("fictional-access");
        form.cloud.secret_key.push_str("fictional-secret");
        form.password.push_str("fictional password");
        form.repeat.push_str("fictional password");
        form.submit();
        assert!(form.pending.is_none());
        assert!(
            form.error
                .as_ref()
                .unwrap()
                .starts_with("Confirm that you trust")
        );
        form.cloud_recovery.trust_snapshot = true;
        form.submit();
        assert!(form.pending.is_none());
        assert_eq!(
            form.error.as_deref(),
            Some("Select a cloud space to recover")
        );
    }
}
