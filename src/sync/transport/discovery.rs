//! Resolve immutable references directly, then scan for previously unknown heads.
//! References are availability hints only; authorization remains in core replay.
//!
//! Only authenticated history (and pinned anchors) may require an object. A
//! reference from an unverified candidate or a freshly downloaded object is a
//! hint: it is fetched when available, but its absence never marks the pass
//! incomplete. Otherwise one unsigned object pointing at a hash that will never
//! exist would stop every replica from publishing.
//!
//! Hints are fetched only after LIST, under their own small budget. Anyone with
//! bucket write can plant candidates that name hashes which will never exist;
//! they must never use up the budget that finds other members' new heads.

use crate::sync::{
    protocol::{Error, Hash, MAX_OBJECT_BYTES, MAX_OBJECTS, Object, SyncState, decode},
    transport::{Cancellation, Download, MAX_BATCH_BYTES, Result, Storage, TransportError},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

const MAX_DOWNLOADS: usize = 128;
const MAX_FETCHES: usize = 256;
/// GET attempts for hints, separate from [`MAX_FETCHES`] and spent after LIST.
const MAX_HINT_FETCHES: usize = 64;
/// LIST pages per pass. A longer listing resumes on the next pass.
const MAX_PAGES: usize = 64;
/// Wall-clock limit per pass. Generous, so a slow link still finishes a batch;
/// a pass that hits it keeps its progress like any other budget.
const PASS_TIME_LIMIT: Duration = Duration::from_secs(10 * 60);

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
    /// Keys recently seen holding junk. Skipped without a GET unless required.
    skipped: BTreeSet<Hash>,
    /// Where the previous pass stopped listing; `None` starts a new sweep.
    resume: Option<String>,
    /// Counted from the start of [`Discovery::download`].
    time_limit: Duration,
}

impl Discovery {
    pub(super) fn new(namespace: Hash, known: BTreeSet<Hash>) -> Self {
        Self {
            namespace,
            known,
            required: BTreeSet::new(),
            hints: BTreeSet::new(),
            skipped: BTreeSet::new(),
            resume: None,
            time_limit: PASS_TIME_LIMIT,
        }
    }

    #[cfg(test)]
    pub(crate) fn within(mut self, limit: Duration) -> Self {
        self.time_limit = limit;
        self
    }

    /// Continue listing at [`Download::resume`] of the previous pass. Every
    /// page before it was fully processed by the passes since the sweep began.
    pub fn resuming(mut self, cursor: Option<String>) -> Self {
        self.resume = cursor;
        self
    }

