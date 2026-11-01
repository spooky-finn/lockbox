//! Initial identity material. No protocol signatures or admission are produced here.

use ed25519_dalek::SigningKey;
#[cfg(test)]
use hkdf::Hkdf;
use hpke::{Deserializable, Kem, Serializable, kem::X25519HkdfSha256};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub(crate) type Key = [u8; 32];

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct DeviceKeys {
    signing_secret: Key,
    hpke_secret: Key,
}

#[derive(Serialize, Deserialize, Zeroize)]
pub(crate) struct PublicIdentity {
    pub(crate) user_id: Key,
    pub(crate) root_key: Key,
    pub(crate) device_id: Key,
    pub(crate) signing_key: Key,
    pub(crate) hpke_key: Key,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct SpaceKeys {
    pub(crate) space_id: Key,
    pub(crate) namespace: Key,
    epoch_key: Key,
    pub(crate) epoch_context: Key,
}

pub(crate) fn random32() -> Result<Zeroizing<Key>, getrandom::Error> {
    let mut bytes = Zeroizing::new([0; 32]);
    getrandom::fill(bytes.as_mut())?;
    Ok(bytes)
}

pub(crate) fn fingerprint(domain: &[u8], key: &Key) -> Key {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(key);
    hash.finalize().into()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl DeviceKeys {
    pub(crate) fn sync_secret(&self) -> crate::core::sync::DeviceSecret {
        crate::core::sync::DeviceSecret::from_parts(self.signing_secret, self.hpke_secret)
    }
    pub(crate) fn generate(root: &SigningKey) -> Result<(Self, PublicIdentity), getrandom::Error> {
        let signing_secret = random32()?;
        let signing_key = SigningKey::from_bytes(&signing_secret)
            .verifying_key()
            .to_bytes();
        let ikm = random32()?;
        // Use the RFC 9180 KEM implementation for key derivation, not custom DH.
        let (hpke_secret, hpke_public) = X25519HkdfSha256::derive_keypair(ikm.as_ref());
        let mut secret_bytes = Zeroizing::new([0; 32]);
        hpke_secret.write_exact(secret_bytes.as_mut());
        let mut public_bytes = [0; 32];
        hpke_public.write_exact(&mut public_bytes);
        let root_key = root.verifying_key().to_bytes();
        Ok((
            Self {
                signing_secret: *signing_secret,
                hpke_secret: *secret_bytes,
            },
            PublicIdentity {
                user_id: fingerprint(b"lockbox.user\0", &root_key),
                root_key,
                device_id: fingerprint(b"lockbox.device\0", &signing_key),
                signing_key,
                hpke_key: public_bytes,
            },
        ))
    }

    pub(crate) fn matches(&self, public: &PublicIdentity) -> bool {
        let Ok(secret) = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(&self.hpke_secret)
        else {
            return false;
        };

        let mut hpke_public = [0; 32];
        X25519HkdfSha256::sk_to_pk(&secret).write_exact(&mut hpke_public);
        SigningKey::from_bytes(&self.signing_secret)
            .verifying_key()
            .to_bytes()
            == public.signing_key
            && hpke_public == public.hpke_key
            && fingerprint(b"lockbox.user\0", &public.root_key) == public.user_id
            && fingerprint(b"lockbox.device\0", &public.signing_key) == public.device_id
            && ed25519_dalek::VerifyingKey::from_bytes(&public.root_key)
                .is_ok_and(|key| !key.is_weak())
    }
}

#[cfg(test)]
pub(crate) type ContentKeys = (Zeroizing<Key>, Zeroizing<Key>);

impl SpaceKeys {
    pub(crate) fn epoch_key(&self) -> &Key {
        &self.epoch_key
    }
    pub(crate) fn generate() -> Result<Self, getrandom::Error> {
        Ok(Self {
            space_id: *random32()?,
            namespace: *random32()?,
            epoch_key: *random32()?,
            epoch_context: *random32()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn content_keys(&self) -> ContentKeys {
        let hkdf = Hkdf::<Sha256>::new(Some(&self.space_id), &self.epoch_key);
        let derive = |domain: &[u8]| {
            let mut info = domain.to_vec();
            info.extend_from_slice(&self.epoch_context);
            let mut key = Zeroizing::new([0; 32]);
            hkdf.expand(&info, key.as_mut())
                .expect("fixed HKDF output size");
            key
        };
        (
            derive(b"lockbox.changes\0"),
            derive(b"lockbox.checkpoint\0"),
        )
    }
}

#[cfg(test)]
#[path = "tests/identity.rs"]
mod tests;
