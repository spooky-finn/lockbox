//! Prepare commands and complete reserved signing intents without performing I/O.

use super::super::{
    Blob, Certificate, Counter, DeviceSecret, Error, Hash, Object, Projection, Result, Role,
    Signed, Transaction, canonical,
    crypto::{self, CHANGE_DOMAIN, CONTROL_DOMAIN, nonce},
    decode,
};
use super::membership::{build_control, epoch_key, validate_control, validate_control_body};
use super::{
    Machine,
    protocol::{Change, ChangeHeader, Control, Epoch, Member, PROFILE},
    state::{ExcludedEdit, PreparedChange, Replay, SyncState},
};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

impl Machine {
    /// Create an owner-certified, signed empty genesis and its durable outbox.
    /// The host must store the returned state before publishing genesis bytes.
    pub fn create(
        space: Hash,
        namespace: Hash,
        certificate: Signed<Certificate>,
        device: &DeviceSecret,
        epoch_key: &[u8; 32],
        context: Hash,
    ) -> Result<SyncState> {
        certificate.verify()?;
        if certificate.body.device != device.id() {
            return Err(Error::UnauthorizedDevice);
        }
        let root = certificate.body.root_key;
        let owner = certificate.body.user;
        let members = vec![Member {
            certificate,
            role: Role::Owner,
        }];
        let epoch = Epoch {
            profile: PROFILE.into(),
            space,
            number: Counter(0),
            context,
            previous: None,
        };
        let control = build_control(
            epoch,
            owner,
            device.id(),
            members,
            vec![],
            &Projection::default(),
            epoch_key,
        )?;
        let object = Object::Control(crypto::sign(control, &device.signer(), CONTROL_DOMAIN)?);
        let genesis = object.hash()?;
        Ok(SyncState {
            space,
            namespace,
            genesis,
            owner_root: root,
            objects: BTreeMap::from([(genesis, Blob(object.bytes()?))]),
            outbox: BTreeSet::from([genesis]),
            intent: None,
            fault: None,
            last_exchange: None,
        })
    }

    /// Bootstrap trust must come from an independently authenticated owner root,
    /// space ID and genesis hash. Provider-supplied names never establish trust.
    pub fn join(space: Hash, namespace: Hash, genesis: Hash, owner_root: [u8; 32]) -> SyncState {
        SyncState {
            space,
            namespace,
            genesis,
            owner_root,
            objects: BTreeMap::new(),
            outbox: BTreeSet::new(),
            intent: None,
            fault: None,
            last_exchange: None,
        }
    }

    /// Idempotent insertion. Membership and plaintext are verified only in replay.
    pub fn receive(state: &mut SyncState, expected: Hash, bytes: Vec<u8>) -> Result<bool> {
        let object: Object = decode(&bytes)?;
        if object.hash()? != expected {
            return Err(Error::HashMismatch);
        }
        if let Some(existing) = state.objects.get(&expected) {
            if existing.0 != bytes {
                return Err(Error::HashMismatch);
            }
            return Ok(false);
        }
        capacity(state, bytes.len())?;
        state.objects.insert(expected, Blob(bytes));
        Ok(true)
    }

    /// Validate an atomic transaction against the current verified causal view,
    /// then encrypt its exact body without signing it.
    ///
    /// Save the returned body as [`SyncState::intent`] before completing it.
    /// Existing intents, missing history, insufficient roles and resource limits
    /// fail before a new signature can be emitted.
    pub fn prepare(
        state: &SyncState,
        device: &DeviceSecret,
        transaction: Transaction,
    ) -> Result<PreparedChange> {
        if state.intent.is_some() {
            return Err(Error::PendingIntent);
        }
        let replay = Self::replay(state, device)?;
        require_writer(&replay)?;
        transaction.validate(&replay.projection)?;
        let plaintext = Zeroizing::new(canonical(&transaction)?);
        if plaintext.len() > super::super::MAX_TRANSACTION_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        if replay.frontier.len() > super::super::MAX_PARENTS {
            return Err(Error::ResourceBudgetExceeded);
        }
        let control = control_at(state, replay.membership)?;
        let key = epoch_key(&control.body, device)?;
        let content_key = crypto::content_key(&key, state.space, control.body.epoch.context, false);
        let mut previous = None;
        let mut sequence: u64 = 0;
        for hash in &replay.accepted {
            let Object::Change(change) = decode::<Object>(&state.objects[hash].0)? else {
                continue;
            };
            if change.body.header.membership == replay.membership
                && change.body.header.author == device.id()
                && change.body.header.sequence.0 > sequence
            {
                previous = Some(*hash);
                sequence = change.body.header.sequence.0;
            }
        }
        let mut nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(|_| Error::Random)?;
        let header = ChangeHeader {
            profile: PROFILE.into(),
            space: state.space,
            membership: replay.membership,
            epoch: Counter(replay.epoch),
            author: device.id(),
            sequence: Counter(
                sequence
                    .checked_add(1)
                    .ok_or(Error::ResourceBudgetExceeded)?,
            ),
            previous,
            parents: replay.frontier.into_iter().collect(),
            nonce: Blob(nonce.to_vec()),
        };
        let ciphertext = crypto::encrypt(&content_key, &nonce, &plaintext, &canonical(&header)?)?;
        let change = Change { header, ciphertext };
        capacity(
            state,
            Object::Change(Signed {
                body: change.clone(),
                signature: Blob(vec![0; 64]),
            })
            .bytes()?
            .len(),
        )?;
        Ok(PreparedChange::Change(change))
    }