    /// Skip keys that recently held junk. Required objects are always fetched.
    pub fn skipping(mut self, keys: impl IntoIterator<Item = Hash>) -> Self {
        self.skipped.extend(keys);
        self
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

    /// Each pass bounds GET attempts, object count, bytes, pages, cursor size
    /// and time. Saved candidates make the next pass progress without trusting
    /// list order.
    pub fn download(self, storage: &mut dyn Storage, cancel: &Cancellation) -> Result<Download> {
        let mut batch = Batch {
            deadline: Instant::now().checked_add(self.time_limit),
            request: &self,
            attempted: BTreeSet::new(),
            missing: BTreeSet::new(),
            listed: BTreeSet::new(),
            queue: self.required.iter().copied().collect(),
            pinned: self.required.clone(),
            deferred: self.hints.iter().copied().collect(),
            fetches: 0,
            hint_fetches: 0,
            objects: Vec::new(),
            rejected: Vec::new(),
            bad_copies: Vec::new(),
            bytes: 0,
            resume: self.resume.clone(),
        };

        if !batch.fetch(storage, cancel)? {
            return Ok(batch.finish(true));
        }

        let mut cursor = self.resume.clone();
        let mut cursors: BTreeSet<_> = cursor.iter().cloned().collect();
        for pages in 1.. {
            cancel.check()?;
            // Every page before this cursor has been fully processed.
            if batch.expired() {
                batch.resume = cursor;
                return Ok(batch.finish(true));
            }
            // A body too large even for small pages (see the Storj adapter)
            // ends the pass with what came before it. The next sweep starts
            // over, so heads listed before it keep arriving.
            let page = match storage.list(self.namespace, cursor.as_deref()) {
                Ok(page) => page,
                Err(TransportError::Budget) => {
                    batch.resume = None;
                    return Ok(batch.finish(true));
                }
                Err(error) => return Err(error),
            };

            if page.objects.len() > 1000 {
                return Err(TransportError::InvalidResponse);
            }
            batch.listed.extend(page.objects.iter().copied());
            batch.queue.extend(unpredictable(page.objects));

            // Re-list this page next pass: its remaining keys are still queued.
            batch.resume = cursor.clone();
            if !batch.fetch(storage, cancel)? {
                return Ok(batch.finish(true));
            }

            match page.next {
                Some(next) if next.len() <= 8192 && cursors.insert(next.clone()) => {
                    cursor = Some(next);
                }
                Some(_) => return Err(TransportError::InvalidResponse),
                None => break,
            }

            // Junk keys may make the listing longer than one pass may read.
            // Keep the progress and continue there next pass instead of failing.
            if pages == MAX_PAGES {
                batch.resume = cursor;
                return Ok(batch.finish(true));
            }
        }
        batch.resume = None;

        // Hints never mark the pass incomplete, even when their budget runs out.
        batch.fetch_hints(storage, cancel)?;
        Ok(batch.finish(false))
    }
}

/// Listed keys in an order no bucket writer can predict. Junk keys listed
/// first would otherwise use up every pass's budget before honest heads.
/// Required objects are queued before any listed key and keep their place.
fn unpredictable(mut keys: Vec<Hash>) -> Vec<Hash> {
    let mut seed = [0; 32];
    // Listing order still makes bounded progress if the OS cannot help.
    if getrandom::fill(&mut seed).is_ok() {
        keys.sort_by_cached_key(|key| {
            Hash::digest(b"lockbox.sync.fetch-order\0", &[seed, key.0].concat())
        });
    }
    keys
}

enum Fetched {
    Nothing,
    Object(Vec<Hash>),
    /// The batch byte budget is full; persist before downloading more.
    Full,
}

struct Batch<'a> {
    request: &'a Discovery,
    attempted: BTreeSet<Hash>,
    missing: BTreeSet<Hash>,
    listed: BTreeSet<Hash>,
    /// Required objects, their ancestry, then listed keys.
    queue: VecDeque<Hash>,
    /// Required objects and their ancestry, which keep the main budget.
    pinned: BTreeSet<Hash>,
    /// Hints and dependencies of unverified downloads, fetched after LIST.
    deferred: VecDeque<Hash>,
    fetches: usize,
    hint_fetches: usize,
    objects: Vec<(Hash, Vec<u8>)>,
    rejected: Vec<(Hash, Error)>,
    bad_copies: Vec<Hash>,
    bytes: usize,
    resume: Option<String>,
    /// `None` only if the limit overflows the clock: no deadline then.
    deadline: Option<Instant>,
}

