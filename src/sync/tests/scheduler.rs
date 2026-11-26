//! Application orchestration tests through fake transports and ready mailboxes.

use crate::{
    records::entry::Entry,
    sync::{
        protocol::{Machine, Transaction},
        scheduler::*,
    },
    vault::{KdfParams, ProfileStore, ProviderConfiguration, SharedProfile},
};
use secrecy::SecretString;
use std::time::Duration;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct MemoryFactory {
    objects: Arc<Mutex<BTreeMap<Hash, Vec<u8>>>>,
    recovery: Arc<Mutex<BTreeMap<Hash, Vec<u8>>>>,
    fail_backup: Arc<AtomicBool>,
    connections: AtomicUsize,
    connection_threads: Mutex<Vec<std::thread::ThreadId>>,
    fail_once: AtomicBool,
}

impl StorageFactory for MemoryFactory {
    fn connect(&self) -> sync::Result<Box<dyn sync::Storage + Send>> {
        self.connections.fetch_add(1, Ordering::Relaxed);
        self.connection_threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        if self.fail_once.swap(false, Ordering::Relaxed) {
            return Err(TransportError::AccessDenied);
        }
        Ok(Box::new(MemoryStorage(
            self.objects.clone(),
            self.recovery.clone(),
            self.fail_backup.clone(),
        )))
    }
}

struct MemoryStorage(
    Arc<Mutex<BTreeMap<Hash, Vec<u8>>>>,
    Arc<Mutex<BTreeMap<Hash, Vec<u8>>>>,
    Arc<AtomicBool>,
);

impl sync::Storage for MemoryStorage {
    fn list(&mut self, _: Hash, _: Option<&str>) -> sync::Result<sync::Page> {
        Ok(sync::Page {
            objects: self.0.lock().unwrap().keys().rev().copied().collect(),
            next: None,
        })
    }

    fn get(&mut self, _: Hash, hash: Hash) -> sync::Result<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .get(&hash)
            .cloned()
            .ok_or(TransportError::NotFound)
    }

    fn put(&mut self, _: Hash, hash: Hash, bytes: &[u8]) -> sync::Result<()> {
        self.0.lock().unwrap().insert(hash, bytes.to_vec());
        Ok(())
    }
    fn list_recovery(&mut self, _: Hash, _: Option<&str>) -> sync::Result<sync::Page> {
        Ok(sync::Page {
            objects: self.1.lock().unwrap().keys().copied().collect(),
            next: None,
        })
    }
    fn get_recovery(&mut self, _: Hash, hash: Hash) -> sync::Result<Vec<u8>> {
        self.1
            .lock()
            .unwrap()
            .get(&hash)
            .cloned()
            .ok_or(TransportError::NotFound)
    }
    fn put_recovery(&mut self, _: Hash, hash: Hash, bytes: &[u8]) -> sync::Result<()> {
        if self.2.load(Ordering::Relaxed) {
            return Err(TransportError::AccessDenied);
        }
        self.1.lock().unwrap().insert(hash, bytes.to_vec());
        Ok(())
    }
}

#[test]
fn cloud_backup_failure_is_visible_and_retry_publishes_before_synced_status() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    assert!(
        vault
            .enable_cloud_recovery(&SecretString::from("recovery".to_owned()))
            .unwrap()
    );
    let factory = Arc::new(MemoryFactory::default());
    factory.fail_backup.store(true, Ordering::Relaxed);
    let mut session = SpaceSync::new(Some(factory.clone()));
    assert!(drive(&mut session, &mut vault).is_some());
    assert_eq!(session.status, Status::Offline);
    assert!(!factory.objects.lock().unwrap().is_empty());
    assert!(factory.recovery.lock().unwrap().is_empty());
    factory.fail_backup.store(false, Ordering::Relaxed);
    session.retry();
    assert!(drive(&mut session, &mut vault).is_none());
    assert_eq!(session.status, Status::Synced);
    assert_eq!(factory.recovery.lock().unwrap().len(), 1);
}

fn drive(session: &mut SpaceSync, vault: &mut Vault) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if session.needs_tick(false)
            && let Some(error) = session.tick(vault, false)
        {
            return Some(error);
        }
        if matches!(session.status, Status::Synced | Status::Conflict) {
            return None;
        }
        assert!(Instant::now() < deadline, "fixture worker did not finish");
        std::thread::yield_now();
    }
}

