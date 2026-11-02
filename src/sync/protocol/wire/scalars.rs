//! String-encoded identifiers, counters and public bytes.

use crate::sync::protocol::{Error, Result};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

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
        // Emit one string fragment; JCS allocates for each formatted fragment.
        let mut encoded = [0; 64];
        std::io::Write::write_fmt(&mut encoded.as_mut_slice(), format_args!("{self}"))
            .expect("64-byte hexadecimal hash");
        serializer.serialize_str(std::str::from_utf8(&encoded).expect("hexadecimal is ASCII"))
    }
}
impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserialize_text(deserializer)
    }
}
impl WireText for Hash {
    fn from_text<E: serde::de::Error>(text: &str) -> std::result::Result<Self, E> {
        text.parse().map_err(E::custom)
    }
}

/// Network counters are decimal strings so their precision never depends on JSON numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Counter(pub u64);
impl Serialize for Counter {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for Counter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserialize_text(deserializer)
    }
}
impl WireText for Counter {
    fn from_text<E: serde::de::Error>(s: &str) -> std::result::Result<Self, E> {
        let value = s.parse::<u64>().map_err(E::custom)?;

        if s.starts_with('+') || (s.len() > 1 && s.starts_with('0')) {
            return Err(E::custom("noncanonical counter"));
        }

        Ok(Self(value))
    }
}

/// Public bytes and ciphertext only. Private material uses Zeroizing instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blob(pub Vec<u8>);
impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut encoded = [0; 128];
        match URL_SAFE_NO_PAD.encode_slice(&self.0, &mut encoded) {
            Ok(len) => serializer
                .serialize_str(std::str::from_utf8(&encoded[..len]).expect("base64 is ASCII")),
            Err(_) => serializer.serialize_str(&URL_SAFE_NO_PAD.encode(&self.0)),
        }
    }
}
impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserialize_text(deserializer)
    }
}
impl WireText for Blob {
    fn from_text<E: serde::de::Error>(s: &str) -> std::result::Result<Self, E> {
        // This engine rejects padding and nonzero unused trailing bits.
        URL_SAFE_NO_PAD.decode(s).map(Self).map_err(E::custom)
    }
}

trait WireText: Sized {
    fn from_text<E: serde::de::Error>(text: &str) -> std::result::Result<Self, E>;
}

struct TextVisitor<T, E>(std::marker::PhantomData<(T, E)>);

impl<'de, T: WireText, E: serde::de::Error> serde::de::Visitor<'de> for TextVisitor<T, E> {
    type Value = std::result::Result<T, E>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a string")
    }

    fn visit_str<F: serde::de::Error>(self, text: &str) -> std::result::Result<Self::Value, F> {
        Ok(T::from_text(text))
    }

    fn visit_bytes<F: serde::de::Error>(self, bytes: &[u8]) -> std::result::Result<Self::Value, F> {
        match std::str::from_utf8(bytes) {
            Ok(text) => Ok(T::from_text(text)),
            Err(_) => Err(F::invalid_value(serde::de::Unexpected::Bytes(bytes), &self)),
        }
    }
}

fn deserialize_text<'de, D: Deserializer<'de>, T: WireText>(
    deserializer: D,
) -> std::result::Result<T, D::Error> {
    // Return parse errors after string deserialization, preserving their original context.
    deserializer.deserialize_string(TextVisitor::<T, D::Error>(std::marker::PhantomData))?
}

#[cfg(test)]
mod tests;
