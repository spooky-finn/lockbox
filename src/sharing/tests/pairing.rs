use super::*;
use crate::{
    sync::transport::pairing::Mailbox,
    sync::{
        protocol::Role,
        transport::{Cancellation, Page},
    },
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
    },
};
use secrecy::SecretString;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Memory {
    single_request_slot: bool,
    messages: Mutex<BTreeMap<(Hash, Slot), Vec<u8>>>,
    request_unavailable_once: AtomicBool,
    ticket_unavailable_once: AtomicBool,
    ticket_upload_fails_once: AtomicBool,
}

struct Factory(Arc<Memory>);
struct Store(Arc<Memory>);

impl MailboxFactory for Factory {
    fn connect(&self) -> NetworkResult<Box<dyn Mailbox>> {
        Ok(Box::new(Store(self.0.clone())))
    }
}

impl Mailbox for Store {
    fn get(
        &mut self,
        session: Hash,
        slot: Slot,
        cancel: &Cancellation,
    ) -> NetworkResult<Option<Vec<u8>>> {
        cancel.check()?;
        if matches!(slot, Slot::Ticket(_))
            && self
                .0
                .ticket_unavailable_once
                .swap(false, Ordering::Relaxed)
        {
            return Ok(None);
        }
        if matches!(slot, Slot::Request(_))
            && self
                .0
                .request_unavailable_once
                .swap(false, Ordering::Relaxed)
        {
            return Ok(None);
        }
        Ok(self
            .0
            .messages
            .lock()
            .unwrap()
            .get(&(session, slot))
            .cloned())
    }
    fn put(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: &[u8],
        cancel: &Cancellation,
    ) -> NetworkResult<()> {
        cancel.check()?;
        if matches!(slot, Slot::Ticket(_))
            && self
                .0
                .ticket_upload_fails_once
                .swap(false, Ordering::Relaxed)
        {
            return Err(TransportError::Unavailable);
        }
        let mut messages = self.0.messages.lock().unwrap();
        if self.0.single_request_slot && matches!(slot, Slot::Request(_)) {
            messages.retain(|(id, key), _| *id != session || !matches!(key, Slot::Request(_)));
        }
        messages.insert((session, slot), bytes.to_vec());
        Ok(())
    }
    fn requests(&mut self, session: Hash, _: Option<&str>) -> NetworkResult<Page> {
        Ok(Page {
            objects: self
                .0
                .messages
                .lock()
                .unwrap()
                .keys()
                .filter_map(|(id, slot)| match slot {
                    Slot::Request(request) if *id == session => Some(*request),
                    _ => None,
                })
                .collect(),
            next: None,
        })
    }
}

fn profile(directory: &tempfile::TempDir, name: &str) -> Vault {
    ProfileStore::new(Arc::new(MemoryWitness::default()))
        .create_shared(
            directory.path().join(format!("{name}.lbx")),
            &SecretString::from("fictional master".to_owned()),
            &SecretString::from("fictional recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: name.into(),
                space_name: "Team".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "test-bucket".into(),
                    access_key: "fictional-access".into(),
                    secret_key: "fictional-secret".into(),
                },
            },
        )
        .unwrap()
}

fn cycle(pairing: &mut Pairing, vault: &mut Vault, sync: &mut SpaceSync) {
    pairing.wake();
    assert!(pairing.tick(vault, sync, false, 1).is_none());
    let deadline = Instant::now() + Duration::from_secs(10);
    while pairing.worker.is_some() {
        assert!(Instant::now() < deadline, "pairing worker did not finish");
        assert!(pairing.tick(vault, sync, false, 1).is_none());
        std::thread::yield_now();
    }
}

