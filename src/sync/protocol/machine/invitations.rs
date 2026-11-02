//! Addressed consent and admission proofs. Verification uses authenticated public
//! control metadata; ciphertext history and private epoch keys are unnecessary.

use crate::sync::protocol::{
    Admission, AdmissionConfirmation, AdmissionReceipt, Blob, Certificate, Counter, DeviceSecret,
    Error, Hash, Invitation, InviteOffer, JoinRequest, Object, Result, Role, Signed, canonical_eq,
    crypto, decode,
    invitation::{JOIN_DOMAIN, OFFER_DOMAIN, RECEIPT_DOMAIN, offer_hash},
    machine::{
        Machine,
        history::{ControlHistory, with_history},
        protocol::Control,
        state::{PreparedChange, SpaceDescriptor, SyncState},
    },
    wire::sorted_unique,
};
use std::collections::{BTreeMap, BTreeSet};

impl Machine {
    pub fn controller(state: &SyncState) -> Result<Hash> {
        with_history(state, |history| Ok(history.tip().1.body.controller))
    }

    pub fn was_member(state: &SyncState, device: Hash) -> Result<bool> {
        with_history(state, |history| Ok(was_admitted(&history.chain, device)))
    }

    /// Creates an addressed offer under the current controller. Hosts persist it
    /// before export and enforce expiry/cancellation when consuming a response.
    pub fn offer_invitation(
        state: &SyncState,
        device: &DeviceSecret,
        recipient: Signed<Certificate>,
        role: Role,
        expires_at: Option<u64>,
    ) -> Result<Invitation> {
        Self::offer_named_invitation(state, device, None, recipient, role, expires_at)
    }

    /// Include the authenticated display name used by the provider adapter.
    pub fn offer_named_invitation(
        state: &SyncState,
        device: &DeviceSecret,
        space_name: Option<&str>,
        recipient: Signed<Certificate>,
        role: Role,
        expires_at: Option<u64>,
    ) -> Result<Invitation> {
        if space_name.is_some_and(|name| {
            name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control)
        }) {
            return Err(Error::InvalidInvitation);
        }
        state.require_idle()?;
        let replay = Self::replay_state(state, device)?;

