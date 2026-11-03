//! Bounded contact mailbox. Credentials are provisioned independently; links grant no storage rights.

use crate::{
    sync::{
        protocol::{Hash, pairing::MAX_CONTACT_BYTES},
        transport::{Cancellation, Page, Result},
    },
    vault::profile::pairing::Slot,
};

pub(crate) trait Mailbox: Send {
    /// Ticket documents use their complete hash as the address; all subsequent
    /// slots use the signed ticket's random session ID under the same space prefix.
    fn get(&mut self, session: Hash, slot: Slot, cancel: &Cancellation) -> Result<Option<Vec<u8>>>;
    fn put(&mut self, session: Hash, slot: Slot, bytes: &[u8], cancel: &Cancellation)
    -> Result<()>;
    fn requests(&mut self, session: Hash, cursor: Option<&str>) -> Result<Page>;
}

pub(crate) trait MailboxFactory: Send + Sync {
    fn connect(&self) -> Result<Box<dyn Mailbox>>;
}

pub(crate) const CHUNK_BYTES: usize = 1024 * 1024;
pub(crate) const MANIFEST_BYTES: usize = MAX_CONTACT_BYTES;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    pub hash: Hash,
    pub length: u64,
    pub chunks: Vec<Hash>,
}

pub(crate) fn chunk_hash(bytes: &[u8]) -> Hash {
    Hash::digest(b"lockbox.pairing.chunk\0", bytes)
}
pub(crate) fn document_hash(bytes: &[u8]) -> Hash {
    Hash::digest(b"lockbox.pairing.document\0", bytes)
}
