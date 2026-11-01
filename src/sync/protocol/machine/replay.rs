//! Verify control history and causal changes, then derive the reference projection.

use crate::records::history::RecordChange;
use crate::sync::{
    identity::Key,
    protocol::{
        DeviceSecret, Error, Hash, Mutation, Object, Projection, Result, Role, Signed, Transaction,
        canonical,
        crypto::{self, nonce},
        decode,
        machine::{
            Machine,
            history::ControlHistory,
            membership::grant_aad,
            protocol::{Change, ChangeHeader, Control},
            state::{Replay, SyncState},
        },
        model::project,
    },
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
enum RecordHistory<'a> {
    Omit,
    All,
    Record(&'a str),
}

impl RecordHistory<'_> {
    fn requested(self) -> bool {
        !matches!(self, Self::Omit)
    }

    fn includes(self, mutation: &Mutation) -> bool {
        mutation.entity.kind == crate::sync::protocol::EntityKind::Entry
            && match self {
                Self::Omit => false,
                Self::All => true,
                Self::Record(selected) => selected == mutation.entity.id,
            }
    }
}

impl Machine {
    /// Verify pinned control history, roles, signatures, dependencies and epoch
    /// closure, then derive the same projection for every ordering of that set.
    ///
    /// This method performs no I/O and does not mutate the journal. The caller
    /// persists integrity faults and retains its previous readable snapshot while
    /// dependencies are incomplete; inspect [`Replay::ready`] before installing it.
    pub fn replay(state: &SyncState, device: &DeviceSecret) -> Result<Replay> {
        Self::replay_inner(state, device, RecordHistory::All)
    }

    /// Verify the complete history without allocating display-only record revisions.
    pub(crate) fn replay_state(state: &SyncState, device: &DeviceSecret) -> Result<Replay> {
        Self::replay_inner(state, device, RecordHistory::Omit)
    }

    /// Verify the complete history, materializing only the selected record's revisions.
    pub(crate) fn replay_record_history(
        state: &SyncState,
        device: &DeviceSecret,
        record: Option<&str>,
    ) -> Result<Replay> {
        Self::replay_inner(
            state,
            device,
            record.map_or(RecordHistory::Omit, RecordHistory::Record),
        )
    }

    fn replay_inner(
        state: &SyncState,
        device: &DeviceSecret,
        record_history: RecordHistory<'_>,
    ) -> Result<Replay> {
        if let Some(fault) = state.fault {
            return Err(fault);
        }

        if state.objects.len() > crate::sync::protocol::MAX_OBJECTS {
            return Err(Error::ResourceBudgetExceeded);
        }
        if state.candidates.len() > crate::sync::protocol::MAX_CANDIDATES
            || state.rejected_candidates.len() > crate::sync::protocol::MAX_CANDIDATES
            || state
                .candidates
                .values()
                .map(|bytes| bytes.0.len())
                .sum::<usize>()
                > crate::sync::protocol::MAX_CANDIDATE_BYTES
        {
            return Err(Error::ResourceBudgetExceeded);
        }

        let mut controls = BTreeMap::new();
        let mut changes = BTreeMap::new();

        for (hash, bytes) in state.objects.iter().chain(&state.candidates) {
            let object = Object::decode(&bytes.0, *hash)?;

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
        let device_id = device.id();
        let role = current
            .body
            .members
            .iter()
            .find(|m| m.certificate.body.device == device_id)
            .map(|m| m.role);
        let mut result = Replay {
            history: Vec::new(),
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
            rejected: state
                .rejected_candidates
                .iter()
                .chain(history.rejected.iter())
                .map(|(hash, error)| (*hash, *error))
                .collect(),
            excluded: BTreeSet::new(),
            role,
            ready: false,
            history_verified: true,
        };

        let mut previous_epoch_verified = false;

        for (index, (membership, signed)) in chain.iter().enumerate() {
            let control = &signed.body;
            let Some(grant) = control.grants.iter().find(|g| g.device == device_id) else {
                previous_epoch_verified = false;
                result.history_verified = false;
                continue;
            };

            let member = control
                .members
                .iter()
                .find(|m| m.certificate.body.device == device_id)
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

            if previous_epoch_verified && result.projection != baseline {
                return Err(Error::CheckpointMismatch);
            }

            let content_key = crypto::content_key(&key, state.space, control.epoch.context, false);
            let next = chain.get(index + 1).map(|(_, c)| &c.body);
            let mut authenticated =
                authenticate_changes(*membership, control, &content_key, &changes);
            // Dropping invalid bytes must not make their known-invalid ancestry
            // appear merely unavailable to a signed descendant.
            authenticated.rejected.extend(
                state
                    .rejected_candidates
                    .iter()
                    .map(|(hash, error)| (*hash, *error)),
            );
            let replayed = EpochContext {
                membership: *membership,
                control,
                baseline: &baseline,
                next,
            }
            .replay(authenticated, &changes, &controls, record_history)?;
            result.history.extend(replayed.history);
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
            previous_epoch_verified = true;

            if let Some(frontier) = replayed.current_frontier {
                result.frontier = frontier;
                result.ready = true;
            }
        }

        Ok(result)
    }
}

struct VerifiedChange<'a> {
    header: &'a ChangeHeader,
    transaction: Transaction,
}

