//! Local Storj delegation ledger. Only authenticated admission can schedule issuance.
use super::*;
use crate::sync::protocol::pairing::SignedStorage;
use crate::{
    sync::{
        protocol::{AdmissionReceipt, Blob},
        transport::{
            Storage, provider,
            storj::{
                Storj,
                invite::{GuestFactory, Owner, OwnerFactory},
                issuer,
            },
        },
    },
    vault::profile::{BootstrapProfile, ProviderConfiguration},
};
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

pub(crate) enum CredentialTask {
    Register {
        session: Hash,
        request: Hash,
        device: Hash,
        grant: Zeroizing<String>,
        endpoint: String,
        bucket: String,
        space: String,
        namespace: Hash,
        genesis: Hash,
        expected: Blob,
    },
    Revoke {
        device: Hash,
        owner: Zeroizing<String>,
        grant: Zeroizing<String>,
    },
}

pub(crate) enum CredentialResult {
    Registered(Hash, Hash, Hash, issuer::MemberCredentials),
    Revoked(Hash),
}

impl CredentialTask {
    pub(crate) fn execute(self) -> Result<CredentialResult> {
        match self {
            Self::Revoke {
                device,
                owner,
                grant,
            } => {
                issuer::revoke(&owner, &grant).map_err(|_| {
                    VaultError::Pairing(
                        "Storage revocation pending. Check the Storj connection and issuer helper.",
                    )
                })?;
                Ok(CredentialResult::Revoked(device))
            }
            Self::Register {
                session,
                request,
                device,
                grant,
                endpoint,
                bucket,
                space,
                namespace,
                genesis,
                expected,
            } => {
                let keys = issuer::register(&grant).map_err(|_| {
                    VaultError::Pairing("Could not register device storage access with Storj")
                })?;
                if url::Url::parse(&keys.endpoint).ok() != url::Url::parse(&endpoint).ok() {
                    return Err(VaultError::Pairing(
                        "The Storj grant uses a different gateway",
                    ));
                }
                // Confirm this grant can read this vault with its storage encryption context.
                let mut store =
                    Storj::new(&keys.endpoint, &bucket, &space, &keys.access, &keys.secret)?;
                if store.get(namespace, genesis)? != expected.0 {
                    return Err(Error::HashMismatch.into());
                }
                Ok(CredentialResult::Registered(session, request, device, keys))
            }
        }
    }
}

pub(crate) struct IssuerSetup {
    grant: Zeroizing<String>,
    bucket: String,
    space: String,
    device: Hash,
    genesis: Hash,
    expected: Blob,
}

pub(crate) struct VerifiedIssuer(IssuerSetup);

impl IssuerSetup {
    pub(crate) fn verify(mut self) -> Result<VerifiedIssuer> {
        self.grant = issuer::verified_issuer(&self.grant, &self.bucket, &self.space, self.genesis, &self.expected.0)
            .map_err(|_| VaultError::Pairing("Could not verify the Storj Access Grant. Sync this vault and check that the grant allows reading, listing, and uploading its objects."))?;
        Ok(VerifiedIssuer(self))
    }
}

impl Vault {
    pub(crate) fn has_invite_issuer(&self) -> bool {
        self.profile
            .as_ref()
            .is_some_and(|p| p.pairing.issuer.is_some())
    }

    pub(crate) fn prepare_issuer_setup(&self, grant: &str) -> Result<IssuerSetup> {
        self.ensure_profile_writable()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let device = profile.sync_secret().id();
        if self.space_controller()? != device {
            return Err(Error::InsufficientRole.into());
        }
        if self.has_invite_issuer() {
            return Err(VaultError::Pairing(
                "Storage invitation access is already configured",
            ));
        }
        let grant = grant.trim();
        if grant.is_empty() || grant.len() > 32768 {
            return Err(Error::InvalidEncoding.into());
        }
        let state = profile.sync.as_ref().ok_or(Error::MissingDependency)?;
        Ok(IssuerSetup {
            grant: Zeroizing::new(grant.into()),
            bucket: profile.provider.bucket.clone(),
            space: profile.space_name.clone(),
            device,
            genesis: state.genesis,
            expected: state
                .objects
                .get(&state.genesis)
                .ok_or(Error::MissingDependency)?
                .clone(),
        })
    }

    pub(crate) fn configure_invite_issuer(&mut self, verified: VerifiedIssuer) -> Result<()> {
        let setup = verified.0;
        self.ensure_profile_writable()?;
        if self.space_controller()? != setup.device {
            return Err(Error::InsufficientRole.into());
        }
        let p = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        if p.sync_secret().id() != setup.device
            || p.sync.as_ref().map(|s| s.genesis) != Some(setup.genesis)
            || p.provider.bucket != setup.bucket
            || p.space_name != setup.space
            || p.pairing.issuer.is_some()
        {
            return Err(Error::UnauthorizedDevice.into());
        }
        p.pairing.issuer = Some(setup.grant.to_string());
        if let Err(err) = self.save_pairing() {
            self.profile.as_mut().unwrap().pairing.issuer = None;
            return Err(err);
        }
        Ok(())
    }

