//! Encrypted device profile, independent root capsule, and durable sync journal.

use crate::{
    sync::identity::{DeviceKeys, PublicIdentity, SpaceKeys},
    vault::{KdfParams, Result, VaultError},
};
use ed25519_dalek::SigningKey;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

mod root_capsule;
use root_capsule::RootCapsule;

pub(crate) mod pairing;
mod sharing;
pub(crate) use sharing::{AcceptedInvitation, IssuedInvitation, SharingState};
pub use sharing::{AcceptedInvitationInfo, InvitationStatus, IssuedInvitationInfo};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct ProviderConfiguration {
    pub base_url: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct BootstrapProfile {
    #[zeroize(skip)]
    schema_version: u16,
    pub(crate) username: String,
    #[serde(default)]
    #[zeroize(skip)]
    pub(crate) participant_names: std::collections::BTreeMap<crate::sync::protocol::Hash, String>,
    pub(crate) space_name: String,
    pub(crate) identity: PublicIdentity,
    device: DeviceKeys,
    pub(crate) space: SpaceKeys,
    pub(crate) provider: ProviderConfiguration,
    #[serde(default)]
    #[zeroize(skip)]
    pub(crate) sync: Option<crate::sync::protocol::SyncState>,
    #[serde(default)]
    #[zeroize(skip)]
    pub(crate) certificate:
        Option<crate::sync::protocol::Signed<crate::sync::protocol::Certificate>>,
    #[serde(default)]
    #[zeroize(skip)]
    pub(crate) sharing: SharingState,
    #[serde(default)]
    #[zeroize(skip)]
    pub(crate) pairing: pairing::PairingState,
    #[serde(default)]
    #[zeroize(skip)]
    pub(crate) control_approvals: std::collections::BTreeMap<
        crate::sync::protocol::Hash,
        crate::sync::protocol::Signed<crate::sync::protocol::ControlApproval>,
    >,
    root_capsule: RootCapsule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[zeroize(skip)]
    pub(crate) cloud_backup: Option<crate::sync::protocol::cloud_recovery::CloudBackup>,
}

impl BootstrapProfile {
    pub(crate) fn generate(
        username: String,
        space_name: String,
        provider: ProviderConfiguration,
        recovery_password: &SecretString,
        kdf: KdfParams,
    ) -> Result<Self> {
        let phrase = crate::access::mnemonic::RecoveryPhrase::generate()?;
        let root = phrase.signing_key();
        Self::generate_for_root(
            &root,
            username,
            space_name,
            provider,
            recovery_password,
            kdf,
        )
    }

    pub(crate) fn from_recovery_phrase(
        username: String,
        space_name: String,
        provider: ProviderConfiguration,
        password: &SecretString,
        kdf: KdfParams,
        phrase: &crate::access::mnemonic::RecoveryPhrase,
    ) -> Result<Self> {
        Self::generate_for_root(
            &phrase.signing_key(),
            username,
            space_name,
            provider,
            password,
            kdf,
        )
    }

    pub(crate) fn reset_root_password(
        &mut self,
        phrase: &crate::access::mnemonic::RecoveryPhrase,
        password: &SecretString,
        kdf: KdfParams,
    ) -> Result<()> {
        let root = phrase.signing_key();
        if root.verifying_key().to_bytes() != self.identity.root_key {
            return Err(VaultError::WrongRecoveryPhrase);
        }
        self.root_capsule = RootCapsule::seal(&root, password, kdf)?;
        Ok(())
    }

    pub(crate) fn generate_for_root(
        root: &SigningKey,
        username: String,
        space_name: String,
        provider: ProviderConfiguration,
        recovery_password: &SecretString,
        kdf: KdfParams,
    ) -> Result<Self> {
        let (device, identity) = DeviceKeys::generate(root).map_err(|_| VaultError::Random)?;
        let root_capsule = RootCapsule::seal(root, recovery_password, kdf)?;
        let space = SpaceKeys::generate().map_err(|_| VaultError::Random)?;
        let secret = device.sync_secret();
        let certificate = secret.certificate(root, identity.hpke_key)?;
        let sync = crate::sync::protocol::Machine::create(
            crate::sync::protocol::Hash(space.space_id),
            crate::sync::protocol::Hash(space.namespace),
            certificate.clone(),
            &secret,
            space.epoch_key(),
            crate::sync::protocol::Hash(space.epoch_context),
        )?;
        Ok(Self {
            schema_version: 5,
            username,
            participant_names: Default::default(),
            space_name,
            identity,
            device,
            space,
            provider,
            sync: Some(sync),
            certificate: Some(certificate),
            sharing: SharingState::default(),
            pairing: pairing::PairingState::default(),
            control_approvals: std::collections::BTreeMap::new(),
            root_capsule,
            cloud_backup: None,
        })
    }

    pub(crate) fn validate(&self) -> bool {
        ((self.schema_version == 1 && self.sync.is_none())
            || ((2..=8).contains(&self.schema_version)
                && self.sync.is_some()
                && self.certificate.is_some()))
            && (self.schema_version >= 4 || self.sharing.is_empty())
            && (self.schema_version >= 5 || self.control_approvals.is_empty())
            && (self.schema_version >= 7 || self.pairing.is_empty())
            && (self.schema_version >= 8 || !self.pairing.uses_short_links())
            && self.pairing.validate()
            && self.control_approvals.len() <= 128
            && self
                .control_approvals
                .iter()
                .all(|(id, approval)| *id == approval.id() && approval.signature.0.len() == 64)
            && self.sharing.validate()
            && self.device.matches(&self.identity)
            && self.root_capsule.validate()
            && !self.username.trim().is_empty()
            && !self.space_name.trim().is_empty()
            && self.sync.as_ref().is_none_or(|sync| {
                sync.space.0 == self.space.space_id && sync.namespace.0 == self.space.namespace
            })
            && self.certificate.as_ref().is_none_or(|certificate| {
                certificate.verify().is_ok()
                    && certificate.body.device.0 == self.identity.device_id
                    && certificate.body.user.0 == self.identity.user_id
                    && certificate.body.root_key == self.identity.root_key
                    && certificate.body.signing_key == self.identity.signing_key
                    && certificate.body.hpke_key == self.identity.hpke_key
            })
    }

    pub(crate) fn unlock_root(&self, password: &SecretString) -> Result<SigningKey> {
        self.root_capsule.open(password, &self.identity.root_key)
    }

    pub(crate) fn sync_secret(&self) -> crate::sync::protocol::DeviceSecret {
        self.device.sync_secret()
    }

    pub(crate) fn requires_witness(&self) -> bool {
        (3..=8).contains(&self.schema_version)
    }

    pub(crate) fn enable_sharing(&mut self) {
        self.schema_version = self.schema_version.max(4);
    }

    pub(crate) fn enable_pairing(&mut self) {
        self.schema_version = self.schema_version.max(8);
    }

    pub(crate) fn enable_authority(&mut self) {
        self.schema_version = self.schema_version.max(5);
    }

    pub(crate) fn enable_cloud_backup(&mut self, root: &SigningKey) -> Result<()> {
        let state = self
            .sync
            .as_ref()
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;
        let target = crate::sync::protocol::cloud_recovery::target(root, state.descriptor())?;
        self.cloud_backup = Some(crate::sync::protocol::cloud_recovery::CloudBackup {
            target,
            archive: Vec::new(),
            upload: None,
        });
        self.schema_version = self.schema_version.max(6);
        Ok(())
    }

    pub(crate) fn approve_control_recovery(
        &mut self,
        recovery_password: &SecretString,
    ) -> Result<()> {
        let root = self
            .unlock_root(recovery_password)
            .map_err(|_| VaultError::RecoveryFailed)?;
        let device = self.sync_secret();
        crate::sync::protocol::Machine::approve_control_recovery(
            self.sync
                .as_mut()
                .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?,
            &device,
            &root,
        )?;
        Ok(())
    }

    pub(crate) fn recover_identity(
        &self,
        recovery_password: &SecretString,
        kdf: KdfParams,
        username: String,
        space_name: String,
        provider: ProviderConfiguration,
    ) -> Result<Self> {
        let root = self
            .unlock_root(recovery_password)
            .map_err(|_| VaultError::RecoveryFailed)?;
        Self::generate_for_root(
            &root,
            username,
            space_name,
            provider,
            recovery_password,
            kdf,
        )
    }

    pub(crate) fn activate_sync(&mut self, recovery_password: &SecretString) -> Result<()> {
        if self.sync.is_some() {
            return Ok(());
        }

        let root = self
            .unlock_root(recovery_password)
            .map_err(|_| VaultError::RecoveryFailed)?;
        // Activation of a copied bootstrap must never resurrect its old signer.
        let (keys, identity) = DeviceKeys::generate(&root).map_err(|_| VaultError::Random)?;
        let device = keys.sync_secret();
        let certificate = device.certificate(&root, identity.hpke_key)?;
        let sync = crate::sync::protocol::Machine::create(
            crate::sync::protocol::Hash(self.space.space_id),
            crate::sync::protocol::Hash(self.space.namespace),
            certificate.clone(),
            &device,
            self.space.epoch_key(),
            crate::sync::protocol::Hash(self.space.epoch_context),
        )?;
        self.certificate = Some(certificate);
        self.sync = Some(sync);
        self.device = keys;
        self.identity = identity;
        self.schema_version = 5;
        Ok(())
    }

    pub(crate) fn cancel_activation(&mut self) {
        self.sync = None;
        self.certificate = None;
        self.schema_version = 1;
    }
}

#[cfg(test)]
#[path = "tests/profile.rs"]
mod tests;