    /// Owner explicitly supplies the complete next roster, including its own
    /// certificate. Admission never follows possession of provider credentials.
    /// Ownership/control transfer are intentionally unsupported in this profile.
    pub fn prepare_membership(
        state: &SyncState,
        device: &DeviceSecret,
        members: Vec<(Signed<Certificate>, Role)>,
    ) -> Result<PreparedChange> {
        if state.intent.is_some() {
            return Err(Error::PendingIntent);
        }
        let replay = Self::replay(state, device)?;
        require_writer(&replay)?;
        let previous = control_at(state, replay.membership)?;
        if previous.body.controller != device.id() {
            return Err(Error::InsufficientRole);
        }
        let mut members: Vec<_> = members
            .into_iter()
            .map(|(certificate, role)| Member { certificate, role })
            .collect();
        members.sort_by_key(|m| m.certificate.body.device);
        let epoch = Epoch {
            profile: PROFILE.into(),
            space: state.space,
            number: Counter(
                replay
                    .epoch
                    .checked_add(1)
                    .ok_or(Error::ResourceBudgetExceeded)?,
            ),
            context: Hash(*crypto::random32()?),
            previous: Some(replay.membership),
        };
        let control = build_control(
            epoch,
            previous.body.owner,
            device.id(),
            members,
            replay.frontier.into_iter().collect(),
            &replay.projection,
            &*crypto::random32()?,
        )?;
        // Validate shape and authority without producing a signature before the
        // caller has durably reserved this successor slot.
        validate_control_body(&control, Some(&previous), state)?;
        capacity(
            state,
            Object::Control(Signed {
                body: control.clone(),
                signature: Blob(vec![0; 64]),
            })
            .bytes()?
            .len(),
        )?;
        Ok(PreparedChange::Control(control))
    }

    /// Call only after state.intent has been persisted. Ed25519 deterministically
    /// reproduces the exact signature after a crash or a lost save response.
    pub fn complete_intent(state: &mut SyncState, device: &DeviceSecret) -> Result<Hash> {
        let prepared = state.intent.clone().ok_or(Error::PendingIntent)?;
        let replay = Self::replay(state, device)?;
        require_writer(&replay)?;
        let object = match prepared {
            PreparedChange::Change(change) => {
                if change.header.membership != replay.membership
                    || change.header.author != device.id()
                {
                    return Err(Error::UnauthorizedDevice);
                }
                Object::Change(crypto::sign(change, &device.signer(), CHANGE_DOMAIN)?)
            }
            PreparedChange::Control(control) => {
                if control.epoch.previous != Some(replay.membership) {
                    return Err(Error::UnauthorizedDevice);
                }
                let previous = control_at(state, replay.membership)?;
                if previous.body.controller != device.id() {
                    return Err(Error::InsufficientRole);
                }
                let signed = crypto::sign(control, &device.signer(), CONTROL_DOMAIN)?;
                validate_control(&signed, Some(&previous), state)?;
                Object::Control(signed)
            }
        };
        let hash = object.hash()?;
        Self::receive(state, hash, object.bytes()?)?;
        let checked = Self::replay(state, device)?;
        if checked.rejected.contains_key(&hash) || checked.pending.contains(&hash) {
            return Err(Error::InvalidTransaction);
        }
        state.outbox.insert(hash);
        state.intent = None;
        Ok(hash)
    }

