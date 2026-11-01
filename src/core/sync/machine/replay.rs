//! Verify control history and causal changes, then derive the reference projection.

use super::super::{
    DeviceSecret, Error, Hash, Object, Projection, Result, Role, Signed, Transaction, canonical,
    crypto::{self, nonce},
    decode,
    model::project,
    wire::sorted_unique,
};
use super::{
    Machine,
    profile::Profile,
    protocol::{Change, Control},
    state::{Replay, SyncState},
};
use super::{history::ControlHistory, membership::grant_aad};
use std::collections::{BTreeMap, BTreeSet};

impl Machine {
    /// Verify pinned control history, roles, signatures, dependencies and epoch
    /// closure, then derive the same projection for every ordering of that set.
    ///
    /// This method performs no I/O and does not mutate the journal. The caller
    /// persists integrity faults and retains its previous readable snapshot while
    /// dependencies are incomplete; inspect [`Replay::ready`] before installing it.
    pub fn replay(state: &SyncState, device: &DeviceSecret) -> Result<Replay> {
        if let Some(fault) = state.fault {
            return Err(fault);
        }

        if state.objects.len() > super::super::MAX_OBJECTS {
            return Err(Error::ResourceBudgetExceeded);
        }

        let mut controls = BTreeMap::new();
        let mut changes = BTreeMap::new();

        for (hash, bytes) in &state.objects {
            let object: Object = decode(&bytes.0)?;

            if object.hash()? != *hash {
                return Err(Error::HashMismatch);
            }

            match object {
                Object::Control(control) => {
                    controls.insert(*hash, control);
                }
                Object::Change(change) => {
                    changes.insert(*hash, change);
                }
            }
        }

        let history = ControlHistory::verify(&controls, state)?;
        let chain = &history.chain;
        let (tip, current) = history.tip();
        let role = current
            .body
            .members
            .iter()
            .find(|m| m.certificate.body.device == device.id())
            .map(|m| m.role);
        let mut result = Replay {
            membership: tip,
            epoch: current.body.epoch.number.0,
            projection: Projection::default(),
            frontier: BTreeSet::new(),
            accepted: BTreeSet::new(),
            pending: state
                .required_control
                .filter(|head| !history.contains(*head))
                .into_iter()
                .collect(),
            rejected: history.rejected.clone(),
            excluded: BTreeSet::new(),
            role,
            ready: false,
            history_verified: true,
        };

        let mut previous_projection: Option<Projection> = None;

        for (index, (membership, signed)) in chain.iter().enumerate() {
            let control = &signed.body;
            let Some(grant) = control.grants.iter().find(|g| g.device == device.id()) else {
                previous_projection = None;
                result.history_verified = false;
                continue;
            };

            let member = control
                .members
                .iter()
                .find(|m| m.certificate.body.device == device.id())
                .ok_or(Error::UnauthorizedDevice)?;
            let key = device.open_grant(
                &grant.enc,
                &grant.ciphertext,
                &grant_aad(&control.epoch, &member.certificate)?,
            )?;
            let checkpoint_key =
                crypto::content_key(&key, state.space, control.epoch.context, true);
            let nonce = nonce(&control.checkpoint_nonce)?;
            let plaintext = crypto::decrypt(
                &checkpoint_key,
                &nonce,
                &control.checkpoint,
                &canonical(&control.epoch)?,
            )?;
            let baseline: Projection = decode(&plaintext)?;
            baseline.validate()?;

            if index == 0 && !baseline.records.is_empty() {
                return Err(Error::CheckpointMismatch);
            }

            if let Some(previous) = &previous_projection
                && previous != &baseline
            {
                return Err(Error::CheckpointMismatch);
            }

            let content_key = crypto::content_key(&key, state.space, control.epoch.context, false);
            let next = chain.get(index + 1).map(|(_, c)| &c.body);
            let replayed = replay_epoch(
                *membership,
                control,
                &baseline,
                &content_key,
                &changes,
                &controls.keys().copied().collect(),
                next,
            )?;
            result.projection = replayed.projection;
            result.accepted.extend(replayed.accepted);
            result.pending.extend(replayed.pending);
            result.rejected.extend(replayed.rejected);
            result.excluded.extend(replayed.excluded);

            if !result.pending.is_empty() {
                // A known successor stops old publication even while its closed
                // history is incomplete. Retain the last readable projection.
                return Ok(result);
            }
            previous_projection = Some(result.projection.clone());

            if index + 1 == chain.len() {
                result.frontier = replayed.frontier;
                result.ready = true;
            }
        }

        Ok(result)
    }
}

struct VerifiedChange {
    signed: Signed<Change>,
    transaction: Transaction,
}

