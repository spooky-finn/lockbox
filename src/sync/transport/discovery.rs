//! Resolve immutable references directly, then scan for previously unknown heads.
//! References are availability hints only; authorization remains in core replay.
//!
//! Only authenticated history (and pinned anchors) may require an object. A
//! reference from an unverified candidate or a freshly downloaded object is a
//! hint: it is fetched when available, but its absence never marks the pass
//! incomplete. Otherwise one unsigned object pointing at a hash that will never
//! exist would stop every replica from publishing.

use crate::sync::{
    protocol::{Error, Hash, MAX_OBJECT_BYTES, MAX_OBJECTS, Object, SyncState, decode},
    transport::{Cancellation, Download, MAX_BATCH_BYTES, Result, Storage, TransportError},
};
use std::collections::{BTreeSet, VecDeque};

const MAX_DOWNLOADS: usize = 128;
const MAX_FETCHES: usize = 256;

/// A ciphertext-only discovery request that can safely move to a worker.
///
/// Fetch pinned genesis and missing immutable dependencies even when LIST omits
/// them. A successful GET supplies a candidate, never cryptographic authority.
pub struct Discovery {
    namespace: Hash,
    known: BTreeSet<Hash>,
    /// Pinned anchors and dependencies of authenticated objects.
    required: BTreeSet<Hash>,
    /// Dependencies of unverified candidates: fetched, never required.
    hints: BTreeSet<Hash>,
}

impl Discovery {
    pub(super) fn new(namespace: Hash, known: BTreeSet<Hash>) -> Self {
        Self {
            namespace,
            known,
            required: BTreeSet::new(),
            hints: BTreeSet::new(),
        }
    }

    /// Derive outstanding references from the durable local ciphertext journal.
    pub fn for_state(state: &SyncState) -> Result<Self> {
        if state.objects.len() > MAX_OBJECTS {
            return Err(TransportError::Budget);
        }

        let mut request = Self::new(
            state.namespace,
            state
                .objects
                .keys()
                .chain(state.candidates.keys())
                .chain(state.rejected_candidates.keys())
                .copied()
                .collect(),
        );
        request.required.insert(state.genesis);
        request.required.extend(state.required_control);

        for bytes in state.objects.values() {
            let object: Object = decode(&bytes.0).map_err(TransportError::Verification)?;
            request.required.extend(object.dependencies());
        }
        for bytes in state.candidates.values() {
            let object: Object = decode(&bytes.0).map_err(TransportError::Verification)?;
            request.hints.extend(object.dependencies());
        }
        let known = &request.known;
        request.required.retain(|hash| !known.contains(hash));
        let required = &request.required;
        request
            .hints
            .retain(|hash| !known.contains(hash) && !required.contains(hash));
        Ok(request)
    }

    /// Each pass bounds GET attempts, object count, bytes, pages and cursor size.
    /// Saved candidates make the next pass progress without trusting list order.
    pub fn download(self, storage: &mut dyn Storage, cancel: &Cancellation) -> Result<Download> {
        let mut batch = Batch {
            request: &self,
            attempted: BTreeSet::new(),
            missing: BTreeSet::new(),
            listed: BTreeSet::new(),
            queue: self
                .required
                .iter()
                .chain(self.hints.iter())
                .copied()
                .collect(),
            objects: Vec::new(),
            rejected: Vec::new(),
            bytes: 0,
        };

        if !batch.fetch(storage, cancel)? {
            return Ok(batch.finish(true));
        }

        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        loop {
            cancel.check()?;
            let page = storage.list(self.namespace, cursor.as_deref())?;

            if page.objects.len() > 1000 {
                return Err(TransportError::InvalidResponse);
            }
            batch.listed.extend(page.objects.iter().copied());
            batch.queue.extend(page.objects);

            if !batch.fetch(storage, cancel)? {
                return Ok(batch.finish(true));
            }

            match page.next {
                Some(next) if next.len() <= 8192 && cursors.insert(next.clone()) => {
                    cursor = Some(next);
                }
                Some(_) => return Err(TransportError::InvalidResponse),
                None => return Ok(batch.finish(false)),
            }

            if cursors.len() > 64 {
                return Err(TransportError::Budget);
            }
        }
    }
}

struct Batch<'a> {
    request: &'a Discovery,
    attempted: BTreeSet<Hash>,
    missing: BTreeSet<Hash>,
    listed: BTreeSet<Hash>,
    queue: VecDeque<Hash>,
    objects: Vec<(Hash, Vec<u8>)>,
    rejected: Vec<(Hash, Error)>,
    bytes: usize,
}

impl Batch<'_> {
    /// False means bounded progress has been made and must be persisted first.
    fn fetch(&mut self, storage: &mut dyn Storage, cancel: &Cancellation) -> Result<bool> {
        while let Some(hash) = self.queue.pop_front() {
            cancel.check()?;

            if self.request.known.contains(&hash) || self.attempted.contains(&hash) {
                continue;
            }

            if self.objects.len() == MAX_DOWNLOADS || self.attempted.len() == MAX_FETCHES {
                return Ok(false);
            }

            self.attempted.insert(hash);
            let bytes = match storage.get(self.request.namespace, hash) {
                Ok(bytes) => bytes,
                Err(TransportError::NotFound) => {
                    self.missing.insert(hash);
                    continue;
                }
                Err(TransportError::Budget) => {
                    self.invalid(hash, Error::ResourceBudgetExceeded);
                    continue;
                }
                Err(error) => return Err(error),
            };

            if bytes.len() > MAX_OBJECT_BYTES {
                self.invalid(hash, Error::ResourceBudgetExceeded);
                continue;
            }

            if self.bytes + bytes.len() > MAX_BATCH_BYTES {
                return Ok(false);
            }

            let object = match Object::decode(&bytes, hash) {
                Ok(object) => object,
                Err(error) => {
                    self.invalid(hash, error);
                    continue;
                }
            };

            if self.request.known.len() + self.objects.len()
                >= MAX_OBJECTS + 2 * crate::sync::protocol::MAX_CANDIDATES
            {
                return Err(TransportError::Budget);
            }

            self.queue.extend(object.dependencies());
            self.bytes += bytes.len();
            self.objects.push((hash, bytes));
        }

        Ok(true)
    }

    /// Bytes stored under this key can never verify. A required object stays
    /// pending (the provider may still repair it); anything else is remembered
    /// as rejected so one bad upload cannot abort every later pass.
    fn invalid(&mut self, hash: Hash, error: Error) {
        if self.request.required.contains(&hash) {
            self.missing.insert(hash);
        } else {
            self.rejected.push((hash, error));
        }
    }

    fn finish(self, limited: bool) -> Download {
        let blocking = self
            .missing
            .iter()
            .any(|hash| self.request.required.contains(hash) || self.listed.contains(hash));
        Download {
            objects: self.objects,
            incomplete: limited || blocking,
            rejected: self.rejected,
        }
    }
}
