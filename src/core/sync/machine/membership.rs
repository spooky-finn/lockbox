//! Owner authority, roster validation, HPKE grants, and epoch checkpoints.

use super::super::{
    Blob, Certificate, DeviceSecret, Error, Hash, Object, Projection, Result, Role, Signed,
    canonical, crypto, wire::sorted_unique,
};
use super::{
    invitations::verify_admissions,
    protocol::{Control, ControlTransition, Epoch, Grant, Member},
    state::SyncState,
};
use crate::core::identity::Key;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub(super) fn grant_aad(epoch: &Epoch, certificate: &Signed<Certificate>) -> Result<Vec<u8>> {
    canonical(&(epoch, certificate.hash()?))
}

pub(super) fn epoch_key(control: &Control, device: &DeviceSecret) -> Result<Zeroizing<Key>> {
    let grant = control
        .grants
        .iter()
        .find(|g| g.device == device.id())
        .ok_or(Error::KeyUnavailable)?;
    let member = Roster(&control.members).member(device.id())?;
    device.open_grant(
        &grant.enc,
        &grant.ciphertext,
        &grant_aad(&control.epoch, &member.certificate)?,
    )
}

pub(super) fn build_control(
    epoch: Epoch,
    owner: Hash,
    controller: Hash,
    members: Vec<Member>,
    closed_frontier: Vec<Hash>,
    baseline: &Projection,
    key: &Key,
) -> Result<Control> {
    let encryption = EpochEncryption { epoch: &epoch, key };
    let grants = members
        .iter()
        .map(|member| encryption.grant(&member.certificate))
        .collect::<Result<_>>()?;
    let (checkpoint_nonce, checkpoint) = encryption.checkpoint(baseline)?;
    Ok(Control {
        epoch,
        owner,
        controller,
        members,
        closed_frontier,
        grants,
        checkpoint_nonce,
        checkpoint,
        admissions: vec![],
        transition: None,
        approval: None,
    })
}

pub(super) fn validate_control(
    control: &Signed<Control>,
    previous: Option<&Signed<Control>>,
    state: &SyncState,
    ancestors: &[(Hash, &Signed<Control>)],
) -> Result<()> {
    let key = validate_control_body(&control.body, previous, state, ancestors)?;
    control.body.verify_approval(previous, state)?;
    crypto::verify(control, &key, control.body.signing_domain())
}

pub(super) fn validate_control_body(
    body: &Control,
    previous: Option<&Signed<Control>>,
    state: &SyncState,
    ancestors: &[(Hash, &Signed<Control>)],
) -> Result<Key> {
    let key = ControlAuthority::new(body, state).signing_key(previous)?;
    verify_admissions(body, previous, state, ancestors)?;
    if body.approval.is_some() {
        body.verify_approval(previous, state)?;
    }
    Ok(key)
}

/// Recipient grants and checkpoint encryption share one immutable epoch scope.
struct EpochEncryption<'a> {
    epoch: &'a Epoch,
    key: &'a Key,
}

impl EpochEncryption<'_> {
    fn grant(&self, certificate: &Signed<Certificate>) -> Result<Grant> {
        certificate.verify()?;
        let (enc, ciphertext) = crypto::seal_grant(
            &certificate.body.hpke_key,
            self.key,
            &grant_aad(self.epoch, certificate)?,
        )?;
        Ok(Grant {
            device: certificate.body.device,
            enc,
            ciphertext,
        })
    }

    fn checkpoint(&self, baseline: &Projection) -> Result<(Blob, Blob)> {
        let key = crypto::content_key(self.key, self.epoch.space, self.epoch.context, true);
        let mut nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(|_| Error::Random)?;
        let plaintext = Zeroizing::new(canonical(baseline)?);
        let ciphertext = crypto::encrypt(&key, &nonce, &plaintext, &canonical(self.epoch)?)?;
        Ok((Blob(nonce.to_vec()), ciphertext))
    }
}

/// Owns canonical ordering, exact device lookup and user-wide role/device limits.
struct Roster<'a>(&'a [Member]);

impl<'a> Roster<'a> {
    const MAX_DEVICES: usize = 512;
    const MAX_USERS: usize = 128;
    const MAX_DEVICES_PER_USER: usize = 16;

    fn is_canonical(&self) -> bool {
        !self.0.is_empty()
            && self.0.len() <= Self::MAX_DEVICES
            && self
                .0
                .windows(2)
                .all(|pair| pair[0].certificate.body.device < pair[1].certificate.body.device)
    }