#[test]
fn background_mailbox_completes_link_flow_and_retries_partially_available_requests() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = profile(&directory, "Owner");
    let mut guest = profile(&directory, "Alice");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = owner.issued_links().next().unwrap().ticket.clone();
    let memory = Arc::new(Memory::default());
    memory
        .request_unavailable_once
        .store(true, Ordering::Relaxed);
    memory
        .ticket_unavailable_once
        .store(true, Ordering::Relaxed);
    let factory: Arc<dyn MailboxFactory> = Arc::new(Factory(memory.clone()));
    let mut controller = Pairing::new(Some(factory.clone()));
    let mut recipient = Pairing::new(Some(factory));
    let mut owner_sync = SpaceSync::default();
    let mut guest_sync = SpaceSync::default();
    assert!(!owner.link_ready(session));
    assert!(owner.export_link(session, false).is_err());
    cycle(&mut controller, &mut owner, &mut owner_sync);
    let link = owner.export_link(session, false).unwrap();
    assert_eq!(link.len(), 62);
    assert!(link.starts_with("lockbox://i/"));
    guest
        .request_link(&link, &ticket.verification_code().unwrap().to_string(), 1)
        .unwrap();
    cycle(&mut recipient, &mut guest, &mut guest_sync);
    assert!(guest.resolving_link().is_some());
    assert!(guest.joining_link().is_none());
    for _ in 0..16 {
        cycle(&mut controller, &mut owner, &mut owner_sync);
        let pending = owner
            .issued_links()
            .next()
            .filter(|link| link.approval.is_none())
            .and_then(|link| link.requests.keys().next().copied());
        if let Some(request) = pending {
            owner
                .approve_link_request(session, request, Role::Reader, true, 1)
                .unwrap();
        }
        cycle(&mut recipient, &mut guest, &mut guest_sync);
        if guest
            .joining_link()
            .is_some_and(|link| link.offer.is_some() && !link.consented)
        {
            guest.consent_link(session, 1).unwrap();
        }
    }
    assert!(guest.joining_link().unwrap().joined);
    assert!(owner.issued_links().next().unwrap().closed.is_some());
    assert_eq!(owner.space_members().unwrap().len(), 2);
    assert_eq!(
        owner.participant_name(guest.device_certificate().unwrap().body.user),
        Some("Alice")
    );
    assert!(!controller.seen[&session].is_empty());
}

#[test]
fn switching_profile_discards_pending_mailbox_results() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = profile(&directory, "Owner");
    owner.create_link(0, 1).unwrap();
    let memory = Arc::new(Memory::default());
    let mut runtime = Pairing::new(Some(Arc::new(Factory(memory))));
    let mut sync = SpaceSync::default();
    assert!(runtime.tick(&mut owner, &mut sync, false, 1).is_none());
    assert!(runtime.worker.is_some());
    let mut replacement = profile(&directory, "Replacement");
    assert!(
        runtime
            .tick(&mut replacement, &mut sync, false, 1)
            .is_none()
    );
    assert!(runtime.worker.is_none());
    assert!(!replacement.has_pairing());
    assert_eq!(replacement.space_members().unwrap().len(), 1);
}

#[test]
fn interrupted_ticket_upload_disables_copy_until_the_exact_delivery_is_retried() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = profile(&directory, "Owner");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = owner.issued_links().next().unwrap().ticket.clone();
    let hash = ticket.hash().unwrap();
    let memory = Arc::new(Memory::default());
    memory
        .ticket_upload_fails_once
        .store(true, Ordering::Relaxed);
    let mut runtime = Pairing::new(Some(Arc::new(Factory(memory.clone()))));
    let mut sync = SpaceSync::default();
    assert!(runtime.tick(&mut owner, &mut sync, false, 1).is_none());
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        assert!(Instant::now() < deadline);
        if let Some(error) = runtime.tick(&mut owner, &mut sync, false, 1) {
            break error;
        }
        std::thread::yield_now();
    };
    assert!(error.contains("unavailable"));
    assert!(!owner.link_ready(session));
    assert!(owner.export_link(session, false).is_err());
    assert!(
        memory
            .messages
            .lock()
            .unwrap()
            .contains_key(&(session, Slot::Bootstrap))
    );
    assert!(
        !memory
            .messages
            .lock()
            .unwrap()
            .contains_key(&(hash, Slot::Ticket(hash)))
    );
    owner.retry_pairing().unwrap();
    cycle(&mut runtime, &mut owner, &mut sync);
    assert!(owner.link_ready(session));
    assert_eq!(
        owner.export_link(session, false).unwrap(),
        ticket.reference().unwrap().link()
    );
    assert_eq!(
        memory.messages.lock().unwrap()[&(hash, Slot::Ticket(hash))],
        crate::sync::protocol::canonical(&ticket).unwrap()
    );
}

#[test]
fn switching_profile_discards_a_pending_short_ticket_download() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = profile(&directory, "Owner");
    owner.create_link(0, 1).unwrap();
    let ticket = owner.issued_links().next().unwrap().ticket.clone();
    let mut guest = profile(&directory, "Guest");
    guest
        .request_link(
            &ticket.reference().unwrap().link(),
            &ticket.verification_code().unwrap().to_string(),
            1,
        )
        .unwrap();
    let memory = Arc::new(Memory::default());
    memory.messages.lock().unwrap().insert(
        (ticket.hash().unwrap(), Slot::Ticket(ticket.hash().unwrap())),
        crate::sync::protocol::canonical(&ticket).unwrap(),
    );
    let mut runtime = Pairing::new(Some(Arc::new(Factory(memory))));
    let mut sync = SpaceSync::default();
    assert!(runtime.tick(&mut guest, &mut sync, false, 1).is_none());
    assert!(runtime.worker.is_some());
    let mut replacement = profile(&directory, "Replacement");
    assert!(
        runtime
            .tick(&mut replacement, &mut sync, false, 1)
            .is_none()
    );
    assert!(runtime.worker.is_none());
    assert!(!replacement.has_pairing());
    assert_eq!(replacement.space_members().unwrap().len(), 1);
}

