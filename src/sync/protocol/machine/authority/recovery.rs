//! Owner-root authorization of a fresh controller at a pinned content boundary.

use crate::sync::protocol::machine::{
    Machine,
    authority::RECOVERY_DOMAIN,
    commands::control_at,
    protocol::{ControlTransition, Member, RecoveryAnchor},
    state::{PreparedChange, Replay, SyncState},
};

use crate::sync::protocol::{
    Certificate, DeviceSecret, Error, Result, Role, Signed, canonical, decode,
};
use ed25519_dalek::SigningKey;

impl RecoveryAnchor {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        canonical(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode(bytes)
    }

    fn verify_replay(&self, state: &SyncState, replay: &Replay) -> Result<()> {
        replay.require_ready()?;
        if state.descriptor() != self.descriptor
            || replay.membership != self.control
            || !replay.has_frontier(&self.frontier)
        {
            return Err(Error::MissingDependency);
        }
        Ok(())
    }
}

impl Machine {
    /// Stable local signer-journal claim; excludes the approval signature itself.
    pub fn root_recovery_claim(state: &SyncState) -> Result<(crate::sync::protocol::Hash, u64)> {
        let control = state.pending_control(ControlTransition::Recovery)?;
        control.verify_pending(state)?;
        Ok((control.statement_hash()?, control.epoch.number.0))
    }

    pub fn has_root_recovery_claim(
        state: &SyncState,
        claim: crate::sync::protocol::Hash,
    ) -> Result<bool> {
        crate::sync::protocol::machine::history::with_history(state, |history| {
            for (_, control) in &history.chain {
                if control.body.is_recovery() && control.body.statement_hash()? == claim {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    pub fn recovery_anchor(state: &SyncState, device: &DeviceSecret) -> Result<RecoveryAnchor> {
        let replay = Self::replay_state(state, device)?;
        replay.require_ready()?;
        Ok(RecoveryAnchor {
            descriptor: state.descriptor(),
            control: replay.membership,
            frontier: replay.frontier.into_iter().collect(),
        })
    }

    /// The owner root alone cannot recreate content. The source must replay the
    /// exact independently authenticated control and data frontier supplied here.
    pub fn prepare_control_recovery(
        source: &SyncState,
        source_device: &DeviceSecret,
        certificate: Signed<Certificate>,
        anchor: &RecoveryAnchor,
    ) -> Result<PreparedChange> {
        source.require_idle()?;
        certificate.verify()?;
        let replay = Self::replay_state(source, source_device)?;
        anchor.verify_replay(source, &replay)?;
        let previous = control_at(source, replay.membership)?;
        if certificate.body.root_key != source.owner_root {
            return Err(Error::UnauthorizedDevice);
        }
        let target = certificate.body.device;
        let mut members: Vec<_> = previous
            .body
            .members
            .iter()
            .filter(|m| {
                m.certificate.body.device != previous.body.controller
                    && m.certificate.body.device != target
            })
            .cloned()
            .collect();
        members.push(Member {
            certificate,
            role: Role::Owner,
        });
        members.sort_by_key(|m| m.certificate.body.device);
        previous
            .prepare_successor(
                source,
                &replay,
                target,
                members,
                ControlTransition::Recovery,
            )
            .map(PreparedChange::Control)
    }

    /// Only after the unsigned recovery intent is durably reserved. Persist the
    /// resulting root approval before completing the device signature.
    pub fn approve_control_recovery(
        state: &mut SyncState,
        device: &DeviceSecret,
        root: &SigningKey,
    ) -> Result<()> {
        if root.verifying_key().to_bytes() != state.owner_root {
            return Err(Error::UnauthorizedDevice);
        }
        let control = state.pending_control(ControlTransition::Recovery)?;
        if control.controller != device.id() {
            return Err(Error::UnauthorizedDevice);
        }
        control.verify_pending(state)?;
        control.decrypt_checkpoint(device)?.validate()?;
        let approval = control.sign_approval(state, root, control.owner, RECOVERY_DOMAIN)?;
        let mut approved = control.clone();
        approved.set_approval(approval);
        state.intent = Some(PreparedChange::Control(approved));
        Ok(())
    }
}
