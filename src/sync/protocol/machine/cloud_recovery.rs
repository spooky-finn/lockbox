//! Root-addressed encrypted recovery capsules. Device signing secrets never enter
//! this format. Capsules authenticate a recoverable snapshot, not its freshness.

use crate::sync::protocol::{
    Blob, DeviceSecret, Error, Hash, Machine, Object, RecoveryAnchor, Result, Signed,
    SpaceDescriptor, SyncState, canonical, crypto, decode, machine::history::with_history,
};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use hpke::{
    Deserializable, Kem, OpModeR, OpModeS, Serializable, aead::ChaCha20Poly1305, kdf::HkdfSha256,
    kem::X25519HkdfSha256,
};
use rand_core::{OsRng, TryRngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeSet;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const INFO: &[u8] = b"lockbox.cloud-recovery.hpke.v1\0";
const TARGET_DOMAIN: &[u8] = b"lockbox.cloud-recovery.target.v1\0";
const SNAPSHOT_DOMAIN: &[u8] = b"lockbox.cloud-recovery.snapshot.v1\0";
const OBJECT_DOMAIN: &[u8] = b"lockbox.cloud-recovery.object.v1\0";

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(crate) struct EpochKey {
    #[zeroize(skip)]
    pub(crate) membership: Hash,
    pub(crate) key: [u8; 32],
}

impl EpochKey {
    pub(crate) fn new(membership: Hash, key: [u8; 32]) -> Self {
        Self { membership, key }
    }
}

/// Root-signed encryption destination, retained in the encrypted local profile.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryTarget {
    pub descriptor: SpaceDescriptor,
    pub directory: Hash,
    pub recipient: [u8; 32],
}

/// A retry reuses the exact persisted ciphertext. A successor links only after
/// its predecessor received a provider acknowledgement.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryUpload {
    pub fingerprint: Hash,
    pub hash: Hash,
    pub bytes: Blob,
    pub acknowledged: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloudBackup {
    pub target: Signed<RecoveryTarget>,
    pub archive: Vec<EpochKey>,
    pub upload: Option<RecoveryUpload>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    encapsulated: Blob,
    ciphertext: Blob,
}

impl Envelope {
    fn decode_with_hash(bytes: &[u8]) -> Result<(Hash, Self)> {
        let envelope: Self = decode(bytes)?;
        if envelope.version != 1
            || envelope.encapsulated.0.len() != 32
            || envelope.ciphertext.0.len() < 16
        {
            return Err(Error::InvalidEncoding);
        }
        Ok((Hash::digest(OBJECT_DOMAIN, bytes), envelope))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    target: Signed<RecoveryTarget>,
    username: String,
    space_name: String,
    anchor: RecoveryAnchor,
    previous: Option<Hash>,
    controls: Vec<Blob>,
    keys: Vec<EpochKey>,
}

/// Decrypted recovery candidate. Contains epoch keys, never old device secrets.
/// Only candidates returned by authenticated discovery may be restored.
#[derive(Clone)]
pub struct CloudSpace {
    pub(crate) hash: Hash,
    pub(crate) bytes: Blob,
    snapshot: Snapshot,
    controls: BTreeSet<Hash>,
}

impl CloudSpace {
    pub fn name(&self) -> &str {
        &self.snapshot.space_name
    }
    pub fn id(&self) -> Hash {
        self.snapshot.anchor.descriptor.space
    }
    pub fn anchor(&self) -> &RecoveryAnchor {
        &self.snapshot.anchor
    }
    pub(crate) fn username(&self) -> &str {
        &self.snapshot.username
    }
    pub(crate) fn previous(&self) -> Option<Hash> {
        self.snapshot.previous
    }
    pub(crate) fn keys(&self) -> &[EpochKey] {
        &self.snapshot.keys
    }
    pub(crate) fn target(&self) -> Signed<RecoveryTarget> {
        self.snapshot.target.clone()
    }

    pub(crate) fn state(&self) -> Result<SyncState> {
        proof_state(&self.snapshot)
    }
    pub(crate) fn follows(&self, parent: &Self) -> Result<bool> {
        if self.id() != parent.id() || self.anchor().descriptor != parent.anchor().descriptor {
            return Ok(false);
        }
        Ok(self.controls.contains(&parent.anchor().control))
    }

