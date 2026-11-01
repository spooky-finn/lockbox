//! Explicit controller transfer and root-authorized recovery. Detached approvals
//! bind the full prepared statement; the final device signature covers the
//! approval too. No root or incoming-controller approval authorizes itself.

use super::{
    Machine, commands::control_at, history::with_history,
    membership::{build_control, epoch_key, validate_control_body},
    profile::{PROFILE, Profile},
    protocol::{Control, ControlApproval, ControlTransfer, ControlTransition, Epoch, Member, RecoveryAnchor},
    state::{PreparedChange, SyncState},
};
use crate::core::sync::{Blob, Certificate, Counter, DeviceSecret, Error, Hash, Object, Projection,
    Result, Role, Signed, canonical, crypto, decode, wire};
use ed25519_dalek::SigningKey;
use std::collections::BTreeSet;

const APPROVAL_PROFILE: &str = "LBX-CONTROL-2";
const TRANSFER_DOMAIN: &[u8] = b"lockbox.sync.control-transfer-approval.v2\0";
const OFFER_DOMAIN: &[u8] = b"lockbox.sync.control-transfer-offer.v2\0";
const RECOVERY_DOMAIN: &[u8] = b"lockbox.sync.control-recovery-root.v2\0";
const RECOVERY_DEVICE_DOMAIN: &[u8] = b"lockbox.sync.control-recovery-device.v2\0";

pub(super) fn control_domain(body: &Control) -> Result<&'static [u8]> {
    Ok(if body.transition == Some(ControlTransition::Recovery) {
        RECOVERY_DEVICE_DOMAIN
    } else { Profile::parse(&body.epoch.profile)?.control_domain() })
}

fn statement(body: &Control) -> Result<Hash> {
    let mut unsigned = body.clone();
    unsigned.approval = None;
    Ok(Hash::digest(b"lockbox.sync.control-statement.v2\0", &canonical(&unsigned)?))
}

pub(super) fn verify_approval(body: &Control, previous: Option<&Signed<Control>>, state: &SyncState) -> Result<()> {
    let Some(transition) = body.transition else {
        return if body.approval.is_none() { Ok(()) } else { Err(Error::InvalidEncoding) };
    };
    let previous = previous.ok_or(Error::UnauthorizedDevice)?;
    let approval = body.approval.as_ref().ok_or(Error::ControlApprovalRequired)?;
    let (key, author, domain) = match transition {
        ControlTransition::Transfer => {
            let target = body.members.iter().find(|m| m.certificate.body.device == body.controller && m.role == Role::Owner)
                .ok_or(Error::UnauthorizedDevice)?;
            (target.certificate.body.signing_key, body.controller, TRANSFER_DOMAIN)
        }
        ControlTransition::Recovery => {
            let owner = previous.body.members.iter().find(|m| m.certificate.body.device == previous.body.controller && m.role == Role::Owner)
                .ok_or(Error::UnauthorizedDevice)?;
            (owner.certificate.body.root_key, previous.body.owner, RECOVERY_DOMAIN)
        }
    };
    if approval.body.profile != APPROVAL_PROFILE || approval.body.descriptor != state.descriptor()
        || approval.body.statement != statement(body)? || approval.body.author != author {
        return Err(Error::UnauthorizedDevice);
    }
    crypto::verify(approval, &key, domain)
}

fn sign_approval(body: &Control, state: &SyncState, key: &SigningKey, author: Hash, domain: &[u8]) -> Result<Signed<ControlApproval>> {
    crypto::sign(ControlApproval { profile: APPROVAL_PROFILE.into(), descriptor: state.descriptor(),
        statement: statement(body)?, author }, key, domain)
}

impl Signed<ControlApproval> {
    pub fn bytes(&self) -> Result<Vec<u8>> { canonical(self) }
    pub fn decode(bytes: &[u8]) -> Result<Self> { decode(bytes) }
}

impl ControlTransfer {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        let bytes = canonical(self)?;
        if bytes.len() > crate::core::sync::MAX_INVITATION_BYTES { return Err(Error::ResourceBudgetExceeded); }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        wire::decode_bounded(bytes, crate::core::sync::MAX_INVITATION_BYTES)
    }
    pub fn descriptor(&self) -> &super::SpaceDescriptor { &self.descriptor }
    pub fn target(&self) -> Hash { self.control.controller }
}

impl RecoveryAnchor {
    pub fn bytes(&self) -> Result<Vec<u8>> { canonical(self) }
    pub fn decode(bytes: &[u8]) -> Result<Self> { decode(bytes) }
}

impl Machine {
    pub fn recovery_anchor(state: &SyncState, device: &DeviceSecret) -> Result<RecoveryAnchor> {
        let replay = Self::replay(state, device)?;
        if !replay.ready { return Err(Error::MissingDependency); }
        Ok(RecoveryAnchor { descriptor: state.descriptor(), control: replay.membership,
            frontier: replay.frontier.into_iter().collect() })
    }

