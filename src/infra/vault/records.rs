//! Read-only access to another vault file, used to import its records.

use super::{
    Result, Vault, VaultError,
    format::{Header, decrypt},
};
use crate::core::{entry::Entry, folder::Folder};
use secrecy::SecretString;
use std::{fs, path::Path};

/// Readable records of a vault. Shared-profile keys and credentials are not kept.
#[derive(Debug, Default)]
pub struct Records {
    pub entries: Vec<Entry>,
    pub folders: Vec<Folder>,
}

impl Vault {
    /// Decrypts any supported vault or profile version without taking its
    /// process lock, recovering sync intents or writing to disk, so the file
    /// may stay open in another window. For a shared profile this is its local view.
    pub fn read_records(path: impl AsRef<Path>, password: &SecretString) -> Result<Records> {
        let data = fs::read(path)?;
        let header = Header::parse(&data)?;
        let key = header.password_key(password)?;

        let contents = decrypt(&data, &key).ok_or(VaultError::Corrupted)?;
        Ok(Records {
            entries: contents.entries,
            folders: contents.folders,
        })
    }
}
