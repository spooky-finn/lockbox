//! Encrypted device profile, independent root capsule, and durable sync journal.

use crate::{
    sync::identity::{DeviceKeys, Key, PublicIdentity, SpaceKeys},
    vault::{KdfParams, Result, VaultError},
};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

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
    pub(crate) control_approvals: std::collections::BTreeMap<
        crate::sync::protocol::Hash,
        crate::sync::protocol::Signed<crate::sync::protocol::ControlApproval>,
    >,
    root_capsule: RootCapsule,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct RootCapsule {
    #[zeroize(skip)]
    version: u16,
    #[zeroize(skip)]
    kdf: KdfParams,
    salt: [u8; 16],
    nonce: [u8; 24],
    ciphertext: Vec<u8>,
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
            control_approvals: std::collections::BTreeMap::new(),
            root_capsule,
        })
    }

    pub(crate) fn validate(&self) -> bool {
        ((self.schema_version == 1 && self.sync.is_none())
            || ((2..=5).contains(&self.schema_version)
                && self.sync.is_some()
                && self.certificate.is_some()))
            && (self.schema_version >= 4 || self.sharing.is_empty())
            && (self.schema_version >= 5 || self.control_approvals.is_empty())
            && self.control_approvals.len() <= 128
            && self
                .control_approvals
                .iter()
                .all(|(id, approval)| *id == approval.id() && approval.signature.0.len() == 64)
            && self.sharing.validate()
            && self.device.matches(&self.identity)
            && self.root_capsule.version == 1
            && self.root_capsule.kdf.validate().is_ok()
            && self.root_capsule.ciphertext.len() == 48
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
        (3..=5).contains(&self.schema_version)
    }

    pub(crate) fn enable_sharing(&mut self) {
        self.schema_version = self.schema_version.max(4);
    }

    pub(crate) fn enable_authority(&mut self) {
        self.schema_version = 5;
    }

    pub(crate) fn approve_control_recovery(
        &mut self,
        recovery_password: &SecretString,
    ) -> Result<()> {
        let root = self
            .root_capsule
            .open(recovery_password, &self.identity.root_key)
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
            .root_capsule
            .open(recovery_password, &self.identity.root_key)
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
            .root_capsule
            .open(recovery_password, &self.identity.root_key)
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

impl RootCapsule {
    fn key(&self, password: &SecretString) -> Result<Zeroizing<Key>> {
        let stretched = super::format::stretch(password, &self.salt, self.kdf)?;
        let mut key = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(None, stretched.as_ref())
            .expand(b"lockbox.root-capsule.v1\0", key.as_mut())
            .expect("fixed HKDF output size");
        Ok(key)
    }

    fn aad(&self, public_key: &Key) -> Vec<u8> {
        let mut aad = b"lockbox.root-capsule.v1\0".to_vec();
        aad.extend_from_slice(&self.version.to_le_bytes());

        for n in [self.kdf.memory_kib, self.kdf.passes, self.kdf.lanes] {
            aad.extend_from_slice(&n.to_le_bytes());
        }
        aad.extend_from_slice(&self.salt);
        aad.extend_from_slice(public_key);
        aad
    }

    fn seal(root: &SigningKey, password: &SecretString, kdf: KdfParams) -> Result<Self> {
        let mut capsule = Self {
            version: 1,
            kdf: kdf.validate()?,
            salt: [0; 16],
            nonce: [0; 24],
            ciphertext: Vec::new(),
        };
        getrandom::fill(&mut capsule.salt).map_err(|_| VaultError::Random)?;
        getrandom::fill(&mut capsule.nonce).map_err(|_| VaultError::Random)?;
        let key = capsule.key(password)?;
        capsule.ciphertext = XChaCha20Poly1305::new(key.as_ref().into())
            .encrypt(
                XNonce::from_slice(&capsule.nonce),
                Payload {
                    msg: root.as_bytes(),
                    aad: &capsule.aad(&root.verifying_key().to_bytes()),
                },
            )
            .map_err(|_| VaultError::Corrupted)?;
        Ok(capsule)
    }

    fn open(&self, password: &SecretString, public_key: &Key) -> Result<SigningKey> {
        if self.version != 1 {
            return Err(VaultError::Corrupted);
        }

        let key = self.key(password)?;
        let plaintext = Zeroizing::new(
            XChaCha20Poly1305::new(key.as_ref().into())
                .decrypt(
                    XNonce::from_slice(&self.nonce),
                    Payload {
                        msg: &self.ciphertext,
                        aad: &self.aad(public_key),
                    },
                )
                .map_err(|_| VaultError::Corrupted)?,
        );
        let seed: Zeroizing<Key> = Zeroizing::new(
            plaintext
                .as_slice()
                .try_into()
                .map_err(|_| VaultError::Corrupted)?,
        );
        let root = SigningKey::from_bytes(&seed);

        if root.verifying_key().to_bytes() != *public_key {
            return Err(VaultError::Corrupted);
        }

        Ok(root)
    }
}

#[cfg(test)]
#[path = "tests/profile.rs"]
mod tests;