    /// Prepare without signing. Persist this exact control intent before making
    /// a transfer offer. The incoming controller must already be admitted Owner.
    pub fn prepare_control_transfer(state: &SyncState, device: &DeviceSecret, target: Hash) -> Result<PreparedChange> {
        if state.intent.is_some() { return Err(Error::PendingIntent); }
        let replay = Self::replay(state, device)?;
        if !replay.ready { return Err(Error::MissingDependency); }
        let previous = control_at(state, replay.membership)?;
        if previous.body.controller != device.id() { return Err(Error::InsufficientRole); }
        if replay.frontier.len() > crate::core::sync::MAX_PARENTS { return Err(Error::FrontierTooWide); }
        let control = successor(state, &previous.body, &replay, target, previous.body.members.clone(), ControlTransition::Transfer)?;
        with_history(state, |h| validate_control_body(&control, Some(&previous), state, &h.chain))?;
        Ok(PreparedChange::Control(control))
    }

    /// The owner root alone cannot recreate content. The source must replay the
    /// exact independently authenticated control and data frontier supplied here.
    pub fn prepare_control_recovery(source: &SyncState, source_device: &DeviceSecret,
        certificate: Signed<Certificate>, anchor: &RecoveryAnchor) -> Result<PreparedChange> {
        if source.intent.is_some() { return Err(Error::PendingIntent); }
        certificate.verify()?;
        let replay = Self::replay(source, source_device)?;
        if !replay.ready || source.descriptor() != anchor.descriptor || replay.membership != anchor.control
            || replay.frontier.iter().copied().collect::<Vec<_>>() != anchor.frontier {
            return Err(Error::MissingDependency);
        }
        let previous = control_at(source, replay.membership)?;
        if certificate.body.root_key != source.owner_root { return Err(Error::UnauthorizedDevice); }
        let target = certificate.body.device;
        let mut members: Vec<_> = previous.body.members.iter()
            .filter(|m| m.certificate.body.device != previous.body.controller && m.certificate.body.device != target).cloned().collect();
        members.push(Member { certificate, role: Role::Owner });
        members.sort_by_key(|m| m.certificate.body.device);
        let control = successor(source, &previous.body, &replay, target, members, ControlTransition::Recovery)?;
        with_history(source, |h| validate_control_body(&control, Some(&previous), source, &h.chain))?;
        Ok(PreparedChange::Control(control))
    }

    /// Only after the unsigned recovery intent is durably reserved. Persist the
    /// resulting root approval before completing the device signature.
    pub fn approve_control_recovery(state: &mut SyncState, device: &DeviceSecret, root: &SigningKey) -> Result<()> {
        if root.verifying_key().to_bytes() != state.owner_root { return Err(Error::UnauthorizedDevice); }
        let Some(PreparedChange::Control(control)) = &state.intent else { return Err(Error::PendingIntent); };
        if control.transition != Some(ControlTransition::Recovery) || control.controller != device.id() {
            return Err(Error::UnauthorizedDevice);
        }
        validate_pending(state, control)?;
        checkpoint(control, device)?.validate()?;
        let approval = sign_approval(control, state, root, control.owner, RECOVERY_DOMAIN)?;
        let Some(PreparedChange::Control(control)) = &mut state.intent else { unreachable!() };
        control.approval = Some(approval);
        Ok(())
    }

    /// The offer signature has no successor authority. Until new-controller
    /// consent arrives this reserved intent cannot emit a control signature.
    pub fn control_transfer_offer(state: &SyncState, device: &DeviceSecret) -> Result<ControlTransfer> {
        let Some(PreparedChange::Control(control)) = &state.intent else { return Err(Error::PendingIntent); };
        if control.transition != Some(ControlTransition::Transfer) || control.approval.is_some() {
            return Err(Error::UnauthorizedDevice);
        }
        let previous = validate_pending(state, control)?;
        if previous.body.controller != device.id() { return Err(Error::InsufficientRole); }
        let signed = crypto::sign((state.descriptor(), control.clone()), &device.signer(), OFFER_DOMAIN)?;
        let closure = dependencies(state, control)?;
        let offer = ControlTransfer { descriptor: state.descriptor(), control: control.clone(),
            signature: signed.signature, objects: closure.into_iter().map(|h| state.objects[&h].clone()).collect() };
        offer.bytes()?;
        Ok(offer)
    }

