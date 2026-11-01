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
    Blob, Error, Hash, MAX_CANDIDATE_BYTES, MAX_CANDIDATES, Object, Result, Role, Signed, crypto,
    decode, wire::sorted_unique,
};

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

// Ok(false) means dependencies are still unknown, not rejected authority.
fn authenticated(
    hash: Hash,
    object: &Object,
    state: &SyncState,
    history: Option<&ControlHistory<'_>>,
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
            authenticate_change(change, &control.body)
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
    if header.space != control.epoch.space
        || header.epoch != control.epoch.number
        || header.sequence.0 == 0
        || header.parents.len() > crate::sync::protocol::MAX_PARENTS
        || !sorted_unique(&header.parents)
        || header.nonce.0.len() != 24
        || change.body.ciphertext.0.len() < 16
    {
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
        let mut verified = Vec::new();
        let mut rejected = Vec::new();
        for (hash, bytes) in &state.candidates {
            let object: Object = decode(&bytes.0)?;
            if object.hash()? != *hash {
                return Err(Error::HashMismatch);
            }
            match authenticated(*hash, &object, state, history.as_ref()) {
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
                authenticated(*hash, &object, state, history.as_ref()),
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
