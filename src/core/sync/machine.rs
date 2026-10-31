use super::{
    Blob, Certificate, Counter, DeviceSecret, Error, Hash, Object, Projection, Result, Role,
    Signed, Transaction, canonical,
    crypto::{self, CHANGE_DOMAIN, CONTROL_DOMAIN},
    decode,
    model::project,
    wire::sorted_unique,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

const PROFILE: &str = "LBX-SYNC-0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub certificate: Signed<Certificate>,
    pub role: Role,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Epoch {
    profile: String,
    space: Hash,
    number: Counter,
    context: Hash,
    previous: Option<Hash>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    device: Hash,
    enc: Blob,
    ciphertext: Blob,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    epoch: Epoch,
    owner: Hash,
    controller: Hash,
    members: Vec<Member>,
    closed_frontier: Vec<Hash>,
    grants: Vec<Grant>,
    checkpoint_nonce: Blob,
    checkpoint: Blob,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeHeader {
    profile: String,
    space: Hash,
    membership: Hash,
    epoch: Counter,
    author: Hash,
    sequence: Counter,
    previous: Option<Hash>,
    parents: Vec<Hash>,
    nonce: Blob,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    header: ChangeHeader,
    ciphertext: Blob,
}

/// Exact encrypted body reserved before signing. Persist this in the encrypted
/// profile before calling complete_intent; retries must never prepare a new body.
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PreparedChange {
    Change(Change),
    Control(Control),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncState {
    pub space: Hash,
    pub namespace: Hash,
    pub genesis: Hash,
    pub owner_root: [u8; 32],
    pub objects: BTreeMap<Hash, Blob>,
    pub outbox: BTreeSet<Hash>,
    pub intent: Option<PreparedChange>,
    pub fault: Option<Error>,
    pub last_exchange: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceDescriptor {
    pub space: Hash,
    pub namespace: Hash,
    pub genesis: Hash,
    pub owner_root: [u8; 32],
}

impl SyncState {
    pub fn descriptor(&self) -> SpaceDescriptor {
        SpaceDescriptor {
            space: self.space,
            namespace: self.namespace,
            genesis: self.genesis,
            owner_root: self.owner_root,
        }
    }
}

pub struct Replay {
    pub membership: Hash,
    pub epoch: u64,
    pub projection: Projection,
    pub frontier: BTreeSet<Hash>,
    pub accepted: BTreeSet<Hash>,
    pub pending: BTreeSet<Hash>,
    pub rejected: BTreeMap<Hash, Error>,
    pub excluded: BTreeSet<Hash>,
    pub role: Option<Role>,
    pub ready: bool,
    /// False for a new member bootstrapping history it cannot decrypt.
    pub history_verified: bool,
}

pub struct ExcludedEdit {
    pub hash: Hash,
    pub author: Hash,
    pub transaction: Transaction,
}

/// Pure replay and preparation. Filesystem durability and transport are adapters.
pub struct Machine;

impl Machine {
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

    pub fn replay(state: &SyncState, device: &DeviceSecret) -> Result<Replay> {
        if let Some(fault) = state.fault {
            return Err(fault);
        }
        if state.objects.len() > super::MAX_OBJECTS {
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
        let genesis = controls
            .get(&state.genesis)
            .ok_or(Error::MissingDependency)?;
        validate_control(genesis, None, state)?;
        let mut chain = vec![(state.genesis, genesis)];
        let mut seen = BTreeSet::from([state.genesis]);
        let mut rejected = BTreeMap::new();
        loop {
            let (head, previous) = *chain.last().unwrap();
            let mut next = Vec::new();
            for (hash, candidate) in &controls {
                if candidate.body.epoch.previous != Some(head) {
                    continue;
                }
                match validate_control(candidate, Some(previous), state) {
                    Ok(()) => next.push((*hash, candidate)),
                    Err(error) => {
                        rejected.insert(*hash, error);
                    }
                }
            }
            if next.len() > 1 {
                return Err(Error::ControlFork);
            }
            let Some((hash, control)) = next.pop() else {
                break;
            };
            if !seen.insert(hash) {
                return Err(Error::ControlFork);
            }
            chain.push((hash, control));
        }
        let (tip, current) = *chain.last().unwrap();
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
            pending: BTreeSet::new(),
            rejected,
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
        if plaintext.len() > super::MAX_TRANSACTION_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        if replay.frontier.len() > super::MAX_PARENTS {
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

    pub fn prepare_rebase(
        state: &SyncState,
        device: &DeviceSecret,
        excluded: Hash,
    ) -> Result<PreparedChange> {
        use super::{Action, Mutation};
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
    if bytes > super::MAX_OBJECT_BYTES
        || state.objects.len() >= super::MAX_OBJECTS
        || state
            .objects
            .values()
            .map(|blob| blob.0.len())
            .sum::<usize>()
            .saturating_add(bytes)
            > super::MAX_CACHE_BYTES
    {
        return Err(Error::ResourceBudgetExceeded);
    }
    Ok(())
}

impl Object {
    pub(crate) fn dependencies(&self) -> Vec<Hash> {
        match self {
            Self::Control(control) => control
                .body
                .epoch
                .previous
                .into_iter()
                .chain(control.body.closed_frontier.iter().copied())
                .collect(),
            Self::Change(change) => std::iter::once(change.body.header.membership)
                .chain(change.body.header.parents.iter().copied())
                .collect(),
        }
    }
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

fn control_at(state: &SyncState, hash: Hash) -> Result<Signed<Control>> {
    match decode::<Object>(&state.objects.get(&hash).ok_or(Error::MissingDependency)?.0)? {
        Object::Control(control) => Ok(control),
        _ => Err(Error::InvalidEncoding),
    }
}

fn nonce(blob: &Blob) -> Result<[u8; 24]> {
    blob.0
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidEncoding)
}

fn grant_aad(epoch: &Epoch, certificate: &Signed<Certificate>) -> Result<Vec<u8>> {
    canonical(&(
        epoch,
        Hash::digest(
            b"lockbox.sync.certificate-object.v0\0",
            &canonical(certificate)?,
        ),
    ))
}

fn epoch_key(control: &Control, device: &DeviceSecret) -> Result<Zeroizing<[u8; 32]>> {
    let grant = control
        .grants
        .iter()
        .find(|g| g.device == device.id())
        .ok_or(Error::KeyUnavailable)?;
    let member = control
        .members
        .iter()
        .find(|m| m.certificate.body.device == device.id())
        .ok_or(Error::UnauthorizedDevice)?;
    device.open_grant(
        &grant.enc,
        &grant.ciphertext,
        &grant_aad(&control.epoch, &member.certificate)?,
    )
}

fn build_control(
    epoch: Epoch,
    owner: Hash,
    controller: Hash,
    members: Vec<Member>,
    closed_frontier: Vec<Hash>,
    baseline: &Projection,
    key: &[u8; 32],
) -> Result<Control> {
    let mut grants = Vec::new();
    for member in &members {
        member.certificate.verify()?;
        let (enc, ciphertext) = crypto::seal_grant(
            &member.certificate.body.hpke_key,
            key,
            &grant_aad(&epoch, &member.certificate)?,
        )?;
        grants.push(Grant {
            device: member.certificate.body.device,
            enc,
            ciphertext,
        });
    }
    let checkpoint_key = crypto::content_key(key, epoch.space, epoch.context, true);
    let mut checkpoint_nonce = [0; 24];
    getrandom::fill(&mut checkpoint_nonce).map_err(|_| Error::Random)?;
    let plaintext = Zeroizing::new(canonical(baseline)?);
    let checkpoint = crypto::encrypt(
        &checkpoint_key,
        &checkpoint_nonce,
        &plaintext,
        &canonical(&epoch)?,
    )?;
    Ok(Control {
        epoch,
        owner,
        controller,
        members,
        closed_frontier,
        grants,
        checkpoint_nonce: Blob(checkpoint_nonce.to_vec()),
        checkpoint,
    })
}

fn validate_control(
    control: &Signed<Control>,
    previous: Option<&Signed<Control>>,
    state: &SyncState,
) -> Result<()> {
    let key = validate_control_body(&control.body, previous, state)?;
    crypto::verify(control, &key, CONTROL_DOMAIN)
}

fn validate_control_body(
    body: &Control,
    previous: Option<&Signed<Control>>,
    state: &SyncState,
) -> Result<[u8; 32]> {
    if body.epoch.profile != PROFILE {
        return Err(Error::UnsupportedProtocol);
    }
    if body.epoch.space != state.space
        || body.owner != Hash::digest(b"lockbox.user.v1\0", &state.owner_root)
        || body.members.is_empty()
        || body.members.len() > 512
        || body.grants.len() != body.members.len()
        || body.closed_frontier.len() > super::MAX_PARENTS
        || !sorted_unique(&body.closed_frontier)
        || !sorted_unique(
            &body
                .members
                .iter()
                .map(|m| m.certificate.body.device)
                .collect::<Vec<_>>(),
        )
        || body.checkpoint_nonce.0.len() != 24
        || body.checkpoint.0.len() < 16
    {
        return Err(Error::InvalidEncoding);
    }
    let mut users = BTreeMap::new();
    for (member, grant) in body.members.iter().zip(&body.grants) {
        member.certificate.verify()?;
        let certificate = &member.certificate.body;
        if (member.role == Role::Owner) != (certificate.user == body.owner)
            || grant.device != certificate.device
            || grant.enc.0.len() != 32
            || grant.ciphertext.0.len() != 48
        {
            return Err(Error::UnauthorizedDevice);
        }
        let user = users.entry(certificate.user).or_insert((member.role, 0));
        if user.0 != member.role {
            return Err(Error::UnauthorizedDevice);
        }
        user.1 += 1;
        if user.1 > 16 {
            return Err(Error::ResourceBudgetExceeded);
        }
    }
    if users.len() > 128 {
        return Err(Error::ResourceBudgetExceeded);
    }
    let controller = body
        .members
        .iter()
        .find(|m| m.certificate.body.device == body.controller && m.role == Role::Owner)
        .ok_or(Error::UnauthorizedDevice)?;
    match previous {
        None => {
            if body.epoch.number.0 != 0
                || body.epoch.previous.is_some()
                || !body.closed_frontier.is_empty()
                || controller.certificate.body.root_key != state.owner_root
            {
                return Err(Error::UnauthorizedDevice);
            }
            Ok(controller.certificate.body.signing_key)
        }
        Some(previous) => {
            let author = previous
                .body
                .members
                .iter()
                .find(|m| m.certificate.body.device == previous.body.controller)
                .ok_or(Error::UnauthorizedDevice)?;
            if previous.body.epoch.number.0.checked_add(1) != Some(body.epoch.number.0)
                || body.owner != previous.body.owner
                || body.controller != previous.body.controller
                || body.epoch.context == previous.body.epoch.context
                || body.epoch.previous != Some(Object::Control(previous.clone()).hash()?)
            {
                return Err(Error::UnauthorizedDevice);
            }
            Ok(author.certificate.body.signing_key)
        }
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
    next: Option<&Control>,
) -> Result<Replay> {
    let mut verified = BTreeMap::new();
    let mut rejected = BTreeMap::new();
    for (hash, signed) in changes {
        let header = &signed.body.header;
        if header.membership != membership {
            continue;
        }
        let check = || -> Result<Transaction> {
            if header.profile != PROFILE {
                return Err(Error::UnsupportedProtocol);
            }
            if header.space != control.epoch.space
                || header.epoch != control.epoch.number
                || header.sequence.0 == 0
                || header.parents.len() > super::MAX_PARENTS
                || !sorted_unique(&header.parents)
            {
                return Err(Error::InvalidTransaction);
            }
            let member = control
                .members
                .iter()
                .find(|m| m.certificate.body.device == header.author)
                .ok_or(Error::UnauthorizedDevice)?;
            crypto::verify(signed, &member.certificate.body.signing_key, CHANGE_DOMAIN)?;
            if member.role == Role::Reader {
                return Err(Error::InsufficientRole);
            }
            let plaintext = crypto::decrypt(
                key,
                &nonce(&header.nonce)?,
                &signed.body.ciphertext,
                &canonical(header)?,
            )?;
            if plaintext.len() > super::MAX_TRANSACTION_BYTES {
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
    let mut excluded = BTreeSet::new();
    let mut missing_closed = BTreeSet::new();
    if let Some(next) = next {
        let mut relevant = BTreeSet::new();
        let mut stack = next.closed_frontier.clone();
        while let Some(hash) = stack.pop() {
            if !relevant.insert(hash) {
                continue;
            }
            if relevant.len() > super::MAX_OBJECTS {
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
    let mut slots = BTreeMap::new();
    for (hash, change) in &verified {
        let header = &change.signed.body.header;
        if slots
            .insert((header.author, header.sequence), *hash)
            .is_some()
        {
            return Err(Error::DeviceFork);
        }
    }
    let mut accepted: BTreeMap<Hash, VerifiedChange> = BTreeMap::new();
    loop {
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
        if ready.is_empty() {
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
                    rejected.insert(hash, error);
                }
            }
        }
    }
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

fn ancestry(parents: &[Hash], changes: &BTreeMap<Hash, VerifiedChange>) -> Result<BTreeSet<Hash>> {
    let mut visited = BTreeSet::new();
    let mut stack = parents.to_vec();
    while let Some(hash) = stack.pop() {
        if !visited.insert(hash) {
            continue;
        }
        if visited.len() > super::MAX_OBJECTS {
            return Err(Error::ResourceBudgetExceeded);
        }
        let change = changes.get(&hash).ok_or(Error::MissingDependency)?;
        stack.extend(&change.signed.body.header.parents);
    }
    Ok(visited)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
