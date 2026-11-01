//! Incoming-controller consent and signed transfer offers.

use super::super::{
    Machine, SpaceDescriptor,
    commands::control_at,
    protocol::{Control, ControlApproval, ControlTransfer, ControlTransition},
    state::{PreparedChange, SyncState},
};
use super::TRANSFER_DOMAIN;

use crate::core::sync::{
    DeviceSecret, Error, Hash, MAX_CACHE_BYTES, MAX_INVITATION_BYTES, MAX_OBJECTS, MAX_PARENTS,
    Object, Result, Signed, canonical, crypto, decode, wire,
};
use std::collections::BTreeSet;

const OFFER_DOMAIN: &[u8] = b"lockbox.sync.control-transfer-offer\0";

impl ControlTransfer {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        let bytes = canonical(self)?;
        if bytes.len() > MAX_INVITATION_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        wire::decode_bounded(bytes, MAX_INVITATION_BYTES)
    }

    pub fn descriptor(&self) -> &SpaceDescriptor {
        &self.descriptor
    }

    pub fn target(&self) -> Hash {
        self.control.controller
    }

    fn verify_recipient(&self, state: &SyncState, device: &DeviceSecret) -> Result<()> {
        if self.descriptor != state.descriptor()
            || self.control.pending_approval() != Some(ControlTransition::Transfer)
            || self.target() != device.id()
        {
            return Err(Error::UnauthorizedDevice);
        }
        Ok(())
    }

    fn receive_objects(&self, state: &SyncState) -> Result<SyncState> {
        if self.objects.len() > MAX_OBJECTS
            || self.objects.iter().map(|blob| blob.0.len()).sum::<usize>() > MAX_CACHE_BYTES
        {
            return Err(Error::ResourceBudgetExceeded);
        }
        let mut candidate = state.clone();
        for blob in &self.objects {
            let object: Object = decode(&blob.0)?;
            Machine::receive(&mut candidate, object.hash()?, blob.0.clone())?;
        }
        Ok(candidate)
    }

    fn verify_signature(&self, previous: &Control) -> Result<()> {
        let author = previous
            .members
            .iter()
            .find(|member| member.certificate.body.device == previous.controller)
            .ok_or(Error::UnauthorizedDevice)?;
        crypto::verify(
            &Signed {
                body: (self.descriptor.clone(), self.control.clone()),
                signature: self.signature.clone(),
            },
            &author.certificate.body.signing_key,
            OFFER_DOMAIN,
        )
    }

    fn verify_checkpoint(&self, mut candidate: SyncState, device: &DeviceSecret) -> Result<()> {
        // Replay only the explicitly closed ancestry. Later edits stay in the
        // real journal as excluded drafts when the transfer eventually arrives.
        let closure = self.control.dependency_closure(&candidate)?;
        candidate.objects.retain(|hash, _| closure.contains(hash));
        let replay = Machine::replay(&candidate, device)?;
        replay.require_ready()?;
        if !replay.has_frontier(&self.control.closed_frontier)
            || replay.projection != self.control.decrypt_checkpoint(device)?
        {
            return Err(Error::CheckpointMismatch);
        }
        Ok(())
    }
}

impl Machine {
    pub fn control_intent_dependencies(state: &SyncState) -> Result<BTreeSet<Hash>> {
        let Some(PreparedChange::Control(control)) = &state.intent else {
            return Err(Error::PendingIntent);
        };
        control.verify_pending(state)?;
        control.dependency_closure(state)
    }

    /// Prepare without signing. Persist this exact control intent before making
    /// a transfer offer. The incoming controller must already be admitted Owner.
    pub fn prepare_control_transfer(
        state: &SyncState,
        device: &DeviceSecret,
        target: Hash,
    ) -> Result<PreparedChange> {
        state.require_idle()?;
        let replay = Self::replay(state, device)?;
        replay.require_ready()?;
        let previous = control_at(state, replay.membership)?;
        if previous.body.controller != device.id() {
            return Err(Error::InsufficientRole);
        }
        if replay.frontier.len() > MAX_PARENTS {
            return Err(Error::FrontierTooWide);
        }
        previous
            .prepare_successor(
                state,
                &replay,
                target,
                previous.body.members.clone(),
                ControlTransition::Transfer,
            )
            .map(PreparedChange::Control)
    }

    /// The offer signature has no successor authority. Until new-controller
    /// consent arrives this reserved intent cannot emit a control signature.
    pub fn control_transfer_offer(
        state: &SyncState,
        device: &DeviceSecret,
    ) -> Result<ControlTransfer> {
        let control = state.pending_control(ControlTransition::Transfer)?;
        if control.approval.is_some() {
            return Err(Error::UnauthorizedDevice);
        }
        let previous = control.verify_pending(state)?;
        if previous.body.controller != device.id() {
            return Err(Error::InsufficientRole);
        }
        let signed = crypto::sign(
            (state.descriptor(), control.clone()),
            &device.signer(),
            OFFER_DOMAIN,
        )?;
        let closure = control.dependency_closure(state)?;
        let offer = ControlTransfer {
            descriptor: state.descriptor(),
            control: control.clone(),
            signature: signed.signature,
            objects: closure
                .into_iter()
                .map(|h| state.objects[&h].clone())
                .collect(),
        };
        offer.bytes()?;
        Ok(offer)
    }

    pub fn accept_control_transfer(
        state: &SyncState,
        device: &DeviceSecret,
        offer: &ControlTransfer,
    ) -> Result<Signed<ControlApproval>> {
        state.require_idle()?;
        offer.verify_recipient(state, device)?;
        let candidate = offer.receive_objects(state)?;
        let previous = offer.control.verify_pending(&candidate)?;
        offer.verify_signature(&previous.body)?;
        offer.verify_checkpoint(candidate, device)?;
        offer
            .control
            .sign_approval(state, &device.signer(), device.id(), TRANSFER_DOMAIN)
    }

    pub fn approve_control_transfer(
        state: &mut SyncState,
        approval: Signed<ControlApproval>,
    ) -> Result<()> {
        let control = state.pending_control(ControlTransition::Transfer)?;
        let previous = control.verify_pending(state)?;
        let mut updated = control.clone();
        updated.set_approval(approval);
        updated.verify_approval(Some(&previous), state)?;
        state.intent = Some(PreparedChange::Control(updated));
        Ok(())
    }
}

impl Control {
    fn dependency_closure(&self, state: &SyncState) -> Result<BTreeSet<Hash>> {
        let mut pending: Vec<_> = self
            .epoch
            .previous
            .into_iter()
            .chain(self.closed_frontier.iter().copied())
            .collect();
        let mut closure = BTreeSet::new();
        while let Some(hash) = pending.pop() {
            if !closure.insert(hash) {
                continue;
            }
            if closure.len() > MAX_OBJECTS {
                return Err(Error::ResourceBudgetExceeded);
            }
            let object: Object =
                decode(&state.objects.get(&hash).ok_or(Error::MissingDependency)?.0)?;
            pending.extend(object.dependencies());
        }
        Ok(closure)
    }
}