fn vault(dir: &tempfile::TempDir) -> Vault {
    vault_using(dir, &ProfileStore::default())
}

fn vault_using(dir: &tempfile::TempDir, profiles: &ProfileStore) -> Vault {
    profiles
        .create_shared(
            dir.path().join("profile.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Alice".into(),
                space_name: "Team".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "test-bucket".into(),
                    access_key: "access".into(),
                    secret_key: "secret".into(),
                },
            },
        )
        .unwrap()
}

#[test]
fn witness_failure_cancels_old_work_even_during_a_draft_and_retry_can_resume() {
    use crate::vault::witness::MemoryWitness;
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let profiles = ProfileStore::new(witnesses.clone());
    let mut vault = vault_using(&directory, &profiles);
    let factory = Arc::new(MemoryFactory::default());
    let mut session = SpaceSync::new(Some(factory.clone()));
    let (release, wait) = std::sync::mpsc::channel();
    session.scope = Some(Scope::from(vault.sync_state().unwrap()));
    session.worker = Some(Worker::spawn(move |_| {
        let _ = wait.recv();
        Ok(Reply::Uploaded(vec![]))
    }));
    witnesses.fail_reads(true);
    assert!(vault.reconcile_profile().is_err());
    assert!(session.needs_tick_for(&vault, true));
    assert!(session.tick(&mut vault, true).is_none());
    assert!(session.worker.is_none());
    assert_eq!(session.status, Status::Recovery);
    assert!(!session.stopped);
    let _ = release.send(());
    assert_eq!(factory.connections.load(Ordering::Relaxed), 0);
    witnesses.fail_reads(false);
    session.retry();
    assert!(drive(&mut session, &mut vault).is_none());
    assert!(vault.signing_error().is_none());
    assert_eq!(session.status, Status::Synced);
}

#[test]
fn downloaded_history_waits_until_the_active_draft_is_finished() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(&dir);
    let profile = vault.test_profile().unwrap();
    let device = profile.sync_secret();
    let mut remote = profile.sync.as_ref().unwrap().clone();
    let mut entry = Entry::new();
    entry.title = "Remote".into();
    remote.intent = Some(
        Machine::prepare(
            &remote,
            &device,
            Transaction::diff(
                &Machine::replay(&remote, &device).unwrap().projection,
                &[entry],
                &[],
            )
            .unwrap(),
        )
        .unwrap(),
    );
    Machine::complete_intent(&mut remote, &device).unwrap();
    let batch = Download {
        rejected: Vec::new(),
        objects: remote.objects.into_iter().map(|(h, b)| (h, b.0)).collect(),
        incomplete: true,
    };
    let mut session = SpaceSync {
        worker: Some(Worker::ready(Ok(Reply::Downloaded(batch)))),
        factory: Some(Arc::new(MemoryFactory::default())),
        ..SpaceSync::default()
    };
    assert!(!session.needs_tick(true));
    assert!(session.tick(&mut vault, true).is_none());
    assert!(vault.entries().is_empty());
    assert!(session.worker.is_some());
    assert!(session.tick(&mut vault, false).is_none());
    assert_eq!(vault.entries()[0].title, "Remote");
    // Publication is not held hostage by an incomplete download; the status
    // still reports missing history once the local outbox is delivered.
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.status != Status::Incomplete {
        assert!(Instant::now() < deadline, "status {:?}", session.status);
        if session.needs_tick(false) {
            assert!(session.tick(&mut vault, false).is_none());
        }
        std::thread::yield_now();
    }
}

#[test]
fn dropping_the_session_cancels_its_transport_job() {
    let (release, wait) = std::sync::mpsc::channel();
    let (started, receive_cancel) = std::sync::mpsc::channel();
    let mut session = SpaceSync::default();
    session.start(move |cancel| {
        started.send(cancel).unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        Ok(Reply::Uploaded(vec![]))
    });
    let cancel = receive_cancel.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(session);
    assert_eq!(cancel.check(), Err(TransportError::Cancelled));
    release.send(()).unwrap();
}

