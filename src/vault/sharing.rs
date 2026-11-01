//! Durable invitation workflows. Every returned artifact is saved with its exact
//! signature first; membership changes use the ordinary intent/outbox boundary.

use crate::{
    sync::protocol::{
        Admission, AdmissionReceipt, Certificate, Error, Hash, Invitation, Machine, Role, Signed,
        SpaceDescriptor, canonical,
    },
    vault::{
        InvitationStatus, Result, Vault,
        profile::{AcceptedInvitation, IssuedInvitation, SharingState},
    },
};

impl Vault {
    /// Read-only inspection before the UI asks the controller to admit a device.
    /// Actual admission revalidates current history and durable local policy.
    pub fn verify_invitation_response(&self, response: &Admission, now: u64) -> Result<()> {
        let record = self
            .profile
            .as_ref()
            .and_then(|p| p.sharing.issued.get(&response.id()))
            .ok_or(Error::InvalidInvitation)?;
        if canonical(&record.invitation.offer)? != canonical(&response.offer)? {
            return Err(Error::InvalidInvitation.into());
        }
        match record.status(now) {
            InvitationStatus::Cancelled => return Err(Error::InvitationCancelled.into()),
            InvitationStatus::Expired => return Err(Error::InvitationExpired.into()),
            _ => {}
        }
        response.verify_consent()?;
        Ok(())
    }