    pub fn accept_control_transfer(state: &SyncState, device: &DeviceSecret, offer: &ControlTransfer) -> Result<Signed<ControlApproval>> {
        if state.intent.is_some() { return Err(Error::PendingIntent); }
        if offer.descriptor != state.descriptor() || offer.control.transition != Some(ControlTransition::Transfer)
            || offer.control.approval.is_some() || offer.control.controller != device.id() {
            return Err(Error::UnauthorizedDevice);
        }
        if offer.objects.len() > crate::core::sync::MAX_OBJECTS
            || offer.objects.iter().map(|b| b.0.len()).sum::<usize>() > crate::core::sync::MAX_CACHE_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        let mut candidate = state.clone();
        for blob in &offer.objects {
            let object: Object = decode(&blob.0)?;
            Self::receive(&mut candidate, object.hash()?, blob.0.clone())?;
        }
        let previous = validate_pending(&candidate, &offer.control)?;
        let author = previous.body.members.iter().find(|m| m.certificate.body.device == previous.body.controller)
            .ok_or(Error::UnauthorizedDevice)?;
        crypto::verify(&Signed { body: (offer.descriptor.clone(), offer.control.clone()), signature: offer.signature.clone() },
            &author.certificate.body.signing_key, OFFER_DOMAIN)?;
        // Replay only the explicitly closed ancestry. Later edits stay in the
        // real journal as excluded drafts when the transfer eventually arrives.
        let closure = dependencies(&candidate, &offer.control)?;
        candidate.objects.retain(|hash, _| closure.contains(hash));
        let replay = Self::replay(&candidate, device)?;
        if !replay.ready { return Err(Error::MissingDependency); }
        if replay.frontier.iter().copied().collect::<Vec<_>>() != offer.control.closed_frontier
            || replay.projection != checkpoint(&offer.control, device)? {
            return Err(Error::CheckpointMismatch);
        }
        sign_approval(&offer.control, state, &device.signer(), device.id(), TRANSFER_DOMAIN)
    }

    pub fn approve_control_transfer(state: &mut SyncState, approval: Signed<ControlApproval>) -> Result<()> {
        let Some(PreparedChange::Control(control)) = &state.intent else { return Err(Error::PendingIntent); };
        if control.transition != Some(ControlTransition::Transfer) { return Err(Error::UnauthorizedDevice); }
        let previous = validate_pending(state, control)?;
        let mut updated = control.clone(); updated.approval = Some(approval);
        verify_approval(&updated, Some(&previous), state)?;
        state.intent = Some(PreparedChange::Control(updated));
        Ok(())
    }
}

fn validate_pending(state: &SyncState, body: &Control) -> Result<Signed<Control>> {
    if let Some(fault) = state.fault { return Err(fault); }
    with_history(state, |h| {
        let (tip, previous) = h.tip();
        if body.epoch.previous != Some(tip) || state.required_control.is_some_and(|pin| !h.contains(pin)) {
            return Err(Error::MissingDependency);
        }
        validate_control_body(body, Some(previous), state, &h.chain)?;
        Ok(previous.clone())
    })
}

fn successor(state: &SyncState, previous: &Control, replay: &super::Replay, target: Hash,
    members: Vec<Member>, transition: ControlTransition) -> Result<Control> {
    let epoch = Epoch { profile: PROFILE.into(), space: state.space,
        number: Counter(replay.epoch.checked_add(1).ok_or(Error::ResourceBudgetExceeded)?),
        context: Hash(*crypto::random32()?), previous: Some(replay.membership) };
    let mut control = build_control(epoch, previous.owner, target, members,
        replay.frontier.iter().copied().collect(), &replay.projection, &*crypto::random32()?)?;
    control.transition = Some(transition);
    // Reserve enough bytes for both approvals before any signature can exist.
    let mut sized = control.clone();
    sized.approval = Some(Signed { body: ControlApproval { profile: APPROVAL_PROFILE.into(),
        descriptor: state.descriptor(), statement: statement(&control)?, author: target }, signature: Blob(vec![0;64]) });
    super::commands::capacity(state, Object::Control(Signed { body: sized, signature: Blob(vec![0;64]) }).bytes()?.len())?;
    Ok(control)
}

fn checkpoint(body: &Control, device: &DeviceSecret) -> Result<Projection> {
    let key = epoch_key(body, device)?;
    let content_key = crypto::content_key(&key, body.epoch.space, body.epoch.context, true);
    decode(&crypto::decrypt(&content_key, &crypto::nonce(&body.checkpoint_nonce)?, &body.checkpoint, &canonical(&body.epoch)?)?)
}

fn dependencies(state: &SyncState, next: &Control) -> Result<BTreeSet<Hash>> {
    let mut pending: Vec<_> = next.epoch.previous.into_iter().chain(next.closed_frontier.iter().copied()).collect();
    let mut closure = BTreeSet::new();
    while let Some(hash) = pending.pop() {
        if !closure.insert(hash) { continue; }
        if closure.len() > crate::core::sync::MAX_OBJECTS { return Err(Error::ResourceBudgetExceeded); }
        let object: Object = decode(&state.objects.get(&hash).ok_or(Error::MissingDependency)?.0)?;
        pending.extend(object.dependencies());
    }
    Ok(closure)
}