        replay.require_ready()?;
        with_history(state, |history| {
            let (anchor, current) = history.tip();
            let device_id = device.id();

            if current.body.controller != device_id {
                return Err(Error::InsufficientRole);
            }
            validate_recipient(&recipient, role, &current.body)?;

            if was_admitted(&history.chain, recipient.body.device) {
                return Err(Error::RetiredDevice);
            }
            let offer = InviteOffer {
                descriptor: state.descriptor(),
                space_name: space_name.map(str::to_owned),
                anchor,
                id: Hash(*crypto::random32()?),
                author: device_id,
                recipient,
                role,
                expires_at: expires_at.map(Counter),
            };

            let result = Invitation {
                offer: crypto::sign(offer, &device.signer(), OFFER_DOMAIN)?,
                history: encode_history(history)?,
            };
            result.bytes()?;
            Ok(result)
        })
    }

    /// Authenticate `expected` through an independent channel before this call.
    /// Display names, transport access and the artifact itself cannot pin trust.
    pub fn verify_invitation(invitation: &Invitation, expected: &SpaceDescriptor) -> Result<()> {
        let state = metadata_state(expected, &invitation.history)?;
        with_history(&state, |history| {
            if history.tip().0 != invitation.anchor() || history.chain.len() != state.objects.len()
            {
                return Err(Error::InvalidInvitation);
            }
            verify_offer(&invitation.offer, &state, &history.chain)?;
            Ok(())
        })
    }

    /// Sign exact recipient consent. Hosts save it before handing it to the owner;
    /// retries must reuse the stored response rather than allocate a new challenge.
    pub fn accept_invitation(
        invitation: &Invitation,
        expected: &SpaceDescriptor,
        device: &DeviceSecret,
        certificate: &Signed<Certificate>,
    ) -> Result<Admission> {
        Self::verify_invitation(invitation, expected)?;

        if certificate.hash()? != invitation.recipient().hash()?
            || !device.matches_certificate(&certificate.body)
        {
            return Err(Error::UnauthorizedDevice);
        }
        let request = JoinRequest {
            offer: offer_hash(&invitation.offer)?,
            recipient: device.id(),
            challenge: Hash(*crypto::random32()?),
        };

        Ok(Admission {
            offer: invitation.offer.clone(),
            request: crypto::sign(request, &device.signer(), JOIN_DOMAIN)?,
        })
    }

    /// Owner-authorized admission changes the epoch and binds the exact signed
    /// consent into its control record. This does not sign until intent completion.
    pub fn prepare_admission(
        state: &SyncState,
        device: &DeviceSecret,
        admission: Admission,
    ) -> Result<PreparedChange> {
        Self::prepare_admissions(state, device, vec![admission])
    }

    /// Batch consent into one atomic membership transition and key rotation.
    pub fn prepare_admissions(
        state: &SyncState,
        device: &DeviceSecret,
        mut admissions: Vec<Admission>,
    ) -> Result<PreparedChange> {
        let mut members = Self::members(state, device)?;
        admissions.sort_by_key(|proof| proof.recipient().body.device);
        for admission in &admissions {
            if members.iter().any(|(certificate, _)| {
                certificate.body.device == admission.recipient().body.device
            }) {
                return Err(Error::InvitationUsed);
            }
            members.push((admission.recipient().clone(), admission.role()));
        }
        Self::prepare_membership_with_admissions(state, device, members, admissions)
    }

    pub fn admission_receipt(
        state: &SyncState,
        device: &DeviceSecret,
        admission: &Admission,
    ) -> Result<AdmissionReceipt> {
        state.require_idle()?;
        if !Self::replay_state(state, device)?.ready {
            return Err(Error::MissingDependency);
        }
        with_history(state, |history| {
            find_admission(history, admission)?.ok_or(Error::InvalidInvitation)?;
            let (control, current) = history.tip();
            let device_id = device.id();
            if current.body.controller != device_id {
                return Err(Error::InsufficientRole);
            }
            if !current
                .body
                .members
                .iter()
                .any(|m| m.certificate.body.device == admission.recipient().body.device)
            {
                return Err(Error::RetiredDevice);
            }
            let confirmation = AdmissionConfirmation {
                descriptor: state.descriptor(),
                invitation: admission.id(),
                acceptance: admission.hash()?,
                control,
                author: device_id,
            };
            let receipt = AdmissionReceipt {
                confirmation: crypto::sign(confirmation, &device.signer(), RECEIPT_DOMAIN)?,
                history: encode_history(history)?,
            };
            receipt.bytes()?;
            Ok(receipt)
        })
    }

    /// Locate an exact consent proof in verified control history. A different
    /// response for the same invitation is never equivalent to that proof.
    pub fn admission_control(state: &SyncState, admission: &Admission) -> Result<Option<Hash>> {
        with_history(state, |history| find_admission(history, admission))
    }

    /// Verify final admission, pin the current signed head, and bootstrap only
    /// from public metadata. No old device key, counter or snapshot is imported.
    pub fn join_admission(
        invitation: &Invitation,
        admission: &Admission,
        receipt: &AdmissionReceipt,
        expected: &SpaceDescriptor,
        device: &DeviceSecret,
    ) -> Result<SyncState> {
        Self::verify_invitation(invitation, expected)?;

        if receipt.id() != invitation.id()
            || admission.id() != invitation.id()
            || !canonical_eq(&admission.offer, &invitation.offer)?
            || !device.matches_certificate(&admission.recipient().body)
        {
            return Err(Error::InvalidInvitation);
        }
        let mut state = metadata_state(expected, &receipt.history)?;
        let tip = with_history(&state, |history| {
            if history.chain.len() != state.objects.len() {
                return Err(Error::InvalidInvitation);
            }
            let (tip, current) = history.tip();
            let confirmation = &receipt.confirmation.body;
            if confirmation.descriptor != *expected
                || confirmation.control != tip
                || confirmation.acceptance != admission.hash()?
                || confirmation.author != current.body.controller
            {
                return Err(Error::InvalidInvitation);
            }
            let controller = current
                .body
                .members
                .iter()
                .find(|m| {
                    m.certificate.body.device == current.body.controller && m.role == Role::Owner
                })
                .ok_or(Error::InsufficientRole)?;
            crypto::verify(
                &receipt.confirmation,
                &controller.certificate.body.signing_key,
                RECEIPT_DOMAIN,
            )?;
            let found = history
                .chain
                .iter()
                .flat_map(|(_, c)| &c.body.admissions)
                .find(|proof| proof.id() == admission.id())
                .ok_or(Error::InvalidInvitation)?;

            if !canonical_eq(found, admission)? {
                return Err(Error::InvalidInvitation);
            }
            let device_id = device.id();
            let member = current
                .body
                .members
                .iter()
                .find(|m| m.certificate.body.device == device_id)
                .ok_or(Error::RetiredDevice)?;

            if member.certificate.hash()? != admission.recipient().hash()? {
                return Err(Error::RetiredDevice);
            }
            Ok((tip, member.role))
        })?;
        state.required_control = Some(tip.0);
        // Writers can relay owner-signed metadata received out of band before
        // publishing their own events. Readers need no provider PUT permission.
        if matches!(tip.1, Role::Owner | Role::Editor) {
            state.outbox.extend(state.objects.keys().copied());
        }
        Ok(state)
    }
}

