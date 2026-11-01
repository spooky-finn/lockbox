use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// One saved account. Every text field is wiped from memory when the entry is dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    /// Kept as a separate field so TOTP can later move to its own vault file.
    pub totp: Option<Totp>,
    /// Icon chosen by hand (a Simple Icons slug); `None` picks one from the URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Stable folder identifier; older vaults and unfiled records use the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    /// File names and bytes are part of the encrypted entry, locally and in sync.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

impl Entry {
    /// A new empty entry with a random identifier.
    pub fn new() -> Self {
        let mut entry = Self::default();
        entry.id = crate::records::id::new();
        entry
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Totp {
    /// Base32 shared secret, as shown by the service or in an otpauth:// link.
    pub secret: String,
    #[zeroize(skip)]
    pub algorithm: TotpAlgorithm,
    #[zeroize(skip)]
    pub digits: u8,
    #[zeroize(skip)]
    pub period: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TotpAlgorithm {
    #[default]
    Sha1,
    Sha256,
    Sha512,
}

/// Inline files share the entry's encryption and conflict resolution lifecycle.
/// Keep a conservative budget below the cloud transaction limit (including base64).
pub const MAX_ATTACHMENT_BYTES: usize = 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 16;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Attachment {
    pub id: String,
    /// A display name only, never interpreted as a path.
    pub name: String,
    #[serde(with = "file_bytes")]
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attachment")
            .field("id", &self.id)
            .field("bytes", &self.data.len())
            .finish_non_exhaustive()
    }
}

impl Entry {
    pub fn validate_attachments(&self) -> Result<(), &'static str> {
        let mut ids = std::collections::BTreeSet::new();

        if self.attachments.len() > MAX_ATTACHMENTS {
            return Err("At most 16 attachments are allowed per entry");
        }

        let mut total = 0usize;

        for file in &self.attachments {
            if !crate::records::id::is_valid(&file.id)
                || !ids.insert(&file.id)
                || file.name.trim().is_empty()
                || file.name.len() > 255
                || file
                    .name
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\'))
                || matches!(file.name.as_str(), "." | "..")
            {
                return Err("Invalid attachment name or identifier");
            }
            total = total.saturating_add(file.data.len());
        }

        if total > MAX_ATTACHMENT_BYTES {
            return Err("Attachments must total at most 1 MiB per entry");
        }

        Ok(())
    }

    pub fn add_attachment(&mut self, name: String, data: Vec<u8>) -> Result<(), &'static str> {
        self.attachments.push(Attachment {
            id: crate::records::id::new(),
            name,
            data,
        });

        if let Err(error) = self.validate_attachments() {
            self.attachments.pop();
            return Err(error);
        }

        Ok(())
    }
}

mod file_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        let encoded = Zeroizing::new(STANDARD.encode(bytes));
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = Zeroizing::new(String::deserialize(deserializer)?);

        if encoded.len() > crate::records::entry::MAX_ATTACHMENT_BYTES.div_ceil(3) * 4 {
            return Err(D::Error::custom("attachment is too large"));
        }
        STANDARD
            .decode(encoded.as_bytes())
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
#[path = "tests/entry.rs"]
mod tests;
