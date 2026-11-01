//! Explicit controller transfer and root-authorized recovery. Detached approvals
//! bind the full prepared statement; the final device signature covers the
//! approval too. No root or incoming-controller approval authorizes itself.

use super::{
    commands::capacity,
    history::with_history,
    membership::{build_control, epoch_key, validate_control_body},
    profile::{PROFILE, Profile},
    protocol::{Control, ControlApproval, ControlTransition, Epoch, Member},
    state::{PreparedChange, Replay, SyncState},
};
use crate::core::sync::{
    Blob, Counter, DeviceSecret, Error, Hash, Object, Projection, Result, Role, Signed, canonical,
    crypto, decode,
};
use ed25519_dalek::SigningKey;

mod recovery;
mod transfer;

const APPROVAL_PROFILE: &str = "LBX-CONTROL-2";
const TRANSFER_DOMAIN: &[u8] = b"lockbox.sync.control-transfer-approval.v2\0";
const RECOVERY_DOMAIN: &[u8] = b"lockbox.sync.control-recovery-root.v2\0";
const RECOVERY_DEVICE_DOMAIN: &[u8] = b"lockbox.sync.control-recovery-device.v2\0";

impl Control {
    pub(super) fn signing_domain(&self) -> Result<&'static [u8]> {
        if self.is_recovery() {
            Ok(RECOVERY_DEVICE_DOMAIN)
        } else {
            Ok(Profile::parse(&self.epoch.profile)?.control_domain())
        }
    }

    fn statement_hash(&self) -> Result<Hash> {
        let mut unsigned = self.clone();
        unsigned.approval = None;
        Ok(Hash::digest(
            b"lockbox.sync.control-statement.v2\0",
            &canonical(&unsigned)?,
        ))
    }

    pub(super) fn pending_approval(&self) -> Option<ControlTransition> {
        self.transition.filter(|_| self.approval.is_none())
    }

    pub(super) fn signing_controller(&self, previous: &Self) -> Hash {
        if self.is_recovery() {
            self.controller
        } else {
            previous.controller
        }
    }

    fn controller_member(&self) -> Result<&Member> {
        self.members
            .iter()
            .find(|member| {
                member.certificate.body.device == self.controller && member.role == Role::Owner
            })
            .ok_or(Error::UnauthorizedDevice)
    }

    pub(super) fn verify_approval(
        &self,
        previous: Option<&Signed<Self>>,
        state: &SyncState,
    ) -> Result<()> {
        let Some(transition) = self.transition else {
            return if self.approval.is_none() {
                Ok(())
            } else {
                Err(Error::InvalidEncoding)
            };
        };
        let previous = previous.ok_or(Error::UnauthorizedDevice)?;
        let approval = self
            .approval
            .as_ref()
            .ok_or(Error::ControlApprovalRequired)?;
        let (key, author, domain) = match transition {
            ControlTransition::Transfer => (
                self.controller_member()?.certificate.body.signing_key,
                self.controller,
                TRANSFER_DOMAIN,
            ),
            ControlTransition::Recovery => (
                previous.body.controller_member()?.certificate.body.root_key,
                previous.body.owner,
                RECOVERY_DOMAIN,
            ),
        };
        approval.body.verify_binding(self, state, author)?;
        crypto::verify(approval, &key, domain)
    }

    fn sign_approval(
        &self,
        state: &SyncState,
        key: &SigningKey,
        author: Hash,
        domain: &[u8],
    ) -> Result<Signed<ControlApproval>> {
        crypto::sign(
            ControlApproval::for_control(self, state, author)?,
            key,
            domain,
        )
    }

    fn set_approval(&mut self, approval: Signed<ControlApproval>) {
        self.approval = Some(Box::new(approval));
    }

    fn verify_pending(&self, state: &SyncState) -> Result<Signed<Self>> {
        if let Some(fault) = state.fault {
            return Err(fault);
        }
        with_history(state, |history| {
            let (tip, previous) = history.tip();
            if self.epoch.previous != Some(tip)
                || state
                    .required_control
                    .is_some_and(|pin| !history.contains(pin))
            {
                return Err(Error::MissingDependency);
            }
            validate_control_body(self, Some(previous), state, &history.chain)?;
            Ok(previous.clone())
        })
    }

    fn reserve_approval_capacity(&self, state: &SyncState) -> Result<()> {
        // Account for the detached approval and final device signature before signing.
        let mut sized = self.clone();
        sized.set_approval(Signed {
            body: ControlApproval::for_control(self, state, self.controller)?,
            signature: Blob(vec![0; 64]),
        });
        let object = Object::Control(Signed {
            body: sized,
            signature: Blob(vec![0; 64]),
        });
        capacity(state, object.bytes()?.len())
    }

    fn decrypt_checkpoint(&self, device: &DeviceSecret) -> Result<Projection> {
        let key = epoch_key(self, device)?;
        let content_key = crypto::content_key(&key, self.epoch.space, self.epoch.context, true);
        let plaintext = crypto::decrypt(
            &content_key,
            &crypto::nonce(&self.checkpoint_nonce)?,
            &self.checkpoint,
            &canonical(&self.epoch)?,
        )?;
        decode(&plaintext)
    }
}

impl ControlApproval {
    fn for_control(control: &Control, state: &SyncState, author: Hash) -> Result<Self> {
        Ok(Self {
            profile: APPROVAL_PROFILE.into(),
            descriptor: state.descriptor(),
            statement: control.statement_hash()?,
            author,
        })
    }

    fn verify_binding(&self, control: &Control, state: &SyncState, author: Hash) -> Result<()> {
        if self.profile != APPROVAL_PROFILE
            || self.descriptor != state.descriptor()
            || self.statement != control.statement_hash()?
            || self.author != author
        {
            return Err(Error::UnauthorizedDevice);
        }
        Ok(())
    }
}

impl Signed<ControlApproval> {
    pub fn id(&self) -> Hash {
        self.body.statement
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        canonical(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode(bytes)
    }
}

impl super::Machine {
    pub fn pending_control_approval(state: &SyncState) -> Option<ControlTransition> {
        state.pending_control_approval()
    }
}

impl SyncState {
    fn pending_control_approval(&self) -> Option<ControlTransition> {
        match &self.intent {
            Some(PreparedChange::Control(control)) => control.pending_approval(),
            _ => None,
        }
    }

    fn pending_control(&self, transition: ControlTransition) -> Result<&Control> {
        let Some(PreparedChange::Control(control)) = &self.intent else {
            return Err(Error::PendingIntent);
        };
        if control.transition != Some(transition) {
            return Err(Error::UnauthorizedDevice);
        }
        Ok(control)
    }
}

impl Signed<Control> {
    fn prepare_successor(
        &self,
        state: &SyncState,
        replay: &Replay,
        target: Hash,
        members: Vec<Member>,
        transition: ControlTransition,
    ) -> Result<Control> {
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
        let mut control = build_control(
            epoch,
            self.body.owner,
            target,
            members,
            replay.frontier.iter().copied().collect(),
            &replay.projection,
            &*crypto::random32()?,
        )?;
        control.transition = Some(transition);
        control.reserve_approval_capacity(state)?;
        with_history(state, |history| {
            validate_control_body(&control, Some(self), state, &history.chain)
        })?;
        Ok(control)
    }
}