struct EpochContext<'a> {
    membership: Hash,
    control: &'a Control,
    baseline: &'a Projection,
    next: Option<&'a Control>,
}

struct EpochReplay {
    history: Vec<RecordChange>,
    projection: Projection,
    /// Available only for a fully resolved current epoch.
    current_frontier: Option<BTreeSet<Hash>>,
    accepted: BTreeSet<Hash>,
    pending: BTreeSet<Hash>,
    rejected: BTreeMap<Hash, Error>,
    excluded: BTreeSet<Hash>,
}

impl EpochContext<'_> {
    fn replay(
        self,
        authenticated: AuthenticatedChanges<'_>,
        changes: &BTreeMap<Hash, Signed<Change>>,
        controls: &BTreeMap<Hash, Signed<Control>>,
        record_history: RecordHistory<'_>,
    ) -> Result<EpochReplay> {
        let Self {
            membership,
            control,
            baseline,
            next,
        } = self;
        let AuthenticatedChanges {
            mut verified,
            mut rejected,
        } = authenticated;
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
            for parent in &change.header.parents {
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
                    let past = ancestry(&accepted[head].header.parents, &accepted)?;

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
            included
                .iter()
                .map(|h| (*h, accepted[h].header.author, &accepted[h].transaction)),
        );
        let current_frontier = (next.is_none() && pending.is_empty()).then(|| {
            let mut frontier = included.clone();
            for hash in &included {
                for parent in &accepted[hash].header.parents {
                    frontier.remove(parent);
                }
            }
            frontier
        });

        let mut ordered = record_history.requested().then(Vec::new);
        // Ancestry validation is required even when display revisions are omitted.
        for hash in &included {
            let depth = ancestry(&accepted[hash].header.parents, &accepted)?.len();
            if let Some(ordered) = &mut ordered
                && accepted[hash]
                    .transaction
                    .mutations
                    .iter()
                    .any(|mutation| record_history.includes(mutation))
            {
                ordered.push((depth, hash));
            }
        }
        let history = if let Some(mut ordered) = ordered {
            ordered.sort_by_key(|(depth, hash)| (*depth, **hash));
            materialize_history(
                control,
                baseline,
                &accepted,
                ordered.into_iter().map(|(_, hash)| *hash),
                record_history,
            )
        } else {
            Vec::new()
        };
        Ok(EpochReplay {
            history,
            projection,
            current_frontier,
            accepted: included,
            pending,
            rejected,
            excluded,
        })
    }
}

fn materialize_history(
    control: &Control,
    baseline: &Projection,
    accepted: &BTreeMap<Hash, VerifiedChange<'_>>,
    ordered: impl IntoIterator<Item = Hash>,
    record_history: RecordHistory<'_>,
) -> Vec<RecordChange> {
    let mut history = Vec::new();
    let mut actors = BTreeMap::new();
    for hash in ordered {
        let change = &accepted[&hash];
        let author = change.header.author;
        let actor = actors.entry(author).or_insert_with(|| {
            control
                .members
                .iter()
                .find(|m| m.certificate.body.device == author)
                .map(|m| m.certificate.body.user.to_string())
                .unwrap_or_else(|| author.to_string())
        });
        for (index, mutation) in change.transaction.mutations.iter().enumerate() {
            if !record_history.includes(mutation) {
                continue;
            }
            let before = mutation
                .bases
                .iter()
                .filter_map(|base| {
                    let value = accepted
                        .get(&base.change)
                        .and_then(|c| c.transaction.mutations.get(base.index as usize))
                        .and_then(|m| m.value.as_ref())
                        .or_else(|| {
                            baseline
                                .records
                                .iter()
                                .flat_map(|r| &r.versions)
                                .find(|v| v.revision == *base)
                                .and_then(|v| v.value.as_ref())
                        });
                    match value {
                        Some(crate::sync::protocol::Value::Entry(e)) => Some(e.clone()),
                        _ => None,
                    }
                })
                .collect();
            history.push(RecordChange {
                id: mutation.entity.id.clone(),
                revision: format!("{hash}:{index}"),
                actor: actor.clone(),
                device: author.to_string(),
                at: None,
                action: format!("{:?}", mutation.action),
                before,
                after: match &mutation.value {
                    Some(crate::sync::protocol::Value::Entry(e)) => Some(e.clone()),
                    _ => None,
                },
            });
        }
    }
    history
}

struct AuthenticatedChanges<'a> {
    verified: BTreeMap<Hash, VerifiedChange<'a>>,
    rejected: BTreeMap<Hash, Error>,
}