fn replay_epoch(
    membership: Hash,
    control: &Control,
    baseline: &Projection,
    key: &[u8; 32],
    changes: &BTreeMap<Hash, Signed<Change>>,
    controls: &BTreeSet<Hash>,
    next: Option<&Control>,
) -> Result<Replay> {
    let AuthenticatedChanges {
        mut verified,
        mut rejected,
    } = authenticate_changes(membership, control, key, changes);
    let ClosedHistory {
        excluded,
        missing: missing_closed,
    } = select_closed_history(&mut verified, &rejected, next)?;
    ensure_unique_sequences(&verified)?;
    let CausalChanges {
        accepted,
        unresolved: verified,
    } = apply_causal_changes(
        membership,
        baseline,
        verified,
        &mut rejected,
        changes,
        controls,
        next.is_some(),
    )?;
    let mut pending: BTreeSet<_> = verified.keys().copied().collect();
    pending.extend(missing_closed);

    for change in verified.values() {
        for parent in &change.signed.body.header.parents {
            if !accepted.contains_key(parent) && !rejected.contains_key(parent) {
                pending.insert(*parent);
            }
        }
    }

    let included = if let Some(next) = next {
        for head in &next.closed_frontier {
            if !accepted.contains_key(head) {
                pending.insert(*head);
            }
        }

        if pending.is_empty() {
            for head in &next.closed_frontier {
                let past = ancestry(&accepted[head].signed.body.header.parents, &accepted)?;

                if next
                    .closed_frontier
                    .iter()
                    .any(|other| past.contains(other))
                {
                    return Err(Error::CheckpointMismatch);
                }
            }
            ancestry(&next.closed_frontier, &accepted)?
        } else {
            accepted.keys().copied().collect()
        }
    } else {
        accepted.keys().copied().collect()
    };

    let projection = project(
        baseline,
        included.iter().map(|h| {
            (
                *h,
                accepted[h].signed.body.header.author,
                &accepted[h].transaction,
            )
        }),
    );
    let mut frontier = included.clone();

    for hash in &included {
        for parent in &accepted[hash].signed.body.header.parents {
            frontier.remove(parent);
        }
    }

    Ok(Replay {
        membership,
        epoch: control.epoch.number.0,
        projection,
        frontier,
        accepted: included,
        pending,
        rejected,
        excluded,
        role: None,
        ready: true,
        history_verified: true,
    })
}

struct AuthenticatedChanges {
    verified: BTreeMap<Hash, VerifiedChange>,
    rejected: BTreeMap<Hash, Error>,
}

/// Authenticate and decrypt only changes bound to this membership epoch.
fn authenticate_changes(
    membership: Hash,
    control: &Control,
    key: &[u8; 32],
    changes: &BTreeMap<Hash, Signed<Change>>,
) -> AuthenticatedChanges {
    let mut verified = BTreeMap::new();
    let mut rejected = BTreeMap::new();

    for (hash, signed) in changes {
        let header = &signed.body.header;

        if header.membership != membership {
            continue;
        }

        let check = || -> Result<Transaction> {
            if header.profile != control.epoch.profile {
                return Err(Error::UnsupportedProtocol);
            }

            if header.space != control.epoch.space
                || header.epoch != control.epoch.number
                || header.sequence.0 == 0
                || header.parents.len() > super::super::MAX_PARENTS
                || !sorted_unique(&header.parents)
            {
                return Err(Error::InvalidTransaction);
            }

            let member = control
                .members
                .iter()
                .find(|m| m.certificate.body.device == header.author)
                .ok_or(Error::UnauthorizedDevice)?;
            crypto::verify(
                signed,
                &member.certificate.body.signing_key,
                Profile::parse(&header.profile)?.change_domain(),
            )?;

            if member.role == Role::Reader {
                return Err(Error::InsufficientRole);
            }

            let plaintext = crypto::decrypt(
                key,
                &nonce(&header.nonce)?,
                &signed.body.ciphertext,
                &canonical(header)?,
            )?;

            if plaintext.len() > super::super::MAX_TRANSACTION_BYTES {
                return Err(Error::ResourceBudgetExceeded);
            }
            decode(&plaintext)
        };

        match check() {
            Ok(transaction) => {
                verified.insert(
                    *hash,
                    VerifiedChange {
                        signed: signed.clone(),
                        transaction,
                    },
                );
            }
            Err(error) => {
                rejected.insert(*hash, error);
            }
        }
    }
    AuthenticatedChanges { verified, rejected }
}

struct ClosedHistory {
    excluded: BTreeSet<Hash>,
    missing: BTreeSet<Hash>,
}

/// Restrict a closed epoch to the successor's signed causal closure before
/// checking sequence forks. Late excluded history cannot invalidate that closure.
fn select_closed_history(
    verified: &mut BTreeMap<Hash, VerifiedChange>,
    rejected: &BTreeMap<Hash, Error>,
    next: Option<&Control>,
) -> Result<ClosedHistory> {
    let mut excluded = BTreeSet::new();
    let mut missing_closed = BTreeSet::new();

    if let Some(next) = next {
        let mut relevant = BTreeSet::new();
        let mut stack = next.closed_frontier.clone();

        while let Some(hash) = stack.pop() {
            if !relevant.insert(hash) {
                continue;
            }

            if relevant.len() > super::super::MAX_OBJECTS {
                return Err(Error::ResourceBudgetExceeded);
            }

            match verified.get(&hash) {
                Some(change) => stack.extend(&change.signed.body.header.parents),
                None if rejected.contains_key(&hash) => return Err(Error::CheckpointMismatch),
                None => {
                    missing_closed.insert(hash);
                }
            }
        }
        excluded.extend(
            verified
                .keys()
                .filter(|hash| !relevant.contains(hash))
                .copied(),
        );
        verified.retain(|hash, _| relevant.contains(hash));
    }

    Ok(ClosedHistory {
        excluded,
        missing: missing_closed,
    })
}