#[test]
fn new_edits_saved_during_upload_remain_pending_after_old_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(&dir);
    let old_plan: Vec<_> = vault
        .test_profile()
        .unwrap()
        .sync
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .copied()
        .collect();
    let mut entry = Entry::new();
    entry.title = "Saved during upload".into();
    vault.test_entries_mut().push(entry);
    vault.save_changes().unwrap();
    let mut session = SpaceSync {
        worker: Some(Worker::ready(Ok(Reply::Uploaded(old_plan)))),
        ..SpaceSync::default()
    };
    assert!(session.tick(&mut vault, false).is_none());
    assert_eq!(session.status, Status::Saved);
    assert!(session.needs_tick(false));
    assert_eq!(
        vault
            .test_profile()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .outbox
            .len(),
        1
    );
}

#[test]
fn injected_provider_runs_the_complete_background_exchange() {
    let caller = std::thread::current().id();
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let mut entry = Entry::new();
    entry.title = "Local fixture".into();
    vault.test_entries_mut().push(entry);
    vault.save_changes().unwrap();
    let factory = Arc::new(MemoryFactory::default());
    let mut session = SpaceSync::new(Some(factory.clone()));
    assert_eq!(drive(&mut session, &mut vault), None);
    let connections = factory.connection_threads.lock().unwrap();
    assert_eq!(connections.len(), 2);
    assert!(connections.iter().all(|thread| *thread != caller));
    assert_eq!(factory.objects.lock().unwrap().len(), 2);
    assert!(vault.sync_state().unwrap().outbox.is_empty());
    assert!(vault.sync_state().unwrap().last_exchange.is_some());
    assert_eq!(vault.entries()[0].title, "Local fixture");
}

#[test]
fn injected_connection_failure_preserves_the_outbox_until_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let factory = Arc::new(MemoryFactory::default());
    factory.fail_once.store(true, Ordering::Relaxed);
    let mut session = SpaceSync::new(Some(factory.clone()));
    assert_eq!(
        drive(&mut session, &mut vault).as_deref(),
        Some("Storj: provider access denied")
    );
    assert_eq!(session.status, Status::Offline);
    assert_eq!(vault.sync_state().unwrap().outbox.len(), 1);
    session.retry();
    assert_eq!(drive(&mut session, &mut vault), None);
    assert_eq!(factory.objects.lock().unwrap().len(), 1);
    assert!(vault.sync_state().unwrap().outbox.is_empty());
}

#[test]
fn a_verified_fault_cancels_a_pending_job_even_while_editing() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let (started, started_rx) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let mut session = SpaceSync {
        scope: vault.sync_state().map(Scope::from),
        ..SpaceSync::default()
    };
    session.start(move |cancel| {
        started.send(cancel).unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        Ok(Reply::Uploaded(vec![]))
    });
    let cancel = started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    vault
        .test_profile_mut()
        .unwrap()
        .sync
        .as_mut()
        .unwrap()
        .fault = Some(crate::sync::protocol::Error::ControlFork);
    assert!(session.needs_tick_for(&vault, true));
    assert!(session.tick(&mut vault, true).is_none());
    assert!(cancel.check().is_err());
    assert!(session.worker.is_none());
    assert_eq!(session.status, Status::Failed);
    session.saved();
    session.retry();
    assert_eq!(session.status, Status::Failed);
    assert!(!session.needs_tick_for(&vault, false));
    release.send(()).unwrap();
}

#[test]
fn unadmitted_device_waits_for_the_owner_without_reporting_removal() {
    let owner_dir = tempfile::tempdir().unwrap();
    let guest_dir = tempfile::tempdir().unwrap();
    let owner = vault(&owner_dir);
    let mut guest = vault(&guest_dir);
    guest.join_space(owner.space_descriptor().unwrap()).unwrap();
    let batch = Download {
        rejected: Vec::new(),
        incomplete: false,
        objects: owner
            .sync_state()
            .unwrap()
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
    };
    let mut session = SpaceSync::default();
    session.downloaded(&mut guest, batch).unwrap();
    assert_eq!(session.status, Status::Waiting);
    assert!(session.worker.is_none());
    assert!(guest.sync_state().unwrap().outbox.is_empty());
}

