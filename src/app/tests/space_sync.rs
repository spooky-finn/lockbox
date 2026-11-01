//! Application orchestration tests through fake transports and ready mailboxes.

use super::*;
use crate::{
    core::{
        entry::Entry,
        sync::{Machine, Transaction},
    },
    infra::vault::{KdfParams, ProviderConfiguration},
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
    connections: AtomicUsize,
    fail_once: AtomicBool,
}

impl StorageFactory for MemoryFactory {
    fn connect(&self) -> sync::Result<Box<dyn sync::Storage + Send>> {
        self.connections.fetch_add(1, Ordering::Relaxed);
        if self.fail_once.swap(false, Ordering::Relaxed) {
            return Err(TransportError::AccessDenied);
        }
        Ok(Box::new(MemoryStorage(self.objects.clone())))
    }
}

struct MemoryStorage(Arc<Mutex<BTreeMap<Hash, Vec<u8>>>>);

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
    Vault::create_shared(
        dir.path().join("profile.lbx"),
        &SecretString::from("master".to_owned()),
        &SecretString::from("recovery".to_owned()),
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
        "Alice".into(),
        "Team".into(),
        ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "test-bucket".into(),
            access_key: "access".into(),
            secret_key: "secret".into(),
        },
    )
    .unwrap()
}

#[test]
fn downloaded_history_waits_until_the_active_draft_is_finished() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(&dir);
    let profile = vault.profile.as_ref().unwrap();
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
        objects: remote.objects.into_iter().map(|(h, b)| (h, b.0)).collect(),
        incomplete: true,
    };
    let mut session = SpaceSync {
        worker: Some(Worker::ready(Ok(Reply::Downloaded(batch)))),
        ..SpaceSync::default()
    };
    assert!(!session.needs_tick(true));
    assert!(session.tick(&mut vault, true).is_none());
    assert!(vault.entries.is_empty());
    assert!(session.worker.is_some());
    assert!(session.tick(&mut vault, false).is_none());
    assert_eq!(vault.entries[0].title, "Remote");
    assert_eq!(session.status, Status::Incomplete);
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
        .profile
        .as_ref()
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
    vault.entries.push(entry);
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
            .profile
            .as_ref()
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
    let directory = tempfile::tempdir().unwrap();
    let mut vault = vault(&directory);
    let mut entry = Entry::new();
    entry.title = "Local fixture".into();
    vault.entries.push(entry);
    vault.save_changes().unwrap();
    let factory = Arc::new(MemoryFactory::default());
    let mut session = SpaceSync::new(Some(factory.clone()));
    assert_eq!(drive(&mut session, &mut vault), None);
    assert_eq!(factory.connections.load(Ordering::Relaxed), 2);
    assert_eq!(factory.objects.lock().unwrap().len(), 2);
    assert!(vault.sync_state().unwrap().outbox.is_empty());
    assert!(vault.sync_state().unwrap().last_exchange.is_some());
    assert_eq!(vault.entries[0].title, "Local fixture");
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
    vault.profile.as_mut().unwrap().sync.as_mut().unwrap().fault =
        Some(crate::core::sync::Error::ControlFork);
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
        .profile
        .as_mut()
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
    let profile = vault.profile.as_ref().unwrap();
    let device = profile.sync_secret();
    let mut remote = profile.sync.as_ref().unwrap().clone();
    remote.intent = Some(
        Machine::prepare(
            &remote,
            &device,
            Transaction {
                mutations: vec![],
                rebased_from: None,
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