    /// Return certificates and roles from the last verified control record.
    /// Names, provider permissions, and merely self-certified devices do not join
    /// the roster through this query.
    pub fn members(
        state: &SyncState,
        device: &DeviceSecret,
    ) -> Result<Vec<(Signed<Certificate>, Role)>> {
        let replay = Self::replay(state, device)?;
        Ok(control_at(state, replay.membership)?
            .body
            .members
            .into_iter()
            .map(|m| (m.certificate, m.role))
            .collect())
    }

    /// Excluded ciphertext stays locally available as a draft. Its old signature
    /// does not grant current write authority. A caller must explicitly review
    /// before re-authoring it against the current visible revisions.
    pub fn excluded_edits(state: &SyncState, device: &DeviceSecret) -> Result<Vec<ExcludedEdit>> {
        let replay = Self::replay(state, device)?;
        let mut edits = Vec::new();
        for hash in replay.excluded {
            let Object::Change(signed) = decode::<Object>(&state.objects[&hash].0)? else {
                continue;
            };
            let header = &signed.body.header;
            let control = control_at(state, header.membership)?;
            let epoch_key = epoch_key(&control.body, device)?;
            let key =
                crypto::content_key(&epoch_key, state.space, control.body.epoch.context, false);
            let plaintext = crypto::decrypt(
                &key,
                &nonce(&header.nonce)?,
                &signed.body.ciphertext,
                &canonical(header)?,
            )?;
            let transaction = decode(&plaintext)?;
            edits.push(ExcludedEdit {
                hash,
                author: header.author,
                transaction,
            });
        }
        Ok(edits)
    }

    /// Explicitly re-author a reviewed excluded edit in the current epoch.
    /// Current visible revisions become its bases; old plaintext authorship is
    /// preserved only as advisory provenance, never as the new event's signature.
    pub fn prepare_rebase(
        state: &SyncState,
        device: &DeviceSecret,
        excluded: Hash,
    ) -> Result<PreparedChange> {
        use super::super::{Action, Mutation};
        let replay = Self::replay(state, device)?;
        require_writer(&replay)?;
        let edit = Self::excluded_edits(state, device)?
            .into_iter()
            .find(|edit| edit.hash == excluded)
            .ok_or(Error::InvalidTransaction)?;
        let mut mutations = Vec::new();
        for old in edit.transaction.mutations {
            let bases = replay.projection.heads(&old.entity);
            let live = replay
                .projection
                .records
                .iter()
                .find(|r| r.entity == old.entity)
                .is_some_and(|r| r.versions.len() == 1 && r.versions[0].value.is_some());
            let action = if bases.len() > 1 {
                Action::Resolve
            } else {
                match (bases.is_empty(), live, old.value.is_some()) {
                    (true, _, true) | (false, true, true) => Action::Put,
                    (false, false, true) => Action::Restore,
                    (false, true, false) => Action::Delete,
                    _ => continue,
                }
            };
            mutations.push(Mutation {
                entity: old.entity,
                bases,
                action,
                value: old.value,
            });
        }
        Self::prepare(
            state,
            device,
            Transaction {
                mutations,
                rebased_from: Some(excluded),
            },
        )
    }
}

fn capacity(state: &SyncState, bytes: usize) -> Result<()> {
    if bytes > super::super::MAX_OBJECT_BYTES
        || state.objects.len() >= super::super::MAX_OBJECTS
        || state
            .objects
            .values()
            .map(|blob| blob.0.len())
            .sum::<usize>()
            .saturating_add(bytes)
            > super::super::MAX_CACHE_BYTES
    {
        return Err(Error::ResourceBudgetExceeded);
    }
    Ok(())
}

fn require_writer(replay: &Replay) -> Result<()> {
    if !replay.ready {
        return Err(Error::MissingDependency);
    }
    match replay.role {
        Some(Role::Owner | Role::Editor) => Ok(()),
        Some(Role::Reader) => Err(Error::InsufficientRole),
        None => Err(Error::UnauthorizedDevice),
    }
}

pub(super) fn control_at(state: &SyncState, hash: Hash) -> Result<Signed<Control>> {
    match decode::<Object>(&state.objects.get(&hash).ok_or(Error::MissingDependency)?.0)? {
        Object::Control(control) => Ok(control),
        _ => Err(Error::InvalidEncoding),
    }
}
