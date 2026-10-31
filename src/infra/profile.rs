//! Encrypted bootstrap profile; protocol admission and publishing remain disabled.
use super::vault::{KdfParams, Result, VaultError};
use crate::core::identity::{DeviceKeys, PublicIdentity, SpaceKeys, random32};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct ProviderConfiguration {
    pub(crate) base_url: String,
    pub(crate) bucket: String,
    pub(crate) access_key: String,
    pub(crate) secret_key: String,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct BootstrapProfile {
    #[zeroize(skip)]
    schema_version: u16,
    pub(crate) username: String,
    pub(crate) space_name: String,
    pub(crate) identity: PublicIdentity,
    device: DeviceKeys,
    pub(crate) space: SpaceKeys,
    pub(crate) provider: ProviderConfiguration,
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
        let root = SigningKey::from_bytes(&*random32().map_err(|_| VaultError::Random)?);
        let (device, identity) = DeviceKeys::generate(&root).map_err(|_| VaultError::Random)?;
        let root_capsule = RootCapsule::seal(&root, recovery_password, kdf)?;
        let space = SpaceKeys::generate().map_err(|_| VaultError::Random)?;
        Ok(Self {
            schema_version: 1,
            username,
            space_name,
            identity,
            device,
            space,
            provider,
            root_capsule,
        })
    }

    pub(crate) fn validate(&self) -> bool {
        self.schema_version == 1
            && self.device.matches(&self.identity)
            && self.root_capsule.version == 1
            && self.root_capsule.kdf.validate().is_ok()
            && self.root_capsule.ciphertext.len() == 48
            && !self.username.trim().is_empty()
            && !self.space_name.trim().is_empty()
    }
}

impl RootCapsule {
    fn key(&self, password: &SecretString) -> Result<Zeroizing<[u8; 32]>> {
        let kdf = self.kdf.validate()?;
        let params = Params::new(kdf.memory_kib, kdf.passes, kdf.lanes, Some(32))
            .map_err(|_| VaultError::InvalidKdfParams)?;
        let mut stretched = Zeroizing::new([0u8; 32]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(
                password.expose_secret().as_bytes(),
                &self.salt,
                stretched.as_mut(),
            )
            .map_err(|_| VaultError::InvalidKdfParams)?;
        let mut key = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(None, stretched.as_ref())
            .expand(b"lockbox.root-capsule.v1\0", key.as_mut())
            .expect("fixed HKDF output size");
        Ok(key)
    }

    fn aad(&self, public_key: &[u8; 32]) -> Vec<u8> {
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

    #[cfg(test)]
    fn open(&self, password: &SecretString, public_key: &[u8; 32]) -> Result<SigningKey> {
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
        let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
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