    pub(crate) fn inherit_history<'a>(
        &mut self,
        sources: impl Iterator<Item = &'a CloudSpace>,
    ) -> Result<()> {
        let mut keys = std::collections::BTreeMap::new();
        for source in sources {
            if !self.follows(source)? {
                continue;
            }
            for key in &source.snapshot.keys {
                if let Some(previous) = keys.insert(key.membership, key.clone())
                    && previous.key != key.key
                {
                    return Err(Error::AuthenticationFailed);
                }
            }
        }
        self.snapshot.keys = keys.into_values().collect();
        Ok(())
    }
}

fn material(root: &SigningKey, label: &[u8]) -> Zeroizing<[u8; 32]> {
    let secret = Zeroizing::new(root.to_bytes());
    let mut out = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(b"lockbox.cloud-recovery.v1\0"), secret.as_ref())
        .expand(label, out.as_mut())
        .expect("fixed HKDF length");
    out
}

pub(crate) fn directory(root: &SigningKey) -> Hash {
    Hash(*material(root, b"private-directory\0"))
}

fn keypair(
    root: &SigningKey,
) -> (
    <X25519HkdfSha256 as Kem>::PrivateKey,
    <X25519HkdfSha256 as Kem>::PublicKey,
) {
    X25519HkdfSha256::derive_keypair(material(root, b"recovery-encryption\0").as_ref())
}

pub(crate) fn target(
    root: &SigningKey,
    descriptor: SpaceDescriptor,
) -> Result<Signed<RecoveryTarget>> {
    if descriptor.owner_root != root.verifying_key().to_bytes() {
        return Err(Error::UnauthorizedDevice);
    }
    let (_, public) = keypair(root);
    crypto::sign(
        RecoveryTarget {
            descriptor,
            directory: directory(root),
            recipient: public.to_bytes().as_slice().try_into().unwrap(),
        },
        root,
        TARGET_DOMAIN,
    )
}

pub(crate) fn validate_target(
    target: &Signed<RecoveryTarget>,
    descriptor: &SpaceDescriptor,
) -> Result<()> {
    if &target.body.descriptor != descriptor {
        return Err(Error::InvalidSignature);
    }
    crypto::verify(target, &descriptor.owner_root, TARGET_DOMAIN)
}

/// Return a stable snapshot fingerprint before spending randomness or signing.
fn snapshot(
    state: &SyncState,
    device: &DeviceSecret,
    backup: &CloudBackup,
    username: &str,
    space_name: &str,
) -> Result<Snapshot> {
    validate_target(&backup.target, &state.descriptor())?;
    let anchor = Machine::recovery_anchor(state, device)?;
    let controls = with_history(state, |history| {
        if history.tip().1.body.controller != device.id() {
            return Err(Error::InsufficientRole);
        }
        Ok(history
            .chain
            .iter()
            .map(|(hash, _)| state.objects[hash].clone())
            .collect())
    })?;
    Ok(Snapshot {
        target: backup.target.clone(),
        username: username.into(),
        space_name: space_name.into(),
        anchor,
        previous: None,
        controls,
        keys: Machine::recovery_epoch_keys(state, device, &backup.archive)?,
    })
}

pub(crate) fn prepare(
    state: &SyncState,
    device: &DeviceSecret,
    backup: &CloudBackup,
    username: &str,
    space_name: &str,
) -> Result<Option<RecoveryUpload>> {
    let mut snapshot = snapshot(state, device, backup, username, space_name)?;
    let fingerprint = Hash::digest_canonical(SNAPSHOT_DOMAIN, &snapshot)?;
    if let Some(upload) = &backup.upload {
        if !upload.acknowledged {
            return Ok(Some(upload.clone()));
        }
        if upload.fingerprint == fingerprint {
            return Ok(None);
        }
        snapshot.previous = Some(upload.hash);
    }
    let plaintext = Zeroizing::new(canonical(&crypto::sign(
        snapshot,
        &device.signer(),
        SNAPSHOT_DOMAIN,
    )?)?);
    if plaintext.len() > crate::sync::protocol::MAX_OBJECT_BYTES / 2 {
        return Err(Error::ResourceBudgetExceeded);
    }
    let recipient = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(&backup.target.body.recipient)
        .map_err(|_| Error::InvalidEncoding)?;
    let (enc, ciphertext) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
            &OpModeS::Base,
            &recipient,
            INFO,
            &plaintext,
            &backup.target.body.directory.0,
            &mut OsRng.unwrap_err(),
        )
        .map_err(|_| Error::AuthenticationFailed)?;
    let bytes = canonical(&Envelope {
        version: 1,
        encapsulated: Blob(enc.to_bytes().to_vec()),
        ciphertext: Blob(ciphertext),
    })?;
    let hash = object_hash(&bytes)?;
    Ok(Some(RecoveryUpload {
        fingerprint,
        hash,
        bytes: Blob(bytes),
        acknowledged: false,
    }))
}

