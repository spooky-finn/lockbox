//! Bounded ciphertext transport. Providers never receive signing or content keys.

use crate::core::sync::{Blob, Error, Hash, MAX_OBJECT_BYTES, MAX_OBJECTS, SyncState};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const MAX_BATCH_BYTES: usize = 16 * 1024 * 1024;
pub use crate::core::sync::MAX_CACHE_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("provider unavailable")]
    Unavailable,
    #[error("provider access denied")]
    AccessDenied,
    #[error("object not yet available")]
    NotFound,
    #[error("invalid provider response")]
    InvalidResponse,
    #[error("synchronization budget exceeded")]
    Budget,
    #[error("synchronization cancelled")]
    Cancelled,
    #[error("history verification failed: {0}")]
    Verification(Error),
}

pub type Result<T> = std::result::Result<T, TransportError>;

pub struct Page {
    /// Complete immutable-object hashes, in any provider order.
    pub objects: Vec<Hash>,
    /// Opaque continuation token; callers detect repetition and never sort by it.
    pub next: Option<String>,
}

/// Immutable, content-addressed objects only. No timestamp, ETag or listing
/// position is interpreted as membership, history order, or deletion evidence.
pub trait Storage {
    /// Enumerate only objects in the given namespace; omission is not deletion.
    /// Return at most 1,000 hashes per page and keep `cursor` opaque.
    fn list(&mut self, namespace: Hash, cursor: Option<&str>) -> Result<Page>;
    /// Fetch bounded bytes without interpreting signatures or membership.
    /// `NotFound` means incomplete availability, not revoked authority.
    fn get(&mut self, namespace: Hash, hash: Hash) -> Result<Vec<u8>>;
    /// Idempotently publish the exact bytes under their complete-object hash.
    /// A successful acknowledgement does not imply receipt by other members.
    fn put(&mut self, namespace: Hash, hash: Hash, bytes: &[u8]) -> Result<()>;
}

/// Creates a transport on the calling worker thread.
///
/// Factories contain provider configuration only. Implementations must never
/// capture a vault, signing identity, content key, or decrypted projection.
/// Keeping construction separate lets the application use any [`Storage`]
/// implementation without depending on a particular provider SDK.
pub trait StorageFactory: Send + Sync {
    /// Establish a blocking provider adapter. Do not perform protocol validation.
    fn connect(&self) -> Result<Box<dyn Storage + Send>>;
}

pub(crate) mod worker;

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            Err(TransportError::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub struct Download {
    pub objects: Vec<(Hash, Vec<u8>)>,
    pub incomplete: bool,
}

/// Full bounded scan, deliberately independent of Storj's listing order. Known
/// objects remain in the local cache when omitted by a provider. New verified
/// objects are persisted between passes, allowing bounded scans to make progress.
pub fn download(
    storage: &mut dyn Storage,
    namespace: Hash,
    known: &BTreeSet<Hash>,
    cancel: &Cancellation,
) -> Result<Download> {
    let mut cursor = None;
    let mut cursors = BTreeSet::new();
    let mut discovered = BTreeSet::new();
    let mut objects = Vec::new();
    let mut bytes_used = 0;
    let mut incomplete = false;
    loop {
        cancel.check()?;
        let page = storage.list(namespace, cursor.as_deref())?;
        if page.objects.len() > 1000 {
            return Err(TransportError::InvalidResponse);
        }
        for hash in page.objects {
            cancel.check()?;
            if known.contains(&hash) || !discovered.insert(hash) {
                continue;
            }
            if known.len() + discovered.len() > MAX_OBJECTS {
                return Err(TransportError::Budget);
            }
            let bytes = match storage.get(namespace, hash) {
                Ok(bytes) => bytes,
                Err(TransportError::NotFound) => {
                    incomplete = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if bytes.len() > MAX_OBJECT_BYTES {
                return Err(TransportError::Budget);
            }
            bytes_used += bytes.len();
            if bytes_used > MAX_BATCH_BYTES {
                return Ok(Download {
                    objects,
                    incomplete: true,
                });
            }
            let object = crate::core::sync::decode::<crate::core::sync::Object>(&bytes)
                .map_err(TransportError::Verification)?;
            if object.hash().map_err(TransportError::Verification)? != hash {
                return Err(TransportError::Verification(Error::HashMismatch));
            }
            objects.push((hash, bytes));
            if objects.len() == 128 {
                return Ok(Download {
                    objects,
                    incomplete: true,
                });
            }
        }
        match page.next {
            Some(next) if next.len() <= 8192 && cursors.insert(next.clone()) => cursor = Some(next),
            Some(_) => return Err(TransportError::InvalidResponse),
            None => {
                return Ok(Download {
                    objects,
                    incomplete,
                });
            }
        }
        if cursors.len() > 64 {
            return Err(TransportError::Budget);
        }
    }
}

/// Caller builds this plan only after verified replay and a durable local save.
/// Changes from closed epochs stay in the local log for explicit rebase.
pub fn publication(
    state: &SyncState,
    replay: &crate::core::sync::Replay,
) -> Result<Vec<(Hash, Blob)>> {
    if state.fault.is_some() || !replay.ready || replay.role.is_none() {
        return Ok(vec![]);
    }
    // Decode each eligible object once, then order only the dependency edges.
    // Independent objects may have any order; hashes never choose a data winner.
    let mut pending = BTreeMap::new();
    for hash in &state.outbox {
        let bytes = state
            .objects
            .get(hash)
            .ok_or(TransportError::InvalidResponse)?;
        let object = crate::core::sync::decode::<crate::core::sync::Object>(&bytes.0)
            .map_err(TransportError::Verification)?;
        if matches!(object, crate::core::sync::Object::Control(_))
            || (replay.accepted.contains(hash) && !replay.excluded.contains(hash))
        {
            pending.insert(*hash, (bytes.clone(), object.dependencies()));
        }
    }
    let mut incoming = BTreeMap::new();
    let mut children: BTreeMap<Hash, Vec<Hash>> = BTreeMap::new();
    let mut ready = BTreeSet::new();
    for (hash, (_, dependencies)) in &pending {
        let mut count = 0;
        for dependency in dependencies
            .iter()
            .filter(|dependency| pending.contains_key(dependency))
        {
            count += 1;
            children.entry(*dependency).or_default().push(*hash);
        }
        incoming.insert(*hash, count);
        if count == 0 {
            ready.insert(*hash);
        }
    }
    let mut ordered = Vec::new();
    while let Some(hash) = ready.pop_first() {
        ordered.push((hash, pending.remove(&hash).unwrap().0));
        for child in children.remove(&hash).unwrap_or_default() {
            let count = incoming.get_mut(&child).unwrap();
            *count -= 1;
            if *count == 0 {
                ready.insert(child);
            }
        }
    }
    if !pending.is_empty() {
        return Err(TransportError::Verification(Error::InvalidTransaction));
    }
    Ok(ordered)
}

pub fn upload(
    storage: &mut dyn Storage,
    namespace: Hash,
    plan: &[(Hash, Blob)],
    cancel: &Cancellation,
) -> Result<Vec<Hash>> {
    let mut receipts = Vec::new();
    for (hash, bytes) in plan {
        cancel.check()?;
        storage.put(namespace, *hash, &bytes.0)?;
        receipts.push(*hash);
    }
    Ok(receipts)
}

#[cfg(test)]
mod tests;
