//! Owner authority, roster validation, HPKE grants, and epoch checkpoints.

use super::super::{
    Blob, Certificate, DeviceSecret, Error, Hash, Object, Projection, Result, Role, Signed,
    canonical,
    crypto::{self, CONTROL_DOMAIN},
    wire::sorted_unique,
};
use super::{
    protocol::{Control, Epoch, Grant, Member, PROFILE},
    state::SyncState,
};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub(super) fn grant_aad(epoch: &Epoch, certificate: &Signed<Certificate>) -> Result<Vec<u8>> {
    canonical(&(
        epoch,
        Hash::digest(
            b"lockbox.sync.certificate-object.v0\0",
            &canonical(certificate)?,
        ),
    ))
}

pub(super) fn epoch_key(control: &Control, device: &DeviceSecret) -> Result<Zeroizing<[u8; 32]>> {
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

pub(super) fn build_control(
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

pub(super) fn validate_control(
    control: &Signed<Control>,
    previous: Option<&Signed<Control>>,
    state: &SyncState,
) -> Result<()> {
    let key = validate_control_body(&control.body, previous, state)?;
    crypto::verify(control, &key, CONTROL_DOMAIN)
}

pub(super) fn validate_control_body(
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
        || body.closed_frontier.len() > super::super::MAX_PARENTS
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
