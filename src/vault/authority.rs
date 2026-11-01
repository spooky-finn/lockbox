//! Durable control succession. Device private keys and signing counters never
//! cross profiles. Final signatures use the ordinary intent/outbox boundary.

use crate::{
    sync::protocol::{
        ControlApproval, ControlTransfer, ControlTransition, Error, Hash, Machine, RecoveryAnchor,
        Signed,
    },
    vault::{Result, Vault},
};
use secrecy::SecretString;

impl Vault {
    /// Authenticate this complete value independently, including its data frontier.
    pub fn recovery_anchor(&self) -> Result<RecoveryAnchor> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Ok(Machine::recovery_anchor(
            profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?,
            &profile.sync_secret(),
        )?)
    }

    pub fn pending_control_approval(&self) -> Option<ControlTransition> {
        Machine::pending_control_approval(self.sync_state()?)
    }

    pub fn begin_control_transfer(&mut self, target: Hash) -> Result<ControlTransfer> {
        self.ensure_exchange_committed()?;
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        if self.pending_control_approval() != Some(ControlTransition::Transfer) {
            let prepared = self.prepare_sync_command(|state, device| {
                Machine::prepare_control_transfer(state, device, target)
            })?;
            self.profile.as_mut().unwrap().enable_authority();
            self.commit_prepared(prepared)?;
        }
        self.save_inner(false)?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let offer = Machine::control_transfer_offer(
            profile.sync.as_ref().unwrap(),
            &profile.sync_secret(),
        )?;
        if offer.target() != target {
            return Err(Error::PendingIntent.into());
        }
        Ok(offer)
    }

    pub fn accept_control_transfer(
        &mut self,
        offer: &ControlTransfer,
    ) -> Result<Signed<ControlApproval>> {
        self.ensure_exchange_committed()?;
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let approval = Machine::accept_control_transfer(
            profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?,
            &profile.sync_secret(),
            offer,
        )?;
        if !profile.control_approvals.contains_key(&approval.id())
            && profile.control_approvals.len() >= 128
        {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        let profile = self.profile.as_mut().unwrap();
        profile.enable_authority();
        profile
            .control_approvals
            .insert(approval.id(), approval.clone());
        self.save_inner(false)?;
        Ok(approval)
    }

    pub fn finish_control_transfer(&mut self, approval: Signed<ControlApproval>) -> Result<()> {
        self.ensure_exchange_committed()?;
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        if Machine::control_transfer_completed(
            self.sync_state().ok_or(Error::UnauthorizedDevice)?,
            &approval,
        )? {
            return self.save_inner(false);
        }
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        Machine::approve_control_transfer(
            profile.sync.as_mut().ok_or(Error::UnauthorizedDevice)?,
            approval,
        )?;
        self.recover_sync()
    }

    pub fn cancel_control_transfer(&mut self) -> Result<()> {
        self.ensure_exchange_committed()?;
        self.ensure_profile_writable()?;
        if self.pending_control_approval() != Some(ControlTransition::Transfer) {
            return Err(Error::PendingIntent.into());
        }
        self.profile.as_mut().unwrap().sync.as_mut().unwrap().intent = None;
        self.save_inner(false)
    }

    /// Resume after interruption between fresh-file creation and root approval.
    pub fn approve_controller_recovery(&mut self, recovery_password: &SecretString) -> Result<()> {
        self.ensure_exchange_committed()?;
        self.ensure_profile_writable()?;
        if self.sync_state().is_some_and(|s| {
            Machine::pending_control_transition(s) == Some(ControlTransition::Recovery)
        }) && self.pending_control_approval().is_none()
        {
            return self.recover_sync();
        }
        if self.pending_control_approval() != Some(ControlTransition::Recovery) {
            return Err(Error::PendingIntent.into());
        }
        self.save_inner(false)?;
        let _root_lease = crate::vault::root_journal::reserve(self)?;
        self.profile
            .as_mut()
            .unwrap()
            .approve_control_recovery(recovery_password)?;
        self.recover_sync()
    }
}
