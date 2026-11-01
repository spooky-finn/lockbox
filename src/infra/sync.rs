//! Bounded ciphertext transport. Providers never receive signing or content keys.

use crate::core::sync::{Blob, Error, Hash, MAX_OBJECT_BYTES, MAX_OBJECTS, SyncState};
use std::{
    collections::BTreeSet,
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
    pub objects: Vec<Hash>,
    pub next: Option<String>,
}

/// Immutable, content-addressed objects only. No timestamp, ETag or listing
/// position is interpreted as membership, history order, or deletion evidence.
pub trait Storage {
    fn list(&mut self, namespace: Hash, cursor: Option<&str>) -> Result<Page>;
    fn get(&mut self, namespace: Hash, hash: Hash) -> Result<Vec<u8>>;
    fn put(&mut self, namespace: Hash, hash: Hash, bytes: &[u8]) -> Result<()>;
}

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
    let mut controls = Vec::new();
    let mut changes = Vec::new();
    for hash in &state.outbox {
        let bytes = state
            .objects
            .get(hash)
            .ok_or(TransportError::InvalidResponse)?;
        match crate::core::sync::decode::<crate::core::sync::Object>(&bytes.0)
            .map_err(TransportError::Verification)?
        {
            crate::core::sync::Object::Control(_) => controls.push((*hash, bytes.clone())),
            crate::core::sync::Object::Change(_)
                if replay.accepted.contains(hash) && !replay.excluded.contains(hash) =>
            {
                changes.push((*hash, bytes.clone()))
            }
            _ => {}
        }
    }
    controls.extend(changes);
    let mut pending: std::collections::BTreeMap<_, _> = controls.into_iter().collect();
    let mut ordered = Vec::new();
    while !pending.is_empty() {
        let mut ready = Vec::new();
        for (hash, bytes) in &pending {
            let object: crate::core::sync::Object =
                crate::core::sync::decode(&bytes.0).map_err(TransportError::Verification)?;
            if object
                .dependencies()
                .iter()
                .all(|dependency| !pending.contains_key(dependency))
            {
                ready.push(*hash);
            }
        }
        if ready.is_empty() {
            return Err(TransportError::Verification(Error::InvalidTransaction));
        }
        for hash in ready {
            ordered.push((hash, pending.remove(&hash).unwrap()));
        }
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