fn ensure_unique_sequences(verified: &BTreeMap<Hash, VerifiedChange>) -> Result<()> {
    let mut slots = BTreeMap::new();

    for (hash, change) in verified {
        let header = &change.signed.body.header;

        if slots
            .insert((header.author, header.sequence), *hash)
            .is_some()
        {
            return Err(Error::DeviceFork);
        }
    }

    Ok(())
}

struct CausalChanges {
    accepted: BTreeMap<Hash, VerifiedChange>,
    unresolved: BTreeMap<Hash, VerifiedChange>,
}

/// Apply only transactions whose authenticated causal past is accepted. Invalid
/// ancestry propagates rejection; genuinely absent history remains unresolved.
fn apply_causal_changes(
    membership: Hash,
    baseline: &Projection,
    mut verified: BTreeMap<Hash, VerifiedChange>,
    rejected: &mut BTreeMap<Hash, Error>,
    changes: &BTreeMap<Hash, Signed<Change>>,
    controls: &BTreeSet<Hash>,
    closed: bool,
) -> Result<CausalChanges> {
    let mut accepted: BTreeMap<Hash, VerifiedChange> = BTreeMap::new();
    loop {
        // A present but rejected or wrong-epoch parent cannot become valid by
        // downloading more objects. Propagate rejection through its descendants
        // instead of keeping the whole space permanently "incomplete".
        let invalid: Vec<_> = verified
            .iter()
            .filter(|(_, change)| {
                change.signed.body.header.parents.iter().any(|parent| {
                    rejected.contains_key(parent)
                        || controls.contains(parent)
                        || changes
                            .get(parent)
                            .is_some_and(|c| c.body.header.membership != membership)
                })
            })
            .map(|(hash, _)| *hash)
            .collect();

        if !invalid.is_empty() && closed {
            return Err(Error::CheckpointMismatch);
        }

        for hash in &invalid {
            verified.remove(hash);
            rejected.insert(*hash, Error::InvalidTransaction);
        }

        let ready: Vec<_> = verified
            .iter()
            .filter(|(_, c)| {
                c.signed
                    .body
                    .header
                    .parents
                    .iter()
                    .all(|p| accepted.contains_key(p))
            })
            .map(|(h, _)| *h)
            .collect();

        if ready.is_empty() && invalid.is_empty() {
            break;
        }

        for hash in ready {
            let change = verified.remove(&hash).unwrap();
            let header = &change.signed.body.header;
            let check = || -> Result<()> {
                let past = ancestry(&header.parents, &accepted)?;

                for parent in &header.parents {
                    let ancestors =
                        ancestry(&accepted[parent].signed.body.header.parents, &accepted)?;

                    if header.parents.iter().any(|p| ancestors.contains(p)) {
                        return Err(Error::InvalidTransaction);
                    }
                }

                match (header.sequence.0, header.previous) {
                    (1, None) => {}
                    (sequence, Some(previous)) if sequence > 1 && past.contains(&previous) => {
                        let previous = &accepted[&previous].signed.body.header;

                        if previous.author != header.author
                            || previous.sequence.0.checked_add(1) != Some(sequence)
                        {
                            return Err(Error::InvalidTransaction);
                        }
                    }
                    _ => return Err(Error::InvalidTransaction),
                }

                let before = project(
                    baseline,
                    past.iter().map(|h| {
                        (
                            *h,
                            accepted[h].signed.body.header.author,
                            &accepted[h].transaction,
                        )
                    }),
                );
                change.transaction.validate(&before)
            };

            match check() {
                Ok(()) => {
                    accepted.insert(hash, change);
                }
                Err(error) => {
                    if closed {
                        // An authenticated control closes over invalid history,
                        // not merely unavailable history. Preserve fault evidence.
                        return Err(Error::CheckpointMismatch);
                    }
                    rejected.insert(hash, error);
                }
            }
        }
    }

    Ok(CausalChanges {
        accepted,
        unresolved: verified,
    })
}

fn ancestry(parents: &[Hash], changes: &BTreeMap<Hash, VerifiedChange>) -> Result<BTreeSet<Hash>> {
    let mut visited = BTreeSet::new();
    let mut stack = parents.to_vec();

    while let Some(hash) = stack.pop() {
        if !visited.insert(hash) {
            continue;
        }

        if visited.len() > super::super::MAX_OBJECTS {
            return Err(Error::ResourceBudgetExceeded);
        }

        let change = changes.get(&hash).ok_or(Error::MissingDependency)?;
        stack.extend(&change.signed.body.header.parents);
    }

    Ok(visited)
}
