//! Unverified downloads have a separate, evictable budget. Public authentication
//! promotes exact bytes; it never replaces plaintext or causal verification.

use super::{
    Machine,
    commands::capacity,
    history::{ControlHistory, controls},
    membership::validate_control,
    protocol::{Change, Control},
    state::SyncState,
};
use crate::sync::protocol::{
    Blob, Error, Hash, LATE_BYTES_PER_AUTHOR, LATE_EDITS_PER_AUTHOR, MAX_CANDIDATE_BYTES,
    MAX_CANDIDATES, MAX_OBJECTS, Object, Result, Role, Signed, crypto, decode,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn insert(state: &mut SyncState, hash: Hash, bytes: Blob) {
    let mut size: usize = state.candidates.values().map(|bytes| bytes.0.len()).sum();
    while state.candidates.len() >= MAX_CANDIDATES
        || size.saturating_add(bytes.0.len()) > MAX_CANDIDATE_BYTES
    {
        let Some((_, removed)) = state.candidates.pop_first() else {
            return;
        };
        size = size.saturating_sub(removed.0.len());
    }
    state.candidates.insert(hash, bytes);
}

/// Durable room for late changes: signed for a closed epoch but outside the
/// successor's signed closed frontier. Replay never applies them; they only
/// back an explicit rebase. Without a quota, a removed member who still holds
/// a signing key could fill the journal with valid old-epoch signatures.
struct LateBudget {
    head: Hash,
    closure: BTreeSet<Hash>,
    used: BTreeMap<Hash, (usize, usize)>,
}

impl LateBudget {
    fn new(state: &SyncState, history: &ControlHistory<'_>) -> Result<Self> {
        let (head, _) = history.tip();
        // Walk each signed closed frontier through known parents. Both journal
        // and quarantine are searched, so arrival order cannot hide a needed
        // ancestor. Parents of late changes are never followed.
        let mut closure = BTreeSet::new();
        let mut stack: Vec<Hash> = history
            .chain
            .iter()
            .flat_map(|(_, control)| control.body.closed_frontier.iter().copied())
            .collect();
        while let Some(hash) = stack.pop() {
            if !closure.insert(hash) {
                continue;
            }
            if closure.len() > MAX_OBJECTS + MAX_CANDIDATES {
                return Err(Error::ResourceBudgetExceeded);
            }
            let bytes = state
                .objects
                .get(&hash)
                .or_else(|| state.candidates.get(&hash));
            if let Some(bytes) = bytes
                && let Object::Change(change) = decode::<Object>(&bytes.0)?
            {
                stack.extend(change.body.header.parents.iter().copied());
            }
        }
        let mut budget = Self {
            head,
            closure,
            used: BTreeMap::new(),
        };
        for (hash, bytes) in &state.objects {
            if let Object::Change(change) = decode::<Object>(&bytes.0)?
                && budget.is_late(*hash, &change)
            {
                let used = budget.used.entry(change.body.header.author).or_default();
                used.0 += 1;
                used.1 += bytes.0.len();
            }
        }
        Ok(budget)
    }

    fn is_late(&self, hash: Hash, change: &Signed<Change>) -> bool {
        change.body.header.membership != self.head && !self.closure.contains(&hash)
    }

    /// Reserve room for one authenticated late change. A refusal keeps it in
    /// evictable quarantine; it is never rejected, because a later frontier
    /// walk may still prove it part of closed history.
    fn admit(&mut self, author: Hash, size: usize) -> bool {
        let used = self.used.entry(author).or_default();
        if used.0 >= LATE_EDITS_PER_AUTHOR || used.1.saturating_add(size) > LATE_BYTES_PER_AUTHOR {
            return false;
        }
        used.0 += 1;
        used.1 += size;
        true
    }
}

// Ok(false) means dependencies are still unknown, not rejected authority.
// `late` is None only for migration, which must not drop already-durable
// objects (including the local outbox) by a quota introduced afterwards.
fn authenticated(
    hash: Hash,
    object: &Object,
    size: usize,
    state: &SyncState,
    history: Option<&ControlHistory<'_>>,
    late: Option<&mut LateBudget>,
) -> Result<bool> {
    let verified = match object {
        Object::Control(control) if hash == state.genesis => {
            validate_control(control, None, state, &[])
        }
        Object::Control(control) => {
            let previous = control
                .body
                .epoch
                .previous
                .ok_or(Error::UnauthorizedDevice)?;
            if state.rejected_candidates.contains_key(&previous) {
                return Err(Error::InvalidTransaction);
            }
            let Some(history) = history else {
                return Ok(false);
            };
            let Some(index) = history.chain.iter().position(|(hash, _)| *hash == previous) else {
                return Ok(false);
            };
            validate_control(
                control,
                Some(history.chain[index].1),
                state,
                &history.chain[..=index],
            )
        }
        Object::Change(change) => {
            let membership = change.body.header.membership;
            if state.rejected_candidates.contains_key(&membership) {
                return Err(Error::UnauthorizedDevice);
            }
            let Some(history) = history else {
                return Ok(false);
            };
            let Some((_, control)) = history.chain.iter().find(|(hash, _)| *hash == membership)
            else {
                return Ok(false);
            };
            let result = authenticate_change(change, &control.body);
            if result.is_ok()
                && let Some(late) = late
                && late.is_late(hash, change)
                && !late.admit(change.body.header.author, size)
            {
                return Ok(false);
            }
            result
        }
    };
    match verified {
        Ok(()) => Ok(true),
        Err(Error::MissingDependency) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn reject(state: &mut SyncState, hash: Hash, error: Error) {
    while state.rejected_candidates.len() >= MAX_CANDIDATES {
        state.rejected_candidates.pop_first();
    }
    state.rejected_candidates.insert(hash, error);
}

pub(super) fn authenticate_change(change: &Signed<Change>, control: &Control) -> Result<()> {
    let header = &change.body.header;
    header.validate(control.epoch.space, control.epoch.number)?;
    if header.nonce.0.len() != 24 || change.body.ciphertext.0.len() < 16 {
        return Err(Error::InvalidTransaction);
    }
    let member = control
        .members
        .iter()
        .find(|member| member.certificate.body.device == header.author)
        .ok_or(Error::UnauthorizedDevice)?;
    crypto::verify(
        change,
        &member.certificate.body.signing_key,
        crypto::CHANGE_DOMAIN,
    )?;
    if member.role == Role::Reader {
        return Err(Error::InsufficientRole);
    }
    Ok(())
}

pub(super) fn promote(state: &mut SyncState) -> Result<()> {
    loop {
        let controls = controls(state)?;
        let history = match ControlHistory::verify(&controls, state) {
            Ok(history) => Some(history),
            Err(Error::MissingDependency) => None,
            // Keep authenticated fork evidence intact for replay and the host's
            // durable freeze. Never turn a verification fault into eviction.
            Err(error) if error.is_history_fault() => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut late = history
            .as_ref()
            .map(|history| LateBudget::new(state, history))
            .transpose()?;
        let mut verified = Vec::new();
        let mut rejected = Vec::new();
        for (hash, bytes) in &state.candidates {
            let object = Object::decode(&bytes.0, *hash)?;
            match authenticated(
                *hash,
                &object,
                bytes.0.len(),
                state,
                history.as_ref(),
                late.as_mut(),
            ) {
                Ok(true) => verified.push(*hash),
                Ok(false) => {}
                Err(error) => rejected.push((*hash, error)),
            }
        }
        for (hash, error) in rejected {
            state.candidates.remove(&hash);
            reject(state, hash, error);
        }
        if verified.is_empty() {
            return Ok(());
        }
        for hash in verified {
            let bytes = &state.candidates[&hash];
            capacity(state, bytes.0.len())?;
            let bytes = state
                .candidates
                .remove(&hash)
                .expect("verified candidate retained");
            state.objects.insert(hash, bytes);
        }
    }
}

impl Machine {
    /// Migrate old mixed journals without changing any signed bytes or intent.
    /// Call after installation protection is checked and persist before signing.
    /// Already frozen histories retain all evidence, including forks.
    pub fn quarantine_unverified(state: &mut SyncState) -> Result<bool> {
        if state.fault.is_some() {
            return Ok(false);
        }
        let previous_count = state.objects.len();
        let controls = controls(state)?;
        let history = match ControlHistory::verify(&controls, state) {
            Ok(history) => Some(history),
            Err(Error::MissingDependency) => None,
            Err(error) if error.is_history_fault() => return Ok(false),
            Err(error) => return Err(error),
        };
        let mut unverified = Vec::new();
        for (hash, bytes) in &state.objects {
            let object: Object = decode(&bytes.0)?;
            if !matches!(
                authenticated(*hash, &object, bytes.0.len(), state, history.as_ref(), None),
                Ok(true)
            ) {
                if state.outbox.contains(hash) {
                    return Err(Error::InvalidSignature);
                }
                unverified.push(*hash);
            }
        }
        let changed = !unverified.is_empty();
        for hash in unverified {
            let bytes = state
                .objects
                .remove(&hash)
                .expect("unverified object retained");
            insert(state, hash, bytes);
        }
        promote(state)?;
        Ok(changed || state.objects.len() != previous_count)
    }
}
