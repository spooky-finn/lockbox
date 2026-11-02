//! Canonical object envelopes and wire validation.

mod scalars;
pub use scalars::{Blob, Counter, Hash};

use crate::sync::protocol::{
    Error, Result,
    crypto::Signed,
    machine::{Change, Control},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

const OBJECT_DOMAIN: &[u8] = b"lockbox.sync.object\0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "object",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Object {
    Control(Signed<Control>),
    Change(Signed<Change>),
}

impl Object {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        canonical(self)
    }
    pub fn hash(&self) -> Result<Hash> {
        Hash::digest_canonical(OBJECT_DOMAIN, self)
    }
    /// Serialize once when both the object hash and canonical bytes are needed.
    pub(super) fn encode(&self) -> Result<(Hash, Vec<u8>)> {
        let bytes = self.bytes()?;
        let hash = Hash::digest(OBJECT_DOMAIN, &bytes);
        Ok((hash, bytes))
    }

    /// Check canonical encoding before comparing the complete-object hash.
    pub(crate) fn decode(bytes: &[u8], expected: Hash) -> Result<Self> {
        let (hash, object) = Self::decode_with_hash(bytes)?;
        if hash != expected {
            return Err(Error::HashMismatch);
        }
        Ok(object)
    }

    /// Decode canonical bytes before computing their complete-object address.
    pub(super) fn decode_with_hash(bytes: &[u8]) -> Result<(Hash, Self)> {
        let object = decode(bytes)?;
        Ok((Hash::digest(OBJECT_DOMAIN, bytes), object))
    }
}

pub fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_jcs::to_vec(value).map_err(|_| Error::InvalidEncoding)
}

impl Hash {
    /// Hash canonical bytes without retaining their final serialized buffer.
    pub(crate) fn digest_canonical(domain: &[u8], value: &impl Serialize) -> Result<Self> {
        let mut hash = Sha256::new();
        hash.update(domain);
        serde_jcs::to_writer(&mut hash, value).map_err(|_| Error::InvalidEncoding)?;
        Ok(Self(hash.finalize().into()))
    }
}

pub(crate) fn canonical_eq<T: Serialize>(left: &T, right: &T) -> Result<bool> {
    let bytes = canonical(left)?;
    crate::serialization::canonical_matches(right, &bytes).map_err(|_| Error::InvalidEncoding)
}

/// This profile requires canonical bytes on the wire. Re-encoding the typed
/// value rejects duplicate keys, unknown nested fields, omitted fields and all
/// alternate spellings before any verification or state mutation can occur.
pub fn decode<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T> {
    decode_bounded(bytes, crate::sync::protocol::MAX_OBJECT_BYTES)
}

pub(super) fn decode_bounded<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    max: usize,
) -> Result<T> {
    if bytes.len() > max {
        return Err(Error::ResourceBudgetExceeded);
    }

    let value: T = serde_json::from_slice(bytes).map_err(|_| Error::InvalidEncoding)?;

    if !crate::serialization::canonical_matches(&value, bytes)
        .map_err(|_| Error::InvalidEncoding)?
    {
        return Err(Error::InvalidEncoding);
    }

    Ok(value)
}

pub(super) fn sorted_unique<T: Ord>(items: impl IntoIterator<Item = T>) -> bool {
    items.into_iter().is_sorted_by(|a, b| a < b)
}
