//! Pairing documents use their own prefix; immutable sync object validation stays unchanged.

use super::{Storj, StorjFactory};
use crate::{
    sharing::transport::{
        CHUNK_BYTES, Document, MANIFEST_BYTES, Mailbox, MailboxFactory, chunk_hash, document_hash,
    },
    sync::{
        protocol::{Hash, canonical, decode},
        transport::{Cancellation, Page, Result, TransportError},
    },
    vault::profile::pairing::Slot,
};

pub(crate) struct Factory(StorjFactory);

impl Factory {
    pub fn new(endpoint: &str, bucket: &str, space: &str, access: &str, secret: &str) -> Self {
        Self(StorjFactory::new(endpoint, bucket, space, access, secret))
    }
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;

impl MailboxFactory for Factory {
    fn connect(&self) -> Result<Box<dyn Mailbox>> {
        Ok(Box::new(Store(Storj::with_region(
            &self.0.endpoint,
            &self.0.bucket,
            &self.0.space_name,
            &self.0.access,
            &self.0.secret,
            &self.0.region,
        )?)))
    }
}

pub(crate) struct Store(pub(super) Storj);

impl Store {
    fn prefix(&self, session: Hash) -> String {
        let space = self
            .0
            .prefix
            .strip_suffix("objects/")
            .expect("configured space prefix");
        format!("{space}pairing/{session}/")
    }
}

impl Mailbox for Store {
    fn get(&mut self, session: Hash, slot: Slot, cancel: &Cancellation) -> Result<Option<Vec<u8>>> {
        cancel.check()?;
        let prefix = self.prefix(session);
        let manifest = match self.0.get_key(format!("{prefix}{}.json", slot.key())) {
            Ok(bytes) => bytes,
            Err(TransportError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        if manifest.len() > MANIFEST_BYTES {
            return Err(TransportError::Budget);
        }
        let document: Document = decode(&manifest).map_err(TransportError::Verification)?;
        let length = usize::try_from(document.length).map_err(|_| TransportError::Budget)?;
        if length == 0
            || length > slot.budget()
            || document.chunks.len() != length.div_ceil(CHUNK_BYTES)
        {
            return Err(TransportError::Budget);
        }
        let mut bytes = Vec::with_capacity(length);
        for (index, expected) in document.chunks.iter().enumerate() {
            cancel.check()?;
            let chunk = match self.0.get_key(format!("{prefix}chunks/{expected}.json")) {
                Ok(chunk) => chunk,
                Err(TransportError::NotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            let expected_length = (length - index * CHUNK_BYTES).min(CHUNK_BYTES);
            if chunk.len() != expected_length || chunk_hash(&chunk) != *expected {
                return Err(TransportError::InvalidResponse);
            }
            bytes.extend(chunk);
        }
        if document_hash(&bytes) != document.hash {
            return Err(TransportError::InvalidResponse);
        }
        Ok(Some(bytes))
    }

    fn put(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: &[u8],
        cancel: &Cancellation,
    ) -> Result<()> {
        cancel.check()?;
        if bytes.is_empty() || bytes.len() > slot.budget() {
            return Err(TransportError::Budget);
        }
        let prefix = self.prefix(session);
        let mut chunks = Vec::new();
        for chunk in bytes.chunks(CHUNK_BYTES) {
            cancel.check()?;
            let hash = chunk_hash(chunk);
            self.0
                .put_key(format!("{prefix}chunks/{hash}.json"), chunk)?;
            chunks.push(hash);
        }
        let document = Document {
            hash: document_hash(bytes),
            length: bytes.len() as u64,
            chunks,
        };
        let manifest = canonical(&document).map_err(TransportError::Verification)?;
        if manifest.len() > MANIFEST_BYTES {
            return Err(TransportError::Budget);
        }
        cancel.check()?;
        self.0
            .put_key(format!("{prefix}{}.json", slot.key()), &manifest)
    }

    fn requests(&mut self, session: Hash, cursor: Option<&str>) -> Result<Page> {
        self.0
            .list_prefix(&format!("{}requests/", self.prefix(session)), cursor)
    }
}
