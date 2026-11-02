use crate::sync::protocol::{
    Error, Result,
    crypto::Signed,
    machine::{Change, Control},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

const OBJECT_DOMAIN: &[u8] = b"lockbox.sync.object\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Zeroize)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn digest(domain: &[u8], bytes: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update(domain);
        hash.update(bytes);
        Self(hash.finalize().into())
    }
}

impl std::fmt::Display for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

impl std::str::FromStr for Hash {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        if s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::InvalidEncoding);
        }

        let mut bytes = [0; 32];

        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[index * 2..index * 2 + 2], 16)
                .map_err(|_| Error::InvalidEncoding)?;
        }

        Ok(Self(bytes))
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Network counters are decimal strings so their precision never depends on JSON numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Counter(pub u64);
impl Serialize for Counter {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for Counter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let value = s.parse::<u64>().map_err(serde::de::Error::custom)?;

        if value.to_string() != s {
            return Err(serde::de::Error::custom("noncanonical counter"));
        }

        Ok(Self(value))
    }
}

/// Public bytes and ciphertext only. Private material uses Zeroizing instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blob(pub Vec<u8>);
impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&URL_SAFE_NO_PAD.encode(&self.0))
    }
}
impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(&s)
            .map_err(serde::de::Error::custom)?;

        if URL_SAFE_NO_PAD.encode(&bytes) != s {
            return Err(serde::de::Error::custom("noncanonical bytes"));
        }

        Ok(Self(bytes))
    }
}

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
        self.encode().map(|(hash, _)| hash)
    }
    /// Serialize once when both the object hash and canonical bytes are needed.
    pub(super) fn encode(&self) -> Result<(Hash, Vec<u8>)> {
        let bytes = self.bytes()?;
        let hash = Hash::digest(OBJECT_DOMAIN, &bytes);
        Ok((hash, bytes))
    }

    /// Check canonical encoding before comparing the complete-object hash.
    pub(crate) fn decode(bytes: &[u8], expected: Hash) -> Result<Self> {
        let object = decode(bytes)?;
        if Hash::digest(OBJECT_DOMAIN, bytes) != expected {
            return Err(Error::HashMismatch);
        }
        Ok(object)
    }
}

pub fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_jcs::to_vec(value).map_err(|_| Error::InvalidEncoding)
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
