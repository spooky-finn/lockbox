use crate::sync::{
    identity::Key,
    protocol::{Blob, Error, Hash, Result, canonical},
};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use hpke::{
    Deserializable, Kem, OpModeR, OpModeS, Serializable, aead::ChaCha20Poly1305, kdf::HkdfSha256,
    kem::X25519HkdfSha256,
};
use rand_core::{OsRng, TryRngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub(super) const CERT_DOMAIN: &[u8] = b"lockbox.sync.certificate\0";
pub(super) const CONTROL_DOMAIN: &[u8] = b"lockbox.sync.control\0";
pub(super) const CHANGE_DOMAIN: &[u8] = b"lockbox.sync.change\0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub body: T,
    pub signature: Blob,
}

pub(super) fn sign<T: Serialize>(body: T, key: &SigningKey, domain: &[u8]) -> Result<Signed<T>> {
    let bytes = signing_bytes(&body, domain)?;
    Ok(Signed {
        signature: Blob(key.sign(&bytes).to_bytes().to_vec()),
        body,
    })
}

pub(super) fn verify<T: Serialize>(signed: &Signed<T>, key: &Key, domain: &[u8]) -> Result<()> {
    let key = VerifyingKey::from_bytes(key).map_err(|_| Error::InvalidSignature)?;

    if key.is_weak() {
        return Err(Error::InvalidSignature);
    }

    let signature =
        Signature::from_slice(&signed.signature.0).map_err(|_| Error::InvalidSignature)?;
    let bytes = signing_bytes(&signed.body, domain)?;
    key.verify_strict(&bytes, &signature)
        .map_err(|_| Error::InvalidSignature)
}

fn signing_bytes<T: Serialize>(body: &T, domain: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = domain.to_vec();
    serde_jcs::to_writer(&mut bytes, body).map_err(|_| Error::InvalidEncoding)?;
    Ok(bytes)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Certificate {
    pub user: Hash,
    pub root_key: Key,
    pub device: Hash,
    pub signing_key: Key,
    pub hpke_key: Key,
}

fn device_id(signing_key: &Key) -> Hash {
    Hash::digest(b"lockbox.device\0", signing_key)
}

impl Signed<Certificate> {
    pub fn hash(&self) -> Result<Hash> {
        Ok(Hash::digest(
            b"lockbox.sync.certificate-object\0",
            &canonical(self)?,
        ))
    }

    pub fn verify(&self) -> Result<()> {
        let body = &self.body;

        if body.user != Hash::digest(b"lockbox.user\0", &body.root_key)
            || body.device != device_id(&body.signing_key)
            || VerifyingKey::from_bytes(&body.signing_key).map_or(true, |key| key.is_weak())
        {
            return Err(Error::InvalidSignature);
        }
        verify(self, &body.root_key, CERT_DOMAIN)
    }
}

/// One installation's signing and HPKE secrets. Never copy to admit another device.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct DeviceSecret {
    signing: Key,
    hpke: Key,
}

impl DeviceSecret {
    pub fn generate(root: &SigningKey) -> Result<(Self, Signed<Certificate>)> {
        let signing = random32()?;
        let (private, public) = X25519HkdfSha256::derive_keypair(random32()?.as_ref());
        let mut hpke = Zeroizing::new([0; 32]);
        private.write_exact(hpke.as_mut());
        let secret = Self {
            signing: *signing,
            hpke: *hpke,
        };

        let certificate =
            secret.certificate(root, public.to_bytes().as_slice().try_into().unwrap())?;
        Ok((secret, certificate))
    }

    pub(crate) fn from_parts(signing: Key, hpke: Key) -> Self {
        Self { signing, hpke }
    }

