use super::{Blob, Error, Hash, Result, canonical};
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

pub(super) const CERT_DOMAIN: &[u8] = b"lockbox.sync.certificate.v0\0";
pub(super) const CONTROL_DOMAIN: &[u8] = b"lockbox.sync.control.v0\0";
pub(super) const CHANGE_DOMAIN: &[u8] = b"lockbox.sync.change.v0\0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub body: T,
    pub signature: Blob,
}

pub(super) fn sign<T: Serialize>(body: T, key: &SigningKey, domain: &[u8]) -> Result<Signed<T>> {
    let mut bytes = domain.to_vec();
    bytes.extend(canonical(&body)?);
    Ok(Signed {
        signature: Blob(key.sign(&bytes).to_bytes().to_vec()),
        body,
    })
}

pub(super) fn verify<T: Serialize>(
    signed: &Signed<T>,
    key: &[u8; 32],
    domain: &[u8],
) -> Result<()> {
    let key = VerifyingKey::from_bytes(key).map_err(|_| Error::InvalidSignature)?;

    if key.is_weak() {
        return Err(Error::InvalidSignature);
    }

    let signature =
        Signature::from_slice(&signed.signature.0).map_err(|_| Error::InvalidSignature)?;
    let mut bytes = domain.to_vec();
    bytes.extend(canonical(&signed.body)?);
    key.verify_strict(&bytes, &signature)
        .map_err(|_| Error::InvalidSignature)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Certificate {
    pub user: Hash,
    pub root_key: [u8; 32],
    pub device: Hash,
    pub signing_key: [u8; 32],
    pub hpke_key: [u8; 32],
}

impl Signed<Certificate> {
    pub fn hash(&self) -> Result<Hash> {
        Ok(Hash::digest(
            b"lockbox.sync.certificate-object.v0\0",
            &canonical(self)?,
        ))
    }

    pub fn verify(&self) -> Result<()> {
        let body = &self.body;

        if body.user != Hash::digest(b"lockbox.user.v1\0", &body.root_key)
            || body.device != Hash::digest(b"lockbox.device.v1\0", &body.signing_key)
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
    signing: [u8; 32],
    hpke: [u8; 32],
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

    pub(crate) fn from_parts(signing: [u8; 32], hpke: [u8; 32]) -> Self {
        Self { signing, hpke }
    }

    pub(crate) fn certificate(
        &self,
        root: &SigningKey,
        hpke_key: [u8; 32],
    ) -> Result<Signed<Certificate>> {
        let root_key = root.verifying_key().to_bytes();
        let signing_key = self.signer().verifying_key().to_bytes();
        sign(
            Certificate {
                user: Hash::digest(b"lockbox.user.v1\0", &root_key),
                root_key,
                device: self.id(),
                signing_key,
                hpke_key,
            },
            root,
            CERT_DOMAIN,
        )
    }

    pub fn id(&self) -> Hash {
        Hash::digest(
            b"lockbox.device.v1\0",
            &self.signer().verifying_key().to_bytes(),
        )
    }

    pub(super) fn matches_certificate(&self, certificate: &Certificate) -> bool {
        let Ok(private) = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(&self.hpke) else {
            return false;
        };
        self.id() == certificate.device
            && self.signer().verifying_key().to_bytes() == certificate.signing_key
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
    ) -> Result<Zeroizing<[u8; 32]>> {
        let private = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(&self.hpke)
            .map_err(|_| Error::AuthenticationFailed)?;
        let enc = <X25519HkdfSha256 as Kem>::EncappedKey::from_bytes(&enc.0)
            .map_err(|_| Error::AuthenticationFailed)?;
        let mut info = b"lockbox.sync.grant.v0\0".to_vec();
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

pub(super) fn seal_grant(public: &[u8; 32], key: &[u8; 32], aad: &[u8]) -> Result<(Blob, Blob)> {
    let public = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(public)
        .map_err(|_| Error::InvalidEncoding)?;
    let mut info = b"lockbox.sync.grant.v0\0".to_vec();
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

pub(super) fn random32() -> Result<Zeroizing<[u8; 32]>> {
    let mut value = Zeroizing::new([0; 32]);
    getrandom::fill(value.as_mut()).map_err(|_| Error::Random)?;
    Ok(value)
}

pub(super) fn content_key(
    key: &[u8; 32],
    space: Hash,
    context: Hash,
    checkpoint: bool,
) -> Zeroizing<[u8; 32]> {
    let domain = if checkpoint {
        b"lockbox.checkpoint.v1\0".as_slice()
    } else {
        b"lockbox.changes.v1\0".as_slice()
    };

    let mut info = domain.to_vec();
    info.extend(context.0);
    let mut out = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(&space.0), key)
        .expand(&info, out.as_mut())
        .expect("fixed HKDF length");
    out
}

pub(super) fn encrypt(key: &[u8; 32], nonce: &[u8; 24], bytes: &[u8], aad: &[u8]) -> Result<Blob> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(nonce), Payload { msg: bytes, aad })
        .map(Blob)
        .map_err(|_| Error::AuthenticationFailed)
}

pub(super) fn decrypt(
    key: &[u8; 32],
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