    pub fn verify_join_receipt(&self, receipt: &AdmissionReceipt) -> Result<SpaceDescriptor> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let record = profile
            .sharing
            .accepted
            .get(&receipt.id())
            .ok_or(Error::InvalidInvitation)?;
        let state = Machine::join_admission(
            &record.invitation,
            &record.admission,
            receipt,
            record.invitation.descriptor(),
            &profile.sync_secret(),
        )?;
        if let Some(previous) = &profile.sync
            && previous.descriptor() == state.descriptor()
            && let Some(anchor) = previous.required_control
            && !state.objects.contains_key(&anchor)
        {
            return Err(Error::MissingDependency.into());
        }
        Ok(state.descriptor())
    }

    pub fn space_members(&self) -> Result<Vec<(Signed<Certificate>, Role)>> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Ok(Machine::members(
            profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?,
            &profile.sync_secret(),
        )?)
    }

    pub fn space_controller(&self) -> Result<Hash> {
        Ok(Machine::controller(
            self.sync_state().ok_or(Error::UnauthorizedDevice)?,
        )?)
    }

    pub fn issued_invitations(&self, now: u64) -> Vec<crate::vault::IssuedInvitationInfo> {
        self.profile
            .as_ref()
            .map(|profile| {
                profile
                    .sharing
                    .issued
                    .values()
                    .map(|record| crate::vault::IssuedInvitationInfo {
                        id: record.invitation.id(),
                        user: record.invitation.recipient().body.user,
                        device: record.invitation.recipient().body.device,
                        role: record.invitation.role(),
                        status: record.status(now),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn accepted_invitations(&self) -> Vec<crate::vault::AcceptedInvitationInfo> {
        self.profile
            .as_ref()
            .map(|profile| {
                profile
                    .sharing
                    .accepted
                    .values()
                    .map(|record| crate::vault::AcceptedInvitationInfo {
                        id: record.invitation.id(),
                        role: record.invitation.role(),
                        joined: record.joined.is_some(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Re-export a durable offer without allocating a new invitation ID.
    pub fn export_invitation(&mut self, id: Hash) -> Result<Invitation> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        let invitation = self
            .profile
            .as_ref()
            .and_then(|p| p.sharing.issued.get(&id))
            .ok_or(Error::InvalidInvitation)?
            .invitation
            .clone();
        self.save_inner(false)?;
        Ok(invitation)
    }

    /// Resume sharing after restart without producing a different signed response.
    pub fn export_invitation_response(&mut self, id: Hash) -> Result<Admission> {
        self.ensure_profile_writable()?;
        self.ensure_sharing_view()?;
        let admission = self
            .profile
            .as_ref()
            .and_then(|p| p.sharing.accepted.get(&id))
            .ok_or(Error::InvalidInvitation)?
            .admission
            .clone();
        self.save_inner(false)?;
        Ok(admission)
    }

    /// The caller independently verifies the intended recipient certificate.
    /// Expiry uses an explicit owner-side clock; replay never consults time.
    pub fn issue_invitation(
        &mut self,
        recipient: Signed<Certificate>,
        role: Role,
        expires_at: Option<u64>,
    ) -> Result<Invitation> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let state = profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let recipient_bytes = canonical(&recipient)?;
        let existing = profile.sharing.issued.values().find(|record| {
            !record.cancelled
                && record.admission.is_none()
                && record.invitation.descriptor() == &state.descriptor()
                && canonical(record.invitation.recipient())
                    .is_ok_and(|bytes| bytes == recipient_bytes)
                && record.invitation.role() == role
                && record.invitation.expires_at() == expires_at
        });
        let invitation = match existing {
            Some(record) => record.invitation.clone(),
            None => {
                Self::sharing_capacity(&profile.sharing)?;
                Machine::offer_named_invitation(
                    state,
                    &profile.sync_secret(),
                    Some(&profile.space_name),
                    recipient,
                    role,
                    expires_at,
                )?
            }
        };
        self.update_sharing(|sharing| {
            sharing
                .issued
                .entry(invitation.id())
                .or_insert_with(|| IssuedInvitation {
                    invitation: invitation.clone(),
                    cancelled: false,
                    admission: None,
                    receipt: None,
                });
        })?;
        self.save_inner(false)?;
        Ok(invitation)
    }

    pub fn invitation_status(&self, id: Hash, now: u64) -> Result<InvitationStatus> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Ok(profile
            .sharing
            .issued
            .get(&id)
            .ok_or(Error::InvalidInvitation)?
            .status(now))
    }

    /// Cancellation is a durable controller-local policy for unconsumed offers.
    /// Already reserved admissions finish through recovery and require removal.
    pub fn cancel_invitation(&mut self, id: Hash) -> Result<()> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        let record = profile
            .sharing
            .issued
            .get_mut(&id)
            .ok_or(Error::InvalidInvitation)?;
        if record.admission.is_some() {
            return Err(Error::InvitationUsed.into());
        }
        record.cancelled = true;
        self.save_inner(false)
    }

    /// Release a terminal local record. Signed admission proofs remain in the
    /// shared journal; forgetting disables this installation's artifact retries.
    pub fn forget_invitation(&mut self, id: Hash, now: u64) -> Result<()> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        let record = profile
            .sharing
            .issued
            .get(&id)
            .ok_or(Error::InvalidInvitation)?;
        if matches!(
            record.status(now),
            InvitationStatus::Pending | InvitationStatus::Admitting
        ) {
            return Err(Error::PendingIntent.into());
        }
        profile.sharing.issued.remove(&id);
        self.save_inner(false)
    }

    /// Authenticate `expected` independently. Accepting never imports private
    /// keys or switches spaces; the exact response survives retry and restart.
    pub fn accept_invitation(
        &mut self,
        invitation: Invitation,
        expected: &SpaceDescriptor,
    ) -> Result<Admission> {
        self.ensure_profile_writable()?;
        self.ensure_sharing_view()?;
        Machine::verify_invitation(&invitation, expected)?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let admission = if let Some(record) = profile.sharing.accepted.get(&invitation.id()) {
            if canonical(&record.invitation)? != canonical(&invitation)? {
                return Err(Error::InvalidInvitation.into());
            }
            record.admission.clone()
        } else {
            self.ensure_joinable(expected)?;
            Self::sharing_capacity(&profile.sharing)?;
            Machine::accept_invitation(
                &invitation,
                expected,
                &profile.sync_secret(),
                profile
                    .certificate
                    .as_ref()
                    .ok_or(Error::UnauthorizedDevice)?,
            )?
        };
        self.update_sharing(|sharing| {
            sharing
                .accepted
                .entry(invitation.id())
                .or_insert_with(|| AcceptedInvitation {
                    invitation,
                    admission: admission.clone(),
                    joined: None,
                });
        })?;
        self.save_inner(false)?;
        Ok(admission)
    }

    /// Consume only a locally issued, live offer. Repeating the exact request
    /// resumes a reserved transition or returns its stored signed confirmation.
    pub fn admit_invitation(&mut self, admission: Admission, now: u64) -> Result<AdmissionReceipt> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        self.recover_sync()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let record = profile
            .sharing
            .issued
            .get(&admission.id())
            .ok_or(Error::InvalidInvitation)?;
        if canonical(&record.invitation.offer)? != canonical(&admission.offer)? {
            return Err(Error::InvalidInvitation.into());
        }
        match record.status(now) {
            InvitationStatus::Cancelled => return Err(Error::InvitationCancelled.into()),
            InvitationStatus::Expired => return Err(Error::InvitationExpired.into()),
            _ => {}
        }
        if let Some(stored) = &record.admission
            && stored.hash()? != admission.hash()?
        {
            return Err(Error::InvitationUsed.into());
        }
        if Machine::admission_control(profile.sync.as_ref().unwrap(), &admission)?.is_none() {
            let prepared = self.prepare_sync_command(|state, device| {
                Machine::prepare_admission(state, device, admission.clone())
            })?;
            self.update_sharing(|sharing| {
                sharing.issued.get_mut(&admission.id()).unwrap().admission =
                    Some(admission.clone());
            })?;
            self.commit_prepared(prepared)?;
        }
        let profile = self.profile.as_ref().unwrap();
        // Verify current membership even when an old receipt is cached.
        let current = Machine::admission_receipt(
            profile.sync.as_ref().unwrap(),
            &profile.sync_secret(),
            &admission,
        )?;
        let receipt = profile.sharing.issued[&admission.id()]
            .receipt
            .clone()
            .unwrap_or(current);
        self.update_sharing(|sharing| {
            sharing.issued.get_mut(&admission.id()).unwrap().receipt = Some(receipt.clone());
        })?;
        self.save_inner(false)?;
        Ok(receipt)
    }

    /// Complete a previously accepted invitation. A receipt pins its signed
    /// control head, including a recovery profile's independent minimum anchor.
    pub fn join_invitation(&mut self, receipt: AdmissionReceipt) -> Result<()> {
        self.ensure_profile_writable()?;
        self.ensure_sharing_view()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let record = profile
            .sharing
            .accepted
            .get(&receipt.id())
            .ok_or(Error::InvalidInvitation)?;
        let expected = record.invitation.descriptor();
        let device = profile.sync_secret();
        let state = Machine::join_admission(
            &record.invitation,
            &record.admission,
            &receipt,
            expected,
            &device,
        )?;
        if let Some(control) = record.joined {
            if control != receipt.control() {
                return Err(Error::InvitationUsed.into());
            }
            return self.save_inner(false);
        }
        self.ensure_joinable(expected)?;
        if let Some(previous) = &profile.sync
            && previous.descriptor() == *expected
            && let Some(anchor) = previous.required_control
            && !state.objects.contains_key(&anchor)
        {
            return Err(Error::MissingDependency.into());
        }
        let replay = Machine::replay_state(&state, &device)?;
        let space_name = record.invitation.space_name().map(str::to_owned);
        self.entries = replay.projection.entries();
        self.folders = replay.projection.folders();
        let profile = self.profile.as_mut().unwrap();
        if let Some(name) = space_name {
            profile.space_name = name;
        }
        profile.space.space_id = state.space.0;
        profile.space.namespace = state.namespace.0;
        profile.sync = Some(state);
        profile
            .sharing
            .accepted
            .get_mut(&receipt.id())
            .unwrap()
            .joined = Some(receipt.control());
        self.save_inner(false)
    }

    /// Explicitly confirm a later control head for a recipient that has not yet
    /// joined. Ordinary request retries retain their original receipt bytes.
    pub fn refresh_invitation_receipt(&mut self, id: Hash) -> Result<AdmissionReceipt> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        self.recover_sync()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let admission = profile
            .sharing
            .issued
            .get(&id)
            .and_then(|record| record.admission.as_ref())
            .ok_or(Error::InvalidInvitation)?;
        let receipt = Machine::admission_receipt(
            profile.sync.as_ref().unwrap(),
            &profile.sync_secret(),
            admission,
        )?;
        self.update_sharing(|sharing| {
            sharing.issued.get_mut(&id).unwrap().receipt = Some(receipt.clone());
        })?;
        self.save_inner(false)?;
        Ok(receipt)
    }

    fn sharing_capacity(state: &SharingState) -> Result<()> {
        if state.issued.len() + state.accepted.len() >= SharingState::MAX_RECORDS {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        Ok(())
    }

    fn update_sharing(&mut self, update: impl FnOnce(&mut SharingState)) -> Result<()> {
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        let mut candidate = profile.sharing.clone();
        update(&mut candidate);
        if !candidate.validate() {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        profile.sharing = candidate;
        profile.enable_sharing();
        Ok(())
    }

    fn ensure_sharing_view(&self) -> Result<()> {
        match self.ensure_sync_projection() {
            Err(crate::vault::VaultError::Sync(Error::MissingDependency))
                if self.entries.is_empty()
                    && self.folders.is_empty()
                    && self
                        .sync_state()
                        .is_some_and(|s| s.intent.is_none() && s.outbox.is_empty()) =>
            {
                Ok(())
            }
            other => other,
        }
    }

    fn ensure_joinable(&self, expected: &SpaceDescriptor) -> Result<()> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let state = profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;
        if !self.entries.is_empty()
            || !self.folders.is_empty()
            || state.intent.is_some()
            || profile
                .sharing
                .accepted
                .values()
                .any(|record| record.joined.is_some())
            || (state.objects.len() != 1
                && !(state.descriptor() == *expected
                    && state.required_control.is_some()
                    && state.outbox.is_empty()))
        {
            return Err(Error::InvalidTransaction.into());
        }
        Ok(())
    }
}