pub(crate) fn is_current(
    state: &SyncState,
    device: &DeviceSecret,
    backup: &CloudBackup,
    username: &str,
    space_name: &str,
) -> Result<bool> {
    let Some(upload) = &backup.upload else {
        return Ok(false);
    };
    if !upload.acknowledged {
        return Ok(false);
    }
    let snapshot = snapshot(state, device, backup, username, space_name)?;
    Ok(upload.fingerprint == Hash::digest_canonical(SNAPSHOT_DOMAIN, &snapshot)?)
}

pub(crate) fn object_hash(bytes: &[u8]) -> Result<Hash> {
    Envelope::decode_with_hash(bytes).map(|(hash, _)| hash)
}

pub(crate) fn open(root: &SigningKey, hash: Hash, bytes: &[u8]) -> Result<CloudSpace> {
    let (actual, envelope) = Envelope::decode_with_hash(bytes)?;
    if actual != hash {
        return Err(Error::HashMismatch);
    }
    let (private, public) = keypair(root);
    let enc = <X25519HkdfSha256 as Kem>::EncappedKey::from_bytes(&envelope.encapsulated.0)
        .map_err(|_| Error::InvalidEncoding)?;
    let plaintext = Zeroizing::new(
        hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeR::Base,
            &private,
            &enc,
            INFO,
            &envelope.ciphertext.0,
            &directory(root).0,
        )
        .map_err(|_| Error::AuthenticationFailed)?,
    );
    let signed: Signed<Snapshot> = decode(&plaintext)?;
    let body = &signed.body;
    if body.target.body.descriptor.owner_root != root.verifying_key().to_bytes()
        || body.target.body.directory != directory(root)
        || body.target.body.recipient.as_slice() != public.to_bytes().as_slice()
        || !valid_name(&body.space_name)
        || !valid_name(&body.username)
        || body.keys.len() > crate::sync::protocol::MAX_OBJECTS
        || body.anchor.frontier.len() > 512
        || !crate::sync::protocol::wire::sorted_unique(body.anchor.frontier.iter())
        || !crate::sync::protocol::wire::sorted_unique(body.keys.iter().map(|key| key.membership))
    {
        return Err(Error::InvalidEncoding);
    }
    let state = proof_state(body)?;
    let controls = with_history(&state, |history| {
        let (head, control) = history.tip();
        if head != body.anchor.control {
            return Err(Error::MissingDependency);
        }
        let controller = control
            .body
            .members
            .iter()
            .find(|m| m.certificate.body.device == control.body.controller)
            .ok_or(Error::UnauthorizedDevice)?;
        crypto::verify(
            &signed,
            &controller.certificate.body.signing_key,
            SNAPSHOT_DOMAIN,
        )?;
        let memberships: BTreeSet<_> = history.chain.iter().map(|(hash, _)| *hash).collect();
        if !body.keys.iter().any(|key| key.membership == head)
            || body
                .keys
                .iter()
                .any(|key| !memberships.contains(&key.membership))
        {
            return Err(Error::KeyUnavailable);
        }
        Ok(memberships)
    })?;
    Ok(CloudSpace {
        hash,
        bytes: Blob(bytes.to_vec()),
        snapshot: signed.body,
        controls,
    })
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

fn proof_state(snapshot: &Snapshot) -> Result<SyncState> {
    let descriptor = &snapshot.anchor.descriptor;
    validate_target(&snapshot.target, descriptor)?;
    if snapshot.controls.len() > crate::sync::protocol::MAX_OBJECTS {
        return Err(Error::ResourceBudgetExceeded);
    }
    let mut state = Machine::join_at(descriptor.clone(), snapshot.anchor.control);
    for bytes in &snapshot.controls {
        let object: Object = decode(&bytes.0)?;
        if !matches!(object, Object::Control(_)) {
            return Err(Error::InvalidEncoding);
        }
        let hash = object.hash()?;
        if state.objects.insert(hash, bytes.clone()).is_some() {
            return Err(Error::InvalidEncoding);
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests;