    fn member(&self, device: Hash) -> Result<&'a Member> {
        self.0
            .iter()
            .find(|member| member.certificate.body.device == device)
            .ok_or(Error::UnauthorizedDevice)
    }

    fn controller(&self, device: Hash) -> Result<&'a Member> {
        let member = self.member(device)?;

        if member.role != Role::Owner {
            return Err(Error::UnauthorizedDevice);
        }

        Ok(member)
    }

    fn verify(&self, owner: Hash, grants: &[Grant]) -> Result<()> {
        if !self.is_canonical() || grants.len() != self.0.len() {
            return Err(Error::InvalidEncoding);
        }

        let mut users = BTreeMap::new();

        for (member, grant) in self.0.iter().zip(grants) {
            member.certificate.verify()?;
            let certificate = &member.certificate.body;

            if (member.role == Role::Owner) != (certificate.user == owner)
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

            if user.1 > Self::MAX_DEVICES_PER_USER {
                return Err(Error::ResourceBudgetExceeded);
            }
        }

        if users.len() > Self::MAX_USERS {
            return Err(Error::ResourceBudgetExceeded);
        }

        Ok(())
    }
}

/// Applies pinned-root and predecessor authority to one control's own roster.
struct ControlAuthority<'a> {
    body: &'a Control,
    state: &'a SyncState,
    roster: Roster<'a>,
}

impl<'a> ControlAuthority<'a> {
    fn new(body: &'a Control, state: &'a SyncState) -> Self {
        Self {
            body,
            state,
            roster: Roster(&body.members),
        }
    }

    fn signing_key(&self, previous: Option<&Signed<Control>>) -> Result<Key> {
        self.validate_shape()?;
        self.roster.verify(self.body.owner, &self.body.grants)?;
        let controller = self.roster.controller(self.body.controller)?;

        match previous {
            None => self.genesis_key(controller),
            Some(previous) => self.successor_key(previous),
        }
    }

    fn validate_shape(&self) -> Result<()> {
        let body = self.body;
        if body.epoch.space != self.state.space
            || body.owner != Hash::digest(b"lockbox.user\0", &self.state.owner_root)
            || body.closed_frontier.len() > body.frontier_limit()
            || !sorted_unique(&body.closed_frontier)
            || body.checkpoint_nonce.0.len() != 24
            || body.checkpoint.0.len() < 16
        {
            return Err(Error::InvalidEncoding);
        }

        Ok(())
    }

    fn genesis_key(&self, controller: &Member) -> Result<Key> {
        let body = self.body;

        if body.epoch.number.0 != 0
            || body.epoch.previous.is_some()
            || !body.closed_frontier.is_empty()
            || body.transition.is_some()
            || body.approval.is_some()
            || controller.certificate.body.root_key != self.state.owner_root
        {
            return Err(Error::UnauthorizedDevice);
        }

        Ok(controller.certificate.body.signing_key)
    }

    fn successor_key(&self, previous: &Signed<Control>) -> Result<Key> {
        let body = self.body;

        let author = Roster(&previous.body.members).member(previous.body.controller)?;

        if previous.body.epoch.number.0.checked_add(1) != Some(body.epoch.number.0)
            || body.owner != previous.body.owner
            || body.epoch.context == previous.body.epoch.context
            || body.epoch.previous != Some(Object::Control(previous.clone()).hash()?)
        {
            return Err(Error::UnauthorizedDevice);
        }

        match body.transition {
            None if body.controller == previous.body.controller => {
                Ok(author.certificate.body.signing_key)
            }
            Some(ControlTransition::Transfer) if body.controller != previous.body.controller => {
                let old_target = Roster(&previous.body.members).controller(body.controller)?;
                let new_target = self.roster.controller(body.controller)?;
                if old_target.certificate.hash()? != new_target.certificate.hash()? {
                    return Err(Error::UnauthorizedDevice);
                }
                Ok(author.certificate.body.signing_key)
            }
            Some(ControlTransition::Recovery) if body.controller != previous.body.controller => {
                if self.roster.member(previous.body.controller).is_ok() {
                    return Err(Error::UnauthorizedDevice);
                }
                let target = self.roster.controller(body.controller)?;
                if target.certificate.body.root_key != author.certificate.body.root_key {
                    return Err(Error::UnauthorizedDevice);
                }
                Ok(target.certificate.body.signing_key)
            }
            _ => Err(Error::UnauthorizedDevice),
        }
    }
}