#[test]
fn changing_spaces_discards_a_completed_old_reply() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let mut session = SpaceSync {
        scope: vault.sync_state().map(Scope::from),
        ..SpaceSync::default()
    };
    let old_genesis = vault.sync_state().unwrap().genesis;
    session.worker = Some(Worker::ready(Ok(Reply::Uploaded(vec![old_genesis]))));
    let mut descriptor = vault.space_descriptor().unwrap();
    descriptor.space = Hash([0xaa; 32]);
    descriptor.namespace = Hash([0xbb; 32]);
    descriptor.genesis = Hash([0xcc; 32]);
    vault.join_space(descriptor).unwrap();
    assert!(session.needs_tick_for(&vault, true));
    assert!(session.tick(&mut vault, true).is_none());
    assert!(session.worker.is_none());
    assert_eq!(session.status, Status::Saved);
    assert!(vault.sync_state().unwrap().last_exchange.is_none());
}

#[test]
fn upload_completion_does_not_hide_an_invalid_outbox() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let hash = Hash([0xee; 32]);
    vault
        .test_profile_mut()
        .unwrap()
        .sync
        .as_mut()
        .unwrap()
        .outbox
        .insert(hash);
    let mut session = SpaceSync {
        worker: Some(Worker::ready(Ok(Reply::Uploaded(vec![])))),
        ..SpaceSync::default()
    };
    assert!(session.tick(&mut vault, false).is_some());
    assert_ne!(session.status, Status::Synced);
    assert!(vault.sync_state().unwrap().last_exchange.is_none());
    assert!(vault.sync_state().unwrap().outbox.contains(&hash));
}

#[test]
fn a_pending_receive_commit_cancels_old_work_and_resumes_after_storage_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let profile = vault.test_profile().unwrap();
    let device = profile.sync_secret();
    let mut remote = profile.sync.as_ref().unwrap().clone();
    remote.intent = Some(
        Machine::prepare(
            &remote,
            &device,
            Transaction {
                mutations: vec![],
                rebased_from: None,
                authored_at: None,
                space_name: None,
                space_icon: None,
            },
        )
        .unwrap(),
    );
    let hash = Machine::complete_intent(&mut remote, &device).unwrap();
    let temporary = vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert!(
        vault
            .receive_sync(Download {
                rejected: Vec::new(),
                objects: vec![(hash, remote.objects[&hash].0.clone())],
                incomplete: false
            })
            .is_err()
    );
    let factory = Arc::new(MemoryFactory::default());
    let mut session = SpaceSync {
        scope: vault.sync_state().map(Scope::from),
        factory: Some(factory),
        ..SpaceSync::default()
    };
    let (started, started_rx) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    session.start(move |cancel| {
        started.send(cancel).unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        Ok(Reply::Uploaded(vec![]))
    });
    let cancel = started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(session.needs_tick_for(&vault, true));
    assert!(session.tick(&mut vault, true).is_none());
    assert!(cancel.check().is_err());
    assert!(session.worker.is_none());
    assert!(vault.has_pending_exchange());
    assert_eq!(session.status, Status::Incomplete);
    release.send(()).unwrap();
    std::fs::remove_dir(temporary).unwrap();
    assert_eq!(drive(&mut session, &mut vault), None);
    assert!(!vault.has_pending_exchange());
    assert!(vault.sync_state().unwrap().objects.contains_key(&hash));
}

/// Drive passes until `done` holds, skipping retry delays. Returns false on timeout.
fn drive_until(
    session: &mut SpaceSync,
    vault: &mut Vault,
    done: impl Fn(&SpaceSync, &Vault) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if done(session, vault) {
            return true;
        }
        if session.worker.is_none() {
            session.retry();
        }
        if session.needs_tick(false) {
            let _ = session.tick(vault, false);
        }
        std::thread::yield_now();
    }
    done(session, vault)
}

fn replace_membership(value: &mut serde_json::Value, membership: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(slot) = map.get_mut("membership") {
                *slot = membership.clone();
                return true;
            }
            map.values_mut()
                .any(|child| replace_membership(child, membership))
        }
        serde_json::Value::Array(items) => items
            .iter_mut()
            .any(|child| replace_membership(child, membership)),
        _ => false,
    }
}

