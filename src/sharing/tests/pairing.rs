use super::*;
use crate::{
    sharing::transport::Mailbox,
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
    messages: Mutex<BTreeMap<(Hash, Slot), Vec<u8>>>,
    request_unavailable_once: AtomicBool,
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
        self.0
            .messages
            .lock()
            .unwrap()
            .insert((session, slot), bytes.to_vec());
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
    guest
        .request_link(
            &ticket.link().unwrap(),
            &ticket.verification_code().unwrap().to_string(),
            1,
        )
        .unwrap();
    let memory = Arc::new(Memory::default());
    memory
        .request_unavailable_once
        .store(true, Ordering::Relaxed);
    let factory: Arc<dyn MailboxFactory> = Arc::new(Factory(memory.clone()));
    let mut controller = Pairing::new(Some(factory.clone()));
    let mut recipient = Pairing::new(Some(factory));
    let mut owner_sync = SpaceSync::default();
    let mut guest_sync = SpaceSync::default();
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