/// Authenticate and decrypt only changes bound to this membership epoch.
fn authenticate_changes<'a>(
    membership: Hash,
    control: &Control,
    key: &Key,
    changes: &'a BTreeMap<Hash, Signed<Change>>,
) -> AuthenticatedChanges<'a> {
    let mut verified = BTreeMap::new();
    let mut rejected = BTreeMap::new();

    for (hash, signed) in changes {
        let header = &signed.body.header;

        if header.membership != membership {
            continue;
        }

        let check = || -> Result<Transaction> {
            header.validate(control.epoch.space, control.epoch.number)?;

            let member = control
                .members
                .iter()
                .find(|m| m.certificate.body.device == header.author)
                .ok_or(Error::UnauthorizedDevice)?;
            crypto::verify(
                signed,
                &member.certificate.body.signing_key,
                crypto::CHANGE_DOMAIN,
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

            if plaintext.len() > crate::sync::protocol::MAX_TRANSACTION_BYTES {
                return Err(Error::ResourceBudgetExceeded);
            }
            decode(&plaintext)
        };

        match check() {
            Ok(transaction) => {
                verified.insert(
                    *hash,
                    VerifiedChange {
                        header,
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
    verified: &mut BTreeMap<Hash, VerifiedChange<'_>>,
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

            if relevant.len() > crate::sync::protocol::MAX_OBJECTS {
                return Err(Error::ResourceBudgetExceeded);
            }

            match verified.get(&hash) {
                Some(change) => stack.extend(&change.header.parents),
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

fn ensure_unique_sequences(verified: &BTreeMap<Hash, VerifiedChange<'_>>) -> Result<()> {
    let mut slots = BTreeMap::new();

    for (hash, change) in verified {
        let header = change.header;

        if slots
            .insert((header.author, header.sequence), *hash)
            .is_some()
        {
            return Err(Error::DeviceFork);
        }
    }

    Ok(())
}

struct CausalChanges<'a> {
    accepted: BTreeMap<Hash, VerifiedChange<'a>>,
    unresolved: BTreeMap<Hash, VerifiedChange<'a>>,
}

/// Apply only transactions whose authenticated causal past is accepted. Invalid
/// ancestry propagates rejection; genuinely absent history remains unresolved.
fn apply_causal_changes<'a>(
    membership: Hash,
    baseline: &Projection,
    mut verified: BTreeMap<Hash, VerifiedChange<'a>>,
    rejected: &mut BTreeMap<Hash, Error>,
    changes: &BTreeMap<Hash, Signed<Change>>,
    controls: &BTreeMap<Hash, Signed<Control>>,
    closed: bool,
) -> Result<CausalChanges<'a>> {
    let mut accepted: BTreeMap<Hash, VerifiedChange<'a>> = BTreeMap::new();
    loop {
        // A present but rejected or wrong-epoch parent cannot become valid by
        // downloading more objects. Propagate rejection through its descendants
        // instead of keeping the whole space permanently "incomplete".
        let invalid: Vec<_> = verified
            .iter()
            .filter(|(_, change)| {
                change.header.parents.iter().any(|parent| {
                    rejected.contains_key(parent)
                        || controls.contains_key(parent)
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
            .filter(|(_, c)| c.header.parents.iter().all(|p| accepted.contains_key(p)))
            .map(|(h, _)| *h)
            .collect();

        if ready.is_empty() && invalid.is_empty() {
            break;
        }

        for hash in ready {
            let change = verified.remove(&hash).unwrap();
            let header = change.header;
            let check = || -> Result<()> {
                let past = ancestry(&header.parents, &accepted)?;

                for parent in &header.parents {
                    let ancestors = ancestry(&accepted[parent].header.parents, &accepted)?;

                    if header.parents.iter().any(|p| ancestors.contains(p)) {
                        return Err(Error::InvalidTransaction);
                    }
                }

                match (header.sequence.0, header.previous) {
                    (1, None) => {}
                    (sequence, Some(previous)) if sequence > 1 && past.contains(&previous) => {
                        let previous = accepted[&previous].header;

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
                    past.iter()
                        .map(|h| (*h, accepted[h].header.author, &accepted[h].transaction)),
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

fn ancestry(
    parents: &[Hash],
    changes: &BTreeMap<Hash, VerifiedChange<'_>>,
) -> Result<BTreeSet<Hash>> {
    let mut visited = BTreeSet::new();
    let mut stack = parents.to_vec();

    while let Some(hash) = stack.pop() {
        if !visited.insert(hash) {
            continue;
        }

        if visited.len() > crate::sync::protocol::MAX_OBJECTS {
            return Err(Error::ResourceBudgetExceeded);
        }

        let change = changes.get(&hash).ok_or(Error::MissingDependency)?;
        stack.extend(&change.header.parents);
    }

    Ok(visited)
}