pub(super) fn verify_admissions(
    body: &Control,
    previous: Option<&Signed<Control>>,
    state: &SyncState,
    ancestors: &[(Hash, &Signed<Control>)],
) -> Result<()> {
    let Some(previous) = previous else {
        return if body.admissions.is_empty() && body.members.len() == 1 {
            Ok(())
        } else {
            Err(Error::ConsentRequired)
        };
    };

    let mut added = BTreeMap::new();

    for member in &body.members {
        let previous_member = previous
            .body
            .members
            .iter()
            .find(|m| m.certificate.body.device == member.certificate.body.device);

        if let Some(old) = previous_member
            && old.certificate.hash()? == member.certificate.hash()?
        {
            continue;
        }
        if was_admitted(ancestors, member.certificate.body.device) {
            return Err(Error::RetiredDevice);
        }
        if body.is_recovery()
            && member.certificate.body.device == body.controller
            && member.certificate.body.user == body.owner
        {
            continue;
        }
        added.insert(member.certificate.body.device, member);
    }

    if added.len() != body.admissions.len() {
        return Err(Error::ConsentRequired);
    }
    if !sorted_unique(
        body.admissions
            .iter()
            .map(|proof| proof.recipient().body.device),
    ) {
        return Err(Error::InvalidInvitation);
    }

    let mut consumed: BTreeSet<_> = ancestors
        .iter()
        .flat_map(|(_, c)| c.body.admissions.iter().map(Admission::id))
        .collect();

    for proof in &body.admissions {
        if !consumed.insert(proof.id()) {
            return Err(Error::InvitationUsed);
        }
        let member = added
            .get(&proof.recipient().body.device)
            .ok_or(Error::InvalidInvitation)?;

        if member.role != proof.role() || member.certificate.hash()? != proof.recipient().hash()? {
            return Err(Error::InvalidInvitation);
        }
        verify_offer(&proof.offer, state, ancestors)?;

        if proof.offer.body.author != previous.body.controller {
            return Err(Error::InvalidInvitation);
        }
        if previous.body.members.iter().any(|m| {
            m.certificate.body.user == member.certificate.body.user && m.role != member.role
        }) {
            return Err(Error::InsufficientRole);
        }
        proof.verify_consent()?;
    }

    Ok(())
}

fn verify_offer(
    offer: &Signed<InviteOffer>,
    state: &SyncState,
    ancestors: &[(Hash, &Signed<Control>)],
) -> Result<()> {
    let body = &offer.body;

    if body.descriptor != state.descriptor()
        || body.space_name.as_ref().is_some_and(|name| {
            name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control)
        })
    {
        return Err(Error::InvalidInvitation);
    }
    let index = ancestors
        .iter()
        .position(|(hash, _)| *hash == body.anchor)
        .ok_or(Error::InvalidInvitation)?;
    let anchor = &ancestors[index].1.body;

    if body.author != anchor.controller {
        return Err(Error::InsufficientRole);
    }
    let author = anchor
        .members
        .iter()
        .find(|m| m.certificate.body.device == body.author && m.role == Role::Owner)
        .ok_or(Error::InsufficientRole)?;
    crypto::verify(offer, &author.certificate.body.signing_key, OFFER_DOMAIN)?;
    validate_recipient(&body.recipient, body.role, anchor)?;

    if was_admitted(&ancestors[..=index], body.recipient.body.device) {
        return Err(Error::RetiredDevice);
    }
    Ok(())
}

fn validate_recipient(
    certificate: &Signed<Certificate>,
    role: Role,
    anchor: &Control,
) -> Result<()> {
    certificate.verify()?;

    if (role == Role::Owner) != (certificate.body.user == anchor.owner)
        || anchor.members.iter().any(|member| {
            member.certificate.body.user == certificate.body.user && member.role != role
        })
    {
        return Err(Error::InsufficientRole);
    }
    Ok(())
}

fn was_admitted(ancestors: &[(Hash, &Signed<Control>)], device: Hash) -> bool {
    ancestors.iter().any(|(_, c)| {
        c.body
            .members
            .iter()
            .any(|m| m.certificate.body.device == device)
    })
}

fn find_admission(history: &ControlHistory<'_>, admission: &Admission) -> Result<Option<Hash>> {
    for (hash, control) in &history.chain {
        if let Some(found) = control
            .body
            .admissions
            .iter()
            .find(|proof| proof.id() == admission.id())
        {
            if !canonical_eq(found, admission)? {
                return Err(Error::InvitationUsed);
            }
            return Ok(Some(*hash));
        }
    }
    Ok(None)
}

fn encode_history(history: &ControlHistory<'_>) -> Result<Vec<Blob>> {
    history
        .chain
        .iter()
        .map(|(_, control)| Object::Control((*control).clone()).bytes().map(Blob))
        .collect()
}

fn metadata_state(descriptor: &SpaceDescriptor, bytes: &[Blob]) -> Result<SyncState> {
    if bytes.len() > crate::sync::protocol::MAX_OBJECTS
        || bytes.iter().map(|blob| blob.0.len()).sum::<usize>()
            > crate::sync::protocol::MAX_CACHE_BYTES
    {
        return Err(Error::ResourceBudgetExceeded);
    }
    let mut state = Machine::join(
        descriptor.space,
        descriptor.namespace,
        descriptor.genesis,
        descriptor.owner_root,
    );

    for blob in bytes {
        if !matches!(decode::<Object>(&blob.0)?, Object::Control(_)) {
            return Err(Error::InvalidInvitation);
        }
        let hash = Object::hash_bytes(&blob.0);

        if !Machine::receive(&mut state, hash, blob.0.clone())? {
            return Err(Error::InvalidInvitation);
        }
    }

    Ok(state)
}
