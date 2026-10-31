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
}

impl Entry {
    /// A new empty entry with a random identifier.
    pub fn new() -> Self {
        let mut entry = Self::default();
        entry.id = crate::core::id::new();
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