    pub(super) fn create_managed_mailbox(
        profile: &BootstrapProfile,
        ticket: &Ticket,
        days: u64,
    ) -> Result<ManagedLink> {
        let config = &profile.provider;
        // Blocking transport owns its Tokio runtime, outside the desktop runtime.
        let grant = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    Owner::new(
                        &config.base_url,
                        &config.bucket,
                        &profile.space_name,
                        &config.access_key,
                        &config.secret_key,
                        ticket.body.id,
                    )?
                    .grant(Duration::from_secs(days * 86400))
                })
                .join()
        })
        .map_err(|_| VaultError::Pairing("Could not prepare the invitation mailbox"))??;
        let managed = ManagedLink::create(ticket.clone(), grant, &profile.sync_secret())?;
        managed.link()?;
        Ok(managed)
    }

    /// Renew transport only for the previously approved device. Admission deadline,
    /// signature, role, consent and verification code remain bound to the same ticket.
    pub(crate) fn renew_mailbox(&mut self, session: Hash) -> Result<()> {
        self.ensure_profile_writable()?;
        let p = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let link = p
            .pairing
            .issued
            .get(&session)
            .ok_or(Error::InvalidInvitation)?;
        if self.space_controller()? != p.sync_secret().id()
            || link.managed.is_none()
            || link.closed.is_some()
            || link
                .approval
                .as_ref()
                .and_then(|a| a.invitation)
                .and_then(|id| p.sharing.issued.get(&id))
                .is_none_or(|r| r.receipt.is_none())
        {
            return Err(Error::InvalidInvitation.into());
        }
        let approved = link.approval.as_ref().ok_or(Error::InvalidInvitation)?;
        let device = link.requests[&approved.request]
            .body
            .certificate
            .body
            .device;
        if !self
            .space_members()?
            .iter()
            .any(|(c, role)| c.body.device == device && *role == approved.role)
        {
            return Err(Error::UnauthorizedDevice.into());
        }
        let renewed = Self::create_managed_mailbox(p, &link.ticket, 1)?;
        self.profile
            .as_mut()
            .unwrap()
            .pairing
            .issued
            .get_mut(&session)
            .unwrap()
            .managed = Some(renewed);
        self.save_pairing()
    }

    pub(crate) fn resume_managed_mailbox(&mut self, session: Hash, value: &str) -> Result<()> {
        self.ensure_profile_writable()?;
        let renewed = ManagedLink::parse(value)?;
        let p = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        let link = p
            .pairing
            .joining
            .get_mut(&session)
            .ok_or(Error::InvalidInvitation)?;
        if link.managed.is_none()
            || !link.consented
            || link.closed.is_some()
            || link.ticket.hash()? != renewed.body.ticket.hash()?
        {
            return Err(Error::InvalidInvitation.into());
        }
        link.managed = Some(renewed);
        self.save_pairing()
    }

    pub(super) fn managed_mailbox_factory(
        profile: &BootstrapProfile,
        ticket: &Ticket,
    ) -> Option<Arc<dyn MailboxFactory>> {
        let t = &ticket.body;
        if profile
            .pairing
            .issued
            .get(&t.id)
            .is_some_and(|l| l.managed.is_some())
        {
            let c = &profile.provider;
            return Some(Arc::new(OwnerFactory::new(
                &c.base_url,
                &c.bucket,
                &t.space_name,
                &c.access_key,
                &c.secret_key,
                t.id,
            )));
        }
        let link = profile.pairing.joining.get(&t.id)?.managed.as_ref()?;
        Some(Arc::new(GuestFactory::new(
            &link.body.grant,
            t.id,
            &t.endpoint,
            &t.bucket,
            &t.space_name,
        )))
    }

    pub(crate) fn next_credential_task(&mut self, round: usize) -> Result<Option<CredentialTask>> {
        let Some(p) = self.profile.as_ref() else {
            return Ok(None);
        };
        let Some(owner) = p.pairing.issuer.as_ref() else {
            return Ok(None);
        };
        if p.pairing.needs_save {
            self.save_pairing()?;
            return Ok(None);
        }
        let members = self.space_members()?;
        let is_member = |device: Hash| members.iter().any(|(c, _)| c.body.device == device);
        // A failed satellite request for one device must not starve other
        // registrations or removals. Only prepare the selected device's grant.
        enum Pending {
            Revoke(Hash),
            Register {
                session: Hash,
                request: Hash,
                device: Hash,
            },
        }
        let mut pending: Vec<_> = p
            .pairing
            .device_grants
            .keys()
            .filter(|d| !is_member(**d))
            .map(|d| Pending::Revoke(*d))
            .collect();
        if self.space_controller()? == p.sync_secret().id() {
            for (session, link) in &p.pairing.issued {
                if link.managed.is_none() || link.closed.is_some() {
                    continue;
                }
                let Some(approval) = &link.approval else {
                    continue;
                };
                if self.has_delivery(*session, Slot::Receipt(approval.request)) {
                    continue;
                }
                let receipt = approval
                    .invitation
                    .and_then(|id| p.sharing.issued.get(&id))
                    .and_then(|r| r.receipt.as_ref());
                let device = link.requests[&approval.request]
                    .body
                    .certificate
                    .body
                    .device;
                if is_member(device)
                    && receipt.is_some_and(|r| r.descriptor() == &link.ticket.body.descriptor)
                {
                    pending.push(Pending::Register {
                        session: *session,
                        request: approval.request,
                        device,
                    });
                }
            }
        }
        if pending.is_empty() {
            return Ok(None);
        }
        let index = round % pending.len();
        let (session, request, device) = match pending.swap_remove(index) {
            Pending::Revoke(device) => {
                return Ok(Some(CredentialTask::Revoke {
                    device,
                    owner: Zeroizing::new(owner.clone()),
                    grant: Zeroizing::new(p.pairing.device_grants[&device].clone()),
                }));
            }
            Pending::Register {
                session,
                request,
                device,
            } => (session, request, device),
        };
        let state = p.sync.as_ref().ok_or(Error::MissingDependency)?;
        let existing = p.pairing.device_grants.get(&device);
        let grant = match existing {
            Some(grant) => Zeroizing::new(grant.clone()),
            None => issuer::prepare(owner, &p.provider.bucket, &p.space_name).map_err(|_| {
                VaultError::Pairing("Could not restrict the Storj grant to this vault")
            })?,
        };
        let task = CredentialTask::Register {
            session,
            request,
            device,
            grant: grant.clone(),
            endpoint: p.provider.base_url.clone(),
            bucket: p.provider.bucket.clone(),
            space: p.space_name.clone(),
            namespace: state.namespace,
            genesis: state.genesis,
            expected: state
                .objects
                .get(&state.genesis)
                .ok_or(Error::MissingDependency)?
                .clone(),
        };
        if existing.is_none() {
            self.profile
                .as_mut()
                .unwrap()
                .pairing
                .device_grants
                .insert(device, grant.to_string());
            self.save_pairing()?;
        }
        Ok(Some(task))
    }

    pub(crate) fn finish_credential_task(&mut self, reply: CredentialResult) -> Result<()> {
        self.ensure_profile_writable()?;
        let p = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        match reply {
            CredentialResult::Revoked(device) => {
                self.profile
                    .as_mut()
                    .unwrap()
                    .pairing
                    .device_grants
                    .remove(&device)
                    .zeroize();
            }
            CredentialResult::Registered(session, request, device, keys) => {
                let link = p
                    .pairing
                    .issued
                    .get(&session)
                    .ok_or(Error::InvalidInvitation)?;
                if link.closed.is_some() || self.has_delivery(session, Slot::Receipt(request)) {
                    return Ok(());
                }
                let approval = link
                    .approval
                    .as_ref()
                    .filter(|a| a.request == request)
                    .ok_or(Error::InvalidInvitation)?;
                let cert = &link.requests[&request].body.certificate.body;
                if !p.pairing.device_grants.contains_key(&device)
                    || cert.device != device
                    || self.space_controller()? != p.sync_secret().id()
                    || !self
                        .space_members()?
                        .iter()
                        .any(|(c, r)| c.body.device == device && *r == approval.role)
                {
                    return Err(Error::UnauthorizedDevice.into());
                }
                let receipt = approval
                    .invitation
                    .and_then(|id| p.sharing.issued.get(&id))
                    .and_then(|r| r.receipt.as_ref())
                    .ok_or(Error::InvalidInvitation)?;
                let storage = SignedStorage::issue(
                    &link.ticket,
                    request,
                    receipt,
                    keys.access.to_string(),
                    keys.secret.to_string(),
                    &p.sync_secret(),
                )?;
                let frame = Envelope::seal(
                    &link.ticket,
                    request,
                    Kind::Receipt,
                    cert,
                    &(receipt, storage),
                )?;
                self.queue_pairing(session, Slot::Receipt(request), frame.bytes()?)?;
            }
        }
        self.save_pairing()
    }

    pub(super) fn managed_receipt(
        link: &JoiningLink,
        request: Hash,
        frame: &Envelope,
        device: &crate::sync::protocol::DeviceSecret,
    ) -> Result<(AdmissionReceipt, Option<ProviderConfiguration>)> {
        if link.managed.is_none() {
            return Ok((
                frame.open(&link.ticket, request, Kind::Receipt, device)?,
                None,
            ));
        }
        let (receipt, storage): (AdmissionReceipt, SignedStorage) =
            frame.open(&link.ticket, request, Kind::Receipt, device)?;
        storage.verify_storage(&link.ticket, request, &receipt)?;
        let t = &link.ticket.body;
        provider::validate(
            &t.endpoint,
            &t.bucket,
            &storage.body.access,
            &storage.body.secret,
        )
        .map_err(|_| Error::InvalidInvitation)?;
        Ok((
            receipt,
            Some(ProviderConfiguration {
                base_url: t.endpoint.clone(),
                bucket: t.bucket.clone(),
                access_key: storage.body.access.clone(),
                secret_key: storage.body.secret.clone(),
            }),
        ))
    }
}