/// Attack: a writer without any space key uploads a change whose membership
/// points to an object that never exists. It must not stop honest publication.
#[test]
fn unauthenticated_object_with_missing_dependency_does_not_block_publication() {
    use crate::sync::protocol::Object;
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let mut entry = Entry::new();
    entry.title = "Before attack".into();
    vault.test_entries_mut().push(entry);
    vault.save_changes().unwrap();
    let factory = Arc::new(MemoryFactory::default());
    let mut session = SpaceSync::new(Some(factory.clone()));
    assert_eq!(drive(&mut session, &mut vault), None);

    let genesis = vault.sync_state().unwrap().genesis;
    let change = factory
        .objects
        .lock()
        .unwrap()
        .iter()
        .find(|(hash, _)| **hash != genesis)
        .map(|(_, bytes)| bytes.clone())
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&change).unwrap();
    let unknown = serde_json::to_value(Hash([9; 32])).unwrap();
    assert!(replace_membership(&mut value, &unknown));
    let forged: Object = serde_json::from_value(value).unwrap();
    let forged_hash = forged.hash().unwrap();
    factory
        .objects
        .lock()
        .unwrap()
        .insert(forged_hash, forged.bytes().unwrap());

    let mut entry = Entry::new();
    entry.title = "After attack".into();
    vault.test_entries_mut().push(entry);
    vault.save_changes().unwrap();
    session.saved();

    let published = drive_until(&mut session, &mut vault, |session, vault| {
        vault.sync_state().unwrap().outbox.is_empty()
            && matches!(session.status, Status::Synced | Status::Conflict)
    });
    assert!(
        published,
        "honest edit stuck behind forged object, status {:?}",
        session.status
    );
    assert!(
        !vault
            .sync_state()
            .unwrap()
            .objects
            .contains_key(&forged_hash)
    );
}

/// Attack: a writer uploads bytes that are not a valid object under some key.
/// The pass must skip it, remember it and still publish honest edits.
#[test]
fn undecodable_listed_object_is_skipped_without_failing_exchange() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let factory = Arc::new(MemoryFactory::default());
    let mut session = SpaceSync::new(Some(factory.clone()));
    assert_eq!(drive(&mut session, &mut vault), None);

    let junk = Hash([8; 32]);
    factory
        .objects
        .lock()
        .unwrap()
        .insert(junk, b"not an object".to_vec());

    let mut entry = Entry::new();
    entry.title = "After junk".into();
    vault.test_entries_mut().push(entry);
    vault.save_changes().unwrap();
    session.saved();

    let published = drive_until(&mut session, &mut vault, |session, vault| {
        vault.sync_state().unwrap().outbox.is_empty()
            && matches!(session.status, Status::Synced | Status::Conflict)
    });
    assert!(
        published,
        "honest edit stuck behind junk object, status {:?}",
        session.status
    );
    // Junk under a key is a bad copy; it must never condemn the hash itself.
    assert!(
        !vault
            .sync_state()
            .unwrap()
            .rejected_candidates
            .contains_key(&junk)
    );
}

/// Attack: a writer floods the namespace with more junk keys than one bounded
/// pass can fetch. Honest local edits must still reach the provider.
#[test]
fn junk_flood_beyond_the_pass_budget_does_not_block_publication() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let factory = Arc::new(MemoryFactory::default());
    let mut session = SpaceSync::new(Some(factory.clone()));
    assert_eq!(drive(&mut session, &mut vault), None);

    {
        let mut objects = factory.objects.lock().unwrap();
        for index in 0u32..600 {
            let mut key = [0xA0; 32];
            key[..4].copy_from_slice(&index.to_be_bytes());
            objects.insert(Hash(key), b"junk".to_vec());
        }
    }

    let mut entry = Entry::new();
    entry.title = "Under flood".into();
    vault.test_entries_mut().push(entry);
    vault.save_changes().unwrap();
    session.saved();

    let published = drive_until(&mut session, &mut vault, |_, vault| {
        vault.sync_state().unwrap().outbox.is_empty()
    });
    assert!(
        published,
        "honest edit stuck behind junk flood, status {:?}",
        session.status
    );
}