impl Batch<'_> {
    /// False means bounded progress has been made and must be persisted first.
    fn fetch(&mut self, storage: &mut dyn Storage, cancel: &Cancellation) -> Result<bool> {
        while let Some(hash) = self.queue.pop_front() {
            cancel.check()?;

            if self.skips(&hash) {
                continue;
            }

            if self.objects.len() == MAX_DOWNLOADS || self.fetches == MAX_FETCHES || self.expired()
            {
                return Ok(false);
            }

            self.fetches += 1;
            match self.get(storage, hash)? {
                Fetched::Nothing => {}
                Fetched::Full => return Ok(false),
                Fetched::Object(dependencies) if self.pinned.contains(&hash) => {
                    for dependency in dependencies {
                        self.pinned.insert(dependency);
                        self.queue.push_back(dependency);
                    }
                }
                Fetched::Object(dependencies) => self.deferred.extend(dependencies),
            }
        }

        Ok(true)
    }

    /// Spend the separate hint budget; running out is never an incomplete pass.
    fn fetch_hints(&mut self, storage: &mut dyn Storage, cancel: &Cancellation) -> Result<()> {
        while let Some(hash) = self.deferred.pop_front() {
            cancel.check()?;

            if self.skips(&hash) {
                continue;
            }

            if self.objects.len() == MAX_DOWNLOADS
                || self.hint_fetches == MAX_HINT_FETCHES
                || self.expired()
            {
                return Ok(());
            }

            self.hint_fetches += 1;
            match self.get(storage, hash)? {
                Fetched::Nothing => {}
                Fetched::Full => return Ok(()),
                Fetched::Object(dependencies) => self.deferred.extend(dependencies),
            }
        }

        Ok(())
    }

    /// Checked between requests only; one in flight keeps its own timeout.
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn skips(&self, hash: &Hash) -> bool {
        self.request.known.contains(hash)
            || self.attempted.contains(hash)
            || (self.request.skipped.contains(hash) && !self.request.required.contains(hash))
    }

    fn get(&mut self, storage: &mut dyn Storage, hash: Hash) -> Result<Fetched> {
        self.attempted.insert(hash);
        let bytes = match storage.get(self.request.namespace, hash) {
            Ok(bytes) => bytes,
            Err(TransportError::NotFound) => {
                self.missing.insert(hash);
                return Ok(Fetched::Nothing);
            }
            // An oversized body, or a response the gateway sends for this key
            // alone (unreadable header, odd 4xx): what a bucket writer stored
            // under it, not a provider fault, so it must not abort the pass.
            // Unavailable, AccessDenied and Cancelled concern the provider
            // as a whole and still do.
            Err(TransportError::Budget | TransportError::InvalidResponse) => {
                self.bad_copy(hash);
                return Ok(Fetched::Nothing);
            }
            Err(error) => return Err(error),
        };

        // Bytes that do not hash to their key are a bad copy, not evidence
        // against the hash: anyone with bucket write can store junk there.
        if bytes.len() > MAX_OBJECT_BYTES || Object::hash_bytes(&bytes) != hash {
            self.bad_copy(hash);
            return Ok(Fetched::Nothing);
        }

        if self.bytes + bytes.len() > MAX_BATCH_BYTES {
            return Ok(Fetched::Full);
        }

        let object = match Object::decode(&bytes, hash) {
            Ok(object) => object,
            Err(error) => {
                self.invalid(hash, error);
                return Ok(Fetched::Nothing);
            }
        };

        if self.request.known.len() + self.objects.len()
            >= MAX_OBJECTS + 2 * crate::sync::protocol::MAX_CANDIDATES
        {
            return Err(TransportError::Budget);
        }

        let dependencies = object.dependencies().collect();
        self.bytes += bytes.len();
        self.objects.push((hash, bytes));
        Ok(Fetched::Object(dependencies))
    }

    /// The bucket holds something else under this key. A required object stays
    /// pending; anything else is skipped this pass, neither rejected (honest
    /// bytes may come back) nor blocking (junk must not stall publication).
    fn bad_copy(&mut self, hash: Hash) {
        if self.request.required.contains(&hash) {
            self.missing.insert(hash);
        } else {
            self.bad_copies.push(hash);
        }
    }

    /// These exact bytes hash to the key yet can never verify, so the hash
    /// itself is invalid. A required object stays pending; anything else is
    /// remembered as rejected so one bad upload cannot abort every later pass.
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
            bad_copies: self.bad_copies,
            resume: self.resume,
        }
    }
}

/// In-memory back-off for keys whose bucket bytes were junk, so one bad upload
/// is not re-downloaded on every pass. Never persisted: it describes the bucket
/// right now, not the hash. Honest bytes restored under a key are picked up
/// within at most [`BadCopies::MAX_WAIT`] passes.
///
/// Rejected hashes wait too: the persisted rejection cache keeps only
/// `MAX_CANDIDATES` of them, fewer than a listing page may hold, so a flood
/// would otherwise be fetched again on every pass and never let it finish.
#[derive(Default)]
pub struct BadCopies {
    pass: u64,
    /// Key -> (consecutive bad copies, first pass allowed to fetch it again).
    keys: BTreeMap<Hash, (u32, u64)>,
}

impl BadCopies {
    pub const MAX_WAIT: u64 = 64;
    const LIMIT: usize = 1024;

    /// Keys the next pass should skip.
    pub fn skipped(&self) -> impl Iterator<Item = Hash> + '_ {
        self.keys
            .iter()
            .filter(|(_, (_, retry))| *retry > self.pass)
            .map(|(hash, _)| *hash)
    }

    /// Learn from a finished pass: received keys are cleared, junk keys wait
    /// 2, 4, 8 … passes, capped at [`BadCopies::MAX_WAIT`].
    pub fn record(&mut self, download: &Download) {
        self.pass += 1;
        for (hash, _) in &download.objects {
            self.keys.remove(hash);
        }
        let rejected = download.rejected.iter().map(|(hash, _)| hash);
        for hash in download.bad_copies.iter().chain(rejected) {
            let strikes = self.keys.get(hash).map_or(0, |(strikes, _)| *strikes) + 1;
            let wait = 1u64
                .checked_shl(strikes)
                .unwrap_or(u64::MAX)
                .min(Self::MAX_WAIT);
            self.keys.insert(*hash, (strikes, self.pass + wait));
        }
        // Bounded: forget the keys due soonest; at worst they are fetched again.
        while self.keys.len() > Self::LIMIT {
            let Some(soonest) = self
                .keys
                .iter()
                .min_by_key(|(_, (_, retry))| *retry)
                .map(|(hash, _)| *hash)
            else {
                break;
            };
            self.keys.remove(&soonest);
        }
    }
}