    pub(crate) fn certificate(
        &self,
        root: &SigningKey,
        hpke_key: Key,
    ) -> Result<Signed<Certificate>> {
        let root_key = root.verifying_key().to_bytes();
        let signing_key = self.signer().verifying_key().to_bytes();
        sign(
            Certificate {
                user: Hash::digest(b"lockbox.user\0", &root_key),
                root_key,
                device: device_id(&signing_key),
                signing_key,
                hpke_key,
            },
            root,
            CERT_DOMAIN,
        )
    }

    pub fn id(&self) -> Hash {
        device_id(&self.signer().verifying_key().to_bytes())
    }

    pub(super) fn matches_certificate(&self, certificate: &Certificate) -> bool {
        let Ok(private) = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(&self.hpke) else {
            return false;
        };
        let signing_key = self.signer().verifying_key().to_bytes();
        device_id(&signing_key) == certificate.device
            && signing_key == certificate.signing_key
            && X25519HkdfSha256::sk_to_pk(&private).to_bytes().as_slice() == certificate.hpke_key
    }
    pub(super) fn signer(&self) -> SigningKey {
        SigningKey::from_bytes(&self.signing)
    }

    pub(super) fn open_grant(
        &self,
        enc: &Blob,
        ciphertext: &Blob,
        aad: &[u8],
    ) -> Result<Zeroizing<Key>> {
        let private = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(&self.hpke)
            .map_err(|_| Error::AuthenticationFailed)?;
        let enc = <X25519HkdfSha256 as Kem>::EncappedKey::from_bytes(&enc.0)
            .map_err(|_| Error::AuthenticationFailed)?;
        let mut info = b"lockbox.sync.grant\0".to_vec();
        info.extend(aad);
        let bytes = Zeroizing::new(
            hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
                &OpModeR::Base,
                &private,
                &enc,
                &info,
                &ciphertext.0,
                aad,
            )
            .map_err(|_| Error::AuthenticationFailed)?,
        );
        Ok(Zeroizing::new(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::AuthenticationFailed)?,
        ))
    }
}

pub(super) fn seal_grant(public: &Key, key: &Key, aad: &[u8]) -> Result<(Blob, Blob)> {
    let public = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(public)
        .map_err(|_| Error::InvalidEncoding)?;
    let mut info = b"lockbox.sync.grant\0".to_vec();
    info.extend(aad);
    let (enc, ciphertext) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
            &OpModeS::Base,
            &public,
            &info,
            key,
            aad,
            &mut OsRng.unwrap_err(),
        )
        .map_err(|_| Error::AuthenticationFailed)?;
    Ok((Blob(enc.to_bytes().to_vec()), Blob(ciphertext)))
}

pub(super) fn random32() -> Result<Zeroizing<Key>> {
    let mut value = Zeroizing::new([0; 32]);
    getrandom::fill(value.as_mut()).map_err(|_| Error::Random)?;
    Ok(value)
}

pub(super) fn content_key(
    key: &Key,
    space: Hash,
    context: Hash,
    checkpoint: bool,
) -> Zeroizing<Key> {
    let domain = if checkpoint {
        b"lockbox.checkpoint\0".as_slice()
    } else {
        b"lockbox.changes\0".as_slice()
    };

    let mut info = domain.to_vec();
    info.extend(context.0);
    let mut out = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(&space.0), key)
        .expand(&info, out.as_mut())
        .expect("fixed HKDF length");
    out
}

pub(super) fn encrypt(key: &Key, nonce: &[u8; 24], bytes: &[u8], aad: &[u8]) -> Result<Blob> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(nonce), Payload { msg: bytes, aad })
        .map(Blob)
        .map_err(|_| Error::AuthenticationFailed)
}

pub(super) fn decrypt(
    key: &Key,
    nonce: &[u8; 24],
    bytes: &Blob,
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), Payload { msg: &bytes.0, aad })
        .map(Zeroizing::new)
        .map_err(|_| Error::AuthenticationFailed)
}

/// Check the XChaCha nonce length before passing it to the primitive.
pub(super) fn nonce(blob: &Blob) -> Result<[u8; 24]> {
    blob.0
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidEncoding)
}