#[test]
fn failed_issuer_verification_is_polled_without_an_invitation_and_can_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = profile(&directory, "Owner");
    let mut links = Pairing::default();
    let mut sync = SpaceSync::default();
    assert!(!owner.has_pairing());
    links.setup = Some(Worker::ready(Err(TransportError::Unavailable.into())));
    assert!(links.needs_tick(&owner, false));
    assert!(links.tick(&mut owner, &mut sync, false, 1).is_some());
    assert!(!links.checking_issuer());
    assert!(!owner.has_invite_issuer());
    links
        .configure_issuer(&owner, "test-invalid-grant")
        .unwrap();
    assert!(links.checking_issuer());
    links.suspend();
    assert!(!links.checking_issuer());
}

#[test]
fn returned_confirmation_waits_for_its_own_request_in_the_background_mailbox() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = profile(&dir, "Owner");
    owner.test_profile_mut().unwrap().pairing.issuer = Some("test-only-issuer".into());
    let mut guest = profile(&dir, "Guest");
    let mut other = profile(&dir, "Other");
    let guest_device = guest.device_certificate().unwrap().body.device;
    let factory: Arc<dyn MailboxFactory> = Arc::new(Factory(Arc::new(Memory {
        single_request_slot: true,
        ..Default::default()
    })));
    let mut controller = Pairing::new(Some(factory.clone()));
    // This test ends at admission. Provider registration has its own suite.
    controller.credential_due = Instant::now() + Duration::from_secs(600);
    let mut recipient = Pairing::new(Some(factory.clone()));
    let mut forwarded = Pairing::new(Some(factory));
    let mut owner_sync = SpaceSync::default();
    let mut guest_sync = SpaceSync::default();
    let mut other_sync = SpaceSync::default();

    let session = owner.create_chat_link(0, 1, Role::Editor).unwrap();
    cycle(&mut controller, &mut owner, &mut owner_sync);
    let invite = owner.export_link(session, false).unwrap();
    guest.join_chat_link(&invite, 1).unwrap();
    other.join_chat_link(&invite, 1).unwrap();

    // The production mailbox has a single request object. The forwarded
    // invitation holder overwrites it before the controller can poll.
    cycle(&mut recipient, &mut guest, &mut guest_sync);
    let confirmation = guest.confirmation_link().unwrap();
    cycle(&mut recipient, &mut guest, &mut guest_sync);
    cycle(&mut forwarded, &mut other, &mut other_sync);
    for _ in 0..3 {
        cycle(&mut forwarded, &mut other, &mut other_sync);
        cycle(&mut controller, &mut owner, &mut owner_sync);
    }
    assert_eq!(owner.issued_links().next().unwrap().requests.len(), 1);
    assert!(owner.approve_chat_link(&confirmation, 1).is_err());
    assert_eq!(owner.space_members().unwrap().len(), 1);

    // The guest's Retry republishes the original signed request. The
    // confirmation already returned through the trusted chat still matches.
    guest.retry_pairing().unwrap();
    cycle(&mut recipient, &mut guest, &mut guest_sync);
    cycle(&mut controller, &mut owner, &mut owner_sync);
    assert_eq!(owner.issued_links().next().unwrap().requests.len(), 2);
    assert!(owner.review_chat_link(&confirmation, 1).is_ok());
    assert!(owner.issued_links().next().unwrap().approval.is_none());

    owner.approve_chat_link(&confirmation, 1).unwrap();
    for _ in 0..5 {
        cycle(&mut controller, &mut owner, &mut owner_sync);
        cycle(&mut recipient, &mut guest, &mut guest_sync);
        cycle(&mut forwarded, &mut other, &mut other_sync);
    }
    assert!(guest.joining_link().unwrap().consented);
    assert!(!other.joining_link().unwrap().consented);
    let members = owner.space_members().unwrap();
    assert_eq!(members.len(), 2);
    assert!(
        members
            .iter()
            .any(|(c, r)| c.body.device == guest_device && *r == Role::Editor)
    );
}
