//! Prepare commands and complete reserved signing intents without performing I/O.

use crate::serialization::canonical_size;
use crate::sync::{
    identity::Key,
    protocol::{
        Blob, Certificate, Counter, DeviceSecret, Error, Hash, Object, Projection, Result, Role,
        Signed, Transaction, canonical,
        crypto::{self, nonce},
        decode,
        machine::{
            Machine,
            history::with_history,
            membership::{build_control, epoch_key, validate_control_body},
            protocol::{Change, ChangeHeader, Control, Epoch, Member},
            state::{ExcludedEdit, PreparedChange, Replay, SyncState},
        },
    },
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
        epoch_key: &Key,
        context: Hash,
    ) -> Result<SyncState> {
        certificate.verify()?;

        if !device.matches_certificate(&certificate.body) {
            return Err(Error::UnauthorizedDevice);
        }

        let root = certificate.body.root_key;
        let owner = certificate.body.user;
        let members = vec![Member {
            certificate,
            role: Role::Owner,
        }];
        let epoch = Epoch {
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
        let object = Object::Control(crypto::sign(
            control,
            &device.signer(),
            crypto::CONTROL_DOMAIN,
        )?);
        let (genesis, bytes) = object.encode()?;
        Ok(SyncState {
            space,
            namespace,
            genesis,
            owner_root: root,
            required_control: None,
            objects: BTreeMap::from([(genesis, Blob(bytes))]),
            candidates: BTreeMap::new(),
            rejected_candidates: BTreeMap::new(),
            outbox: BTreeSet::from([genesis]),
            intent: None,
            fault: None,
            last_exchange: None,
        })
    }

    /// Bootstrap trust must come from an independently authenticated owner root,
    /// space ID and genesis hash. Provider-supplied names never establish trust.
    pub fn join(space: Hash, namespace: Hash, genesis: Hash, owner_root: Key) -> SyncState {
        SyncState {
            space,
            namespace,
            genesis,
            owner_root,
            required_control: None,
            objects: BTreeMap::new(),
            candidates: BTreeMap::new(),
            rejected_candidates: BTreeMap::new(),
            outbox: BTreeSet::new(),
            intent: None,
            fault: None,
            last_exchange: None,
        }
    }

    /// Join with an independently authenticated minimum control anchor. Only a
    /// verified chain containing this exact head can authorize publication.
    pub fn join_at(
        descriptor: crate::sync::protocol::machine::SpaceDescriptor,
        control: Hash,
    ) -> SyncState {
        let mut state = Self::join(
            descriptor.space,
            descriptor.namespace,
            descriptor.genesis,
            descriptor.owner_root,
        );
        state.required_control = Some(control);
        state
    }

    /// Quarantine downloads until their public authority can be authenticated.
    /// Plaintext and causal validity remain replay's responsibility.
    pub fn receive(state: &mut SyncState, expected: Hash, bytes: Vec<u8>) -> Result<bool> {
        let object = Object::decode(&bytes, expected)?;
        if state.rejected_candidates.contains_key(&expected) {
            return Ok(false);
        }

        if matches!(&object, Object::Control(control) if control.body.epoch.previous.is_none() && expected != state.genesis)
        {
            super::candidates::reject(state, expected, Error::UnauthorizedDevice);
            return Ok(true);
        }

        if let Some(existing) = state
            .objects
            .get(&expected)
            .or_else(|| state.candidates.get(&expected))
        {
            if existing.0 != bytes {
                return Err(Error::HashMismatch);
            }
            return Ok(false);
        }
        super::candidates::insert(state, expected, Blob(bytes));
        super::candidates::promote(state)?;
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
        Self::prepare_borrowed(state, device, &transaction)
    }

    /// Reuse a reviewed transaction across host retries without copying its payloads.
    pub(crate) fn prepare_borrowed(
        state: &SyncState,
        device: &DeviceSecret,
        transaction: &Transaction,
    ) -> Result<PreparedChange> {
        state.require_idle()?;

        let replay = Self::replay(state, device)?;
        replay.require_writer()?;
        Self::prepare_transaction(state, device, transaction, replay, false)
    }

    /// Prepare one projection-preserving causal merge when more than 64 heads
    /// would block an edit or membership change. Each event reduces the frontier
    /// by 63 without resolving record conflicts. Persist and complete it through
    /// the same signing-intent boundary as an ordinary edit, then repeat as needed.
    /// Readers cannot create merges. `None` means no folding is needed.
    pub fn prepare_merge(
        state: &SyncState,
        device: &DeviceSecret,
    ) -> Result<Option<PreparedChange>> {
        state.require_idle()?;

        let replay = Self::replay(state, device)?;
        replay.require_writer()?;

        if replay.frontier.len() <= crate::sync::protocol::MAX_PARENTS {
            return Ok(None);
        }
        Self::prepare_transaction(
            state,
            device,
            &Transaction {
                mutations: vec![],
                rebased_from: None,
            },
            replay,
            true,
        )
        .map(Some)
    }

    // Callers check idle state and writer authority against this same immutable state.
    fn prepare_transaction(
        state: &SyncState,
        device: &DeviceSecret,
        transaction: &Transaction,
        replay: Replay,
        folding: bool,
    ) -> Result<PreparedChange> {
        transaction.validate(&replay.projection)?;
        let plaintext = Zeroizing::new(canonical(transaction)?);

        if plaintext.len() > crate::sync::protocol::MAX_TRANSACTION_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }

        if !folding && replay.frontier.len() > crate::sync::protocol::MAX_PARENTS {
            return Err(Error::FrontierTooWide);
        }

        let control = control_at(state, replay.membership)?;
        let key = epoch_key(&control.body, device)?;
        let content_key = crypto::content_key(&key, state.space, control.body.epoch.context, false);
        let device_id = device.id();
        let mut previous = None;
        let mut sequence: u64 = 0;

        for hash in &replay.accepted {
            let Object::Change(change) = decode::<Object>(&state.objects[hash].0)? else {
                continue;
            };

            if change.body.header.membership == replay.membership
                && change.body.header.author == device_id
                && change.body.header.sequence.0 > sequence
            {
                previous = Some(*hash);
                sequence = change.body.header.sequence.0;
            }
        }

        let parents = if folding {
            crate::sync::protocol::machine::parents::fold_parents(state, &replay, previous)?
        } else {
            replay.frontier.into_iter().collect()
        };

        let mut nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(|_| Error::Random)?;
        let header = ChangeHeader {
            space: state.space,
            membership: replay.membership,
            epoch: Counter(replay.epoch),
            author: device_id,
            sequence: Counter(
                sequence
                    .checked_add(1)
                    .ok_or(Error::ResourceBudgetExceeded)?,
            ),
            previous,
            parents,
            nonce: Blob(nonce.to_vec()),
        };

        let ciphertext = crypto::encrypt(&content_key, &nonce, &plaintext, &canonical(&header)?)?;
        let change = Change { header, ciphertext };
        PreparedChange::Change(change).check_capacity(state)
    }

    /// Owner explicitly supplies the complete next roster, including its own
    /// certificate. Admission never follows possession of provider credentials.
    /// Controller transfer uses a separate approval flow.
    pub fn prepare_membership(
        state: &SyncState,
        device: &DeviceSecret,
        members: Vec<(Signed<Certificate>, Role)>,
    ) -> Result<PreparedChange> {
        Self::prepare_membership_with_admissions(state, device, members, vec![])
    }

    pub(super) fn prepare_membership_with_admissions(
        state: &SyncState,
        device: &DeviceSecret,
        members: Vec<(Signed<Certificate>, Role)>,
        admissions: Vec<crate::sync::protocol::Admission>,
    ) -> Result<PreparedChange> {
        state.require_idle()?;

        let replay = Self::replay(state, device)?;
        replay.require_writer()?;
        let previous = control_at(state, replay.membership)?;

        if previous.body.controller != device.id() {
            return Err(Error::InsufficientRole);
        }

        if replay.frontier.len() > crate::sync::protocol::MAX_PARENTS {
            return Err(Error::FrontierTooWide);
        }

        let mut members: Vec<_> = members
            .into_iter()
            .map(|(certificate, role)| Member { certificate, role })
            .collect();
        members.sort_by_key(|m| m.certificate.body.device);
        let epoch = Epoch {
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

        let mut control = build_control(
            epoch,
            previous.body.owner,
            device.id(),
            members,
            replay.frontier.into_iter().collect(),
            &replay.projection,
            &*crypto::random32()?,
        )?;
        control.admissions = admissions;
        // Validate shape and authority without producing a signature before the
        // caller has durably reserved this successor slot.
        with_history(state, |history| {
            validate_control_body(&control, Some(&previous), state, &history.chain)
        })?;
        PreparedChange::Control(control).check_capacity(state)
    }

    /// Call only after state.intent has been persisted. Ed25519 deterministically
    /// reproduces the exact signature after a crash or a lost save response.
    pub fn complete_intent(state: &mut SyncState, device: &DeviceSecret) -> Result<Hash> {
        let prepared = state.intent.clone().ok_or(Error::PendingIntent)?;
        let replay = Self::replay(state, device)?;
        prepared.verify_signing_authority(&replay)?;
        let object = match prepared {
            PreparedChange::Change(change) => {
                if change.header.membership != replay.membership
                    || change.header.author != device.id()
                {
                    return Err(Error::UnauthorizedDevice);
                }

                let domain = crypto::CHANGE_DOMAIN;
                Object::Change(crypto::sign(change, &device.signer(), domain)?)
            }
            PreparedChange::Control(control) => {
                if control.epoch.previous != Some(replay.membership) {
                    return Err(Error::UnauthorizedDevice);
                }

                let previous = control_at(state, replay.membership)?;

                if control.signing_controller(&previous.body) != device.id() {
                    return Err(Error::InsufficientRole);
                }
                with_history(state, |history| {
                    validate_control_body(&control, Some(&previous), state, &history.chain)
                })?;
                control.verify_approval(Some(&previous), state)?;
                let domain = control.signing_domain();
                let signed = crypto::sign(control, &device.signer(), domain)?;
                Object::Control(signed)
            }
        };

        // This exact body was locally reserved and signed above. Insert directly
        // and verify through replay; remote downloads take the quarantine path.
        let (hash, bytes) = object.encode()?;
        let inserted = if let Some(existing) = state.objects.get(&hash) {
            if existing.0 != bytes {
                return Err(Error::HashMismatch);
            }
            false
        } else {
            capacity(state, bytes.len())?;
            state.objects.insert(hash, Blob(bytes));
            true
        };
        let check =
            Self::replay(state, device).and_then(|checked| checked.verify_applied(&object, hash));

        if let Err(error) = check {
            if inserted {
                state.objects.remove(&hash);
            }
            return Err(error);
        }
        state.candidates.remove(&hash);
        state.rejected_candidates.remove(&hash);
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
        Self::decrypt_excluded(state, device, &replay)
    }

    fn decrypt_excluded(
        state: &SyncState,
        device: &DeviceSecret,
        replay: &Replay,
    ) -> Result<Vec<ExcludedEdit>> {
        let mut edits = Vec::new();

        for &hash in &replay.excluded {
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
        use crate::sync::protocol::Action;
        use crate::sync::protocol::Mutation;
        let replay = Self::replay(state, device)?;
        replay.require_writer()?;
        let edit = Self::decrypt_excluded(state, device, &replay)?
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
        state.require_idle()?;
        Self::prepare_transaction(
            state,
            device,
            &Transaction {
                mutations,
                rebased_from: Some(excluded),
            },
            replay,
            false,
        )
    }
}

impl PreparedChange {
    pub(super) fn check_capacity(self, state: &SyncState) -> Result<Self> {
        // Reserve the final signature's space without signing before persistence.
        let signature = Blob(vec![0; 64]);
        let object = match self {
            Self::Change(body) => Object::Change(Signed { body, signature }),
            Self::Control(body) => Object::Control(Signed { body, signature }),
        };
        let bytes = canonical_size(&object).map_err(|_| Error::InvalidEncoding)?;
        capacity(state, bytes)?;
        Ok(match object {
            Object::Change(signed) => Self::Change(signed.body),
            Object::Control(signed) => Self::Control(signed.body),
        })
    }
}

pub(super) fn capacity(state: &SyncState, bytes: usize) -> Result<()> {
    if bytes > crate::sync::protocol::MAX_OBJECT_BYTES
        || state.objects.len() >= crate::sync::protocol::MAX_OBJECTS
        || state
            .objects
            .values()
            .map(|blob| blob.0.len())
            .sum::<usize>()
            .saturating_add(bytes)
            > crate::sync::protocol::MAX_CACHE_BYTES
    {
        return Err(Error::ResourceBudgetExceeded);
    }

    Ok(())
}

pub(super) fn control_at(state: &SyncState, hash: Hash) -> Result<Signed<Control>> {
    match decode::<Object>(&state.objects.get(&hash).ok_or(Error::MissingDependency)?.0)? {
        Object::Control(control) => Ok(control),
        _ => Err(Error::InvalidEncoding),
    }
}
