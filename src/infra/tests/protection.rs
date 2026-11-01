//! Exercise the real encrypted file boundaries with an independently held witness.

use super::*;
use crate::{
    core::sync::{Hash, Machine, Role},
    infra::{
        sync::{Download, Page, Storage, TransportError},
        witness::{DirectoryWitness, MemoryWitness},
    },
};

const FAST: KdfParams = KdfParams {
    memory_kib: 8192,
    passes: 1,
    lanes: 1,
};

fn password() -> SecretString {
    SecretString::from("fixture master".to_owned())
}

fn create(store: &ProfileStore, path: &Path) -> Vault {
    store
        .create_shared(
            path,
            &password(),
            &SecretString::from("fixture recovery".to_owned()),
            FAST,
            SharedProfile {
                username: "Fixture user".into(),
                space_name: "Fixture space".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "fixture-bucket".into(),
                    access_key: "fixture-access".into(),
                    secret_key: "fixture-secret".into(),
                },
            },
        )
        .unwrap()
}

fn add(vault: &mut Vault, title: &str) {
    let mut entry = Entry::new();
    entry.title = title.into();
    entry.password = "fictional secret".into();
    vault.entries.push(entry);
    vault.save_changes().unwrap();
}

fn device(vault: &Vault) -> Hash {
    vault.device_certificate().unwrap().body.device
}

fn delivery(vault: &Vault) -> Download {
    Download {
        objects: vault
            .sync_state()
            .unwrap()
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
        incomplete: false,
    }
}

struct NoNetwork;

impl Storage for NoNetwork {
    fn list(&mut self, _: Hash, _: Option<&str>) -> std::result::Result<Page, TransportError> {
        panic!("blocked profile performed LIST")
    }
    fn get(&mut self, _: Hash, _: Hash) -> std::result::Result<Vec<u8>, TransportError> {
        panic!("blocked profile performed GET")
    }
    fn put(&mut self, _: Hash, _: Hash, _: &[u8]) -> std::result::Result<(), TransportError> {
        panic!("blocked profile performed PUT")
    }
}

#[test]
fn backup_rollback_remains_readable_and_cannot_sign_or_touch_transport() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.lbx");
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses);
    let mut vault = create(&store, &path);
    add(&mut vault, "Earlier snapshot");
    let backup = fs::read(&path).unwrap();
    vault.entries[0].title = "Latest snapshot".into();
    vault.save_changes().unwrap();
    let key = vault.export_key();
    drop(vault);
    fs::write(&path, &backup).unwrap();
    let mut restored = store.open(&path, &password()).unwrap();
    assert_eq!(restored.entries[0].title, "Earlier snapshot");
    assert_eq!(restored.signing_error(), Some(anchor::Error::Rollback));
    let objects = restored.sync_state().unwrap().objects.clone();
    restored.entries[0].title = "Must not sign".into();
    assert!(matches!(
        restored.save_changes(),
        Err(VaultError::ProfileProtection(anchor::Error::Rollback))
    ));
    assert!(matches!(
        restored.synchronize(&mut NoNetwork),
        Err(VaultError::ProfileProtection(anchor::Error::Rollback))
    ));
    assert_eq!(restored.sync_state().unwrap().objects, objects);
    assert!(restored.sync_state().unwrap().intent.is_none());
    assert_eq!(fs::read(&path).unwrap(), backup);
    drop(restored);
    let by_key = store.open_with_key(&path, &key).unwrap();
    assert_eq!(by_key.signing_error(), Some(anchor::Error::Rollback));
}

#[test]
fn another_installation_cannot_enroll_the_imported_signer_implicitly() {
    let directory = tempfile::tempdir().unwrap();
    let original_store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("profile.lbx");
    let mut original = create(&original_store, &path);
    add(&mut original, "Readable backup");
    let id = device(&original);
    drop(original);
    let fresh_witnesses = Arc::new(MemoryWitness::default());
    let fresh_store = ProfileStore::new(fresh_witnesses.clone());
    let mut imported = fresh_store.open(&path, &password()).unwrap();
    assert_eq!(imported.entries[0].title, "Readable backup");
    assert_eq!(
        imported.signing_error(),
        Some(anchor::Error::MissingBinding)
    );
    for _ in 0..2 {
        assert!(matches!(
            imported.reconcile_profile(),
            Err(VaultError::ProfileProtection(anchor::Error::MissingBinding))
        ));
        assert!(imported.synchronize(&mut NoNetwork).is_err());
    }
    assert!(fresh_witnesses.snapshot(id).is_none());
}

#[test]
fn simultaneous_copies_contend_for_one_device_lease_and_a_move_is_allowed() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("original.lbx");
    let copy = directory.path().join("copy.lbx");
    let vault = create(&store, &path);
    fs::copy(&path, &copy).unwrap();
    assert!(matches!(
        store.open(&copy, &password()),
        Err(VaultError::ProfileProtection(anchor::Error::InUse))
    ));
    drop(vault);
    fs::remove_file(&copy).unwrap();
    fs::rename(&path, &copy).unwrap();
    let moved = store.open(&copy, &password()).unwrap();
    assert!(moved.signing_error().is_none());
    moved.save().unwrap();
}

#[test]
fn a_crash_after_file_commit_catches_up_exactly_one_witness_generation() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let path = directory.path().join("profile.lbx");
    let mut vault = create(&store, &path);
    add(&mut vault, "Durable before crash");
    let id = device(&vault);
    let before = witnesses.snapshot(id).unwrap();
    witnesses.fail_writes(1);
    assert!(matches!(
        vault.save(),
        Err(VaultError::ProfileProtection(anchor::Error::Unavailable))
    ));
    assert_eq!(witnesses.snapshot(id), Some(before));
    assert_eq!(vault.signing_error(), Some(anchor::Error::Unavailable));
    drop(vault);
    let reopened = store.open(&path, &password()).unwrap();
    assert!(reopened.signing_error().is_none());
    assert_eq!(reopened.entries[0].title, "Durable before crash");
    let after = witnesses.snapshot(id).unwrap();
    assert_eq!(after.generation, before.generation + 1);
    assert_eq!(after.digest, Some(protection::digest_file(&path).unwrap()));
}

#[test]
fn an_unwitnessed_intent_does_not_sign_and_restart_completes_its_exact_body() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let path = directory.path().join("profile.lbx");
    let mut vault = create(&store, &path);
    let mut entry = Entry::new();
    entry.title = "Reserved edit".into();
    vault.entries.push(entry);
    witnesses.fail_writes(1);
    assert!(vault.save_changes().is_err());
    assert_eq!(vault.sync_state().unwrap().objects.len(), 1);
    let mut expected = vault.sync_state().unwrap().clone();
    assert!(expected.intent.is_some());
    let hash = Machine::complete_intent(
        &mut expected,
        &vault.profile.as_ref().unwrap().sync_secret(),
    )
    .unwrap();
    let file = fs::read(&path).unwrap();
    witnesses.fail_writes(10);
    assert!(vault.recover_sync().is_err());
    assert_eq!(vault.sync_state().unwrap().objects.len(), 1);
    assert_eq!(fs::read(&path).unwrap(), file);
    assert!(vault.synchronize(&mut NoNetwork).is_err());
    drop(vault);
    witnesses.fail_writes(0);
    let reopened = store.open(&path, &password()).unwrap();
    assert_eq!(reopened.entries[0].title, "Reserved edit");
    assert!(reopened.sync_state().unwrap().intent.is_none());
    assert_eq!(
        reopened.sync_state().unwrap().objects[&hash],
        expected.objects[&hash]
    );
}

#[test]
fn witness_denial_preserves_readable_data_and_recovers_without_reenrollment() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let path = directory.path().join("profile.lbx");
    let mut vault = create(&store, &path);
    add(&mut vault, "Keep readable");
    let id = device(&vault);
    let before = witnesses.snapshot(id);
    drop(vault);
    witnesses.fail_reads(true);
    let mut blocked = store.open(&path, &password()).unwrap();
    assert_eq!(blocked.entries[0].title, "Keep readable");
    assert_eq!(blocked.signing_error(), Some(anchor::Error::Unavailable));
    assert!(blocked.synchronize(&mut NoNetwork).is_err());
    witnesses.fail_reads(false);
    blocked.reconcile_profile().unwrap();
    assert!(blocked.signing_error().is_none());
    assert_eq!(witnesses.snapshot(id), before);
}

#[test]
fn a_lost_witness_acknowledgement_does_not_repeat_or_reset_the_generation() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let vault = create(&store, &directory.path().join("profile.lbx"));
    let id = device(&vault);
    let before = witnesses.snapshot(id).unwrap();
    witnesses.lose_writes(1);
    assert!(vault.save().is_err());
    let committed = witnesses.snapshot(id).unwrap();
    assert_eq!(committed.generation, before.generation + 1);
    vault.reconcile_profile().unwrap();
    assert_eq!(witnesses.snapshot(id), Some(committed));
    vault.save().unwrap();
    assert_eq!(
        witnesses.snapshot(id).unwrap().generation,
        committed.generation + 1
    );
}

#[test]
fn an_inconsistent_independent_witness_never_authorizes_a_write() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let path = directory.path().join("profile.lbx");
    let mut vault = create(&store, &path);
    add(&mut vault, "Keep the profile");
    let id = device(&vault);
    let mut wrong = witnesses.snapshot(id).unwrap();
    wrong.digest = Some(Hash([99; 32]));
    witnesses.restore(id, Some(wrong));
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        vault.save(),
        Err(VaultError::ProfileProtection(anchor::Error::Rollback))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(vault.synchronize(&mut NoNetwork).is_err());
}

#[test]
fn received_revocation_survives_a_crash_before_its_witness_acknowledgement() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let mut owner = create(&store, &directory.path().join("owner.lbx"));
    let path = directory.path().join("editor.lbx");
    let mut editor = create(&store, &path);
    add(&mut owner, "Readable before removal");
    owner
        .set_space_members(vec![
            (owner.device_certificate().unwrap(), Role::Owner),
            (editor.device_certificate().unwrap(), Role::Editor),
        ])
        .unwrap();
    editor
        .join_space(owner.space_descriptor().unwrap())
        .unwrap();
    editor.receive_sync(delivery(&owner)).unwrap();
    owner
        .set_space_members(vec![(owner.device_certificate().unwrap(), Role::Owner)])
        .unwrap();
    witnesses.fail_writes(1);
    assert!(editor.receive_sync(delivery(&owner)).is_err());
    assert!(editor.has_pending_exchange());
    assert_eq!(editor.signing_error(), Some(anchor::Error::Unavailable));
    witnesses.fail_writes(10);
    assert!(editor.synchronize(&mut NoNetwork).is_err());
    drop(editor);
    witnesses.fail_writes(0);
    let reopened = store.open(&path, &password()).unwrap();
    assert_eq!(reopened.entries[0].title, "Readable before removal");
    assert!(reopened.sync_replay().unwrap().unwrap().role.is_none());
    assert!(reopened.signing_error().is_none());
}

#[test]
fn directory_witness_is_persistent_and_corruption_is_not_absence() {
    let directory = tempfile::tempdir().unwrap();
    let witness_root = directory.path().join("independent-installation");
    let path = directory.path().join("profile.lbx");
    let store = ProfileStore::new(Arc::new(DirectoryWitness::new(witness_root.clone())));
    let mut vault = create(&store, &path);
    add(&mut vault, "Persisted witness");
    let id = device(&vault);
    let second_store = ProfileStore::new(Arc::new(DirectoryWitness::new(witness_root.clone())));
    let copy = directory.path().join("copy.lbx");
    fs::copy(&path, &copy).unwrap();
    assert!(matches!(
        second_store.open(&copy, &password()),
        Err(VaultError::ProfileProtection(anchor::Error::InUse))
    ));
    drop(vault);
    let reopened = second_store.open(&path, &password()).unwrap();
    assert!(reopened.signing_error().is_none());
    drop(reopened);
    fs::write(witness_root.join(format!("{id}.json")), b"invalid witness").unwrap();
    let blocked = second_store.open(&path, &password()).unwrap();
    assert_eq!(blocked.entries[0].title, "Persisted witness");
    assert_eq!(blocked.signing_error(), Some(anchor::Error::Unavailable));
}

#[test]
fn authenticated_reencryption_and_unexplained_generation_gaps_are_detected() {
    for gap in [0, 2] {
        let directory = tempfile::tempdir().unwrap();
        let witnesses = Arc::new(MemoryWitness::default());
        let store = ProfileStore::new(witnesses.clone());
        let path = directory.path().join("profile.lbx");
        let mut vault = create(&store, &path);
        add(&mut vault, "Authenticated readable state");
        let data = fs::read(&path).unwrap();
        let mut contents = decrypt(&data, &vault.key).unwrap();
        if gap > 0 {
            let frame = contents.generation.as_mut().unwrap();
            frame.generation += gap;
            frame.previous = witnesses.snapshot(device(&vault)).unwrap().digest;
        }
        let header = vault.header(&[0x71; NONCE_LEN]);
        let plaintext = Zeroizing::new(serde_json::to_vec(&contents).unwrap());
        let encrypted = cipher(&vault.key)
            .encrypt(
                XNonce::from_slice(&header[66..]),
                Payload {
                    msg: &plaintext,
                    aad: &header,
                },
            )
            .unwrap();
        let mut replacement = header.to_vec();
        replacement.extend(encrypted);
        drop(vault);
        fs::write(&path, replacement).unwrap();
        let reopened = store.open(&path, &password()).unwrap();
        assert_eq!(reopened.entries[0].title, "Authenticated readable state");
        assert_eq!(reopened.signing_error(), Some(anchor::Error::Rollback));
    }
}

#[test]
fn legacy_shared_profiles_open_read_only_without_creating_witnesses() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("legacy.lbx");
    let mut vault = create(&store, &path);
    add(&mut vault, "Legacy readable record");
    let data = fs::read(&path).unwrap();
    let contents = decrypt(&data, &vault.key).unwrap();
    let mut legacy = serde_json::to_value(&contents).unwrap();
    legacy["profile"]["schema_version"] = serde_json::json!(2);
    legacy["profile"]["sync"]
        .as_object_mut()
        .unwrap()
        .remove("required_control");
    legacy.as_object_mut().unwrap().remove("generation");
    let header = vault.header(&[0x72; NONCE_LEN]);
    let plaintext = Zeroizing::new(serde_json::to_vec(&legacy).unwrap());
    let encrypted = cipher(&vault.key)
        .encrypt(
            XNonce::from_slice(&header[66..]),
            Payload {
                msg: &plaintext,
                aad: &header,
            },
        )
        .unwrap();
    let mut bytes = header.to_vec();
    bytes.extend(encrypted);
    drop(vault);
    fs::write(&path, bytes).unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let fresh_store = ProfileStore::new(witnesses.clone());
    let mut legacy = fresh_store.open(&path, &password()).unwrap();
    assert_eq!(legacy.entries[0].title, "Legacy readable record");
    assert_eq!(legacy.signing_error(), Some(anchor::Error::Legacy));
    assert!(legacy.sync_replay().unwrap().unwrap().ready);
    assert!(legacy.save_changes().is_err());
    assert!(legacy.synchronize(&mut NoNetwork).is_err());
    assert!(witnesses.snapshot(device(&legacy)).is_none());
}

#[test]
fn backup_recovery_creates_a_fresh_certified_device_and_requires_new_admission() {
    let directory = tempfile::tempdir().unwrap();
    let old_store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = create(&old_store, &directory.path().join("owner.lbx"));
    let old_path = directory.path().join("old-device.lbx");
    let mut old = create(&old_store, &old_path);
    add(&mut owner, "Original shared record");
    owner
        .set_space_members(vec![
            (owner.device_certificate().unwrap(), Role::Owner),
            (old.device_certificate().unwrap(), Role::Editor),
        ])
        .unwrap();
    old.join_space(owner.space_descriptor().unwrap()).unwrap();
    old.receive_sync(delivery(&owner)).unwrap();
    let old_certificate = old.device_certificate().unwrap();
    drop(old);
    owner.entries[0].title = "Latest verified checkpoint".into();
    owner.save_changes().unwrap();
    owner
        .set_space_members(vec![
            (owner.device_certificate().unwrap(), Role::Owner),
            (old_certificate.clone(), Role::Editor),
        ])
        .unwrap();
    let anchor = owner.space_anchor().unwrap();
    let new_store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut source = new_store.open(&old_path, &password()).unwrap();
    assert_eq!(source.signing_error(), Some(anchor::Error::MissingBinding));
    assert_eq!(source.entries[0].title, "Original shared record");
    let target = directory.path().join("new-device.lbx");
    let configuration = || SharedProfile {
        username: "Recovered user".into(),
        space_name: "Team".into(),
        provider: ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "fixture-bucket".into(),
            access_key: "new-access".into(),
            secret_key: "new-secret".into(),
        },
    };
    assert!(matches!(
        new_store.restore_device(
            &target,
            &password(),
            FAST,
            DeviceRecovery {
                source: &source,
                recovery_password: &SecretString::from("wrong password".to_owned()),
                descriptor: owner.space_descriptor().unwrap(),
                control_anchor: anchor,
                configuration: configuration(),
            }
        ),
        Err(VaultError::RecoveryFailed)
    ));
    assert!(!target.exists());
    let mut recovered = new_store
        .restore_device(
            &target,
            &password(),
            FAST,
            DeviceRecovery {
                source: &source,
                recovery_password: &SecretString::from("fixture recovery".to_owned()),
                descriptor: owner.space_descriptor().unwrap(),
                control_anchor: anchor,
                configuration: configuration(),
            },
        )
        .unwrap();
    let certificate = recovered.device_certificate().unwrap();
    certificate.verify().unwrap();
    assert_eq!(certificate.body.user, old_certificate.body.user);
    assert_eq!(certificate.body.root_key, old_certificate.body.root_key);
    assert_ne!(certificate.body.device, old_certificate.body.device);
    assert_ne!(certificate.body.hpke_key, old_certificate.body.hpke_key);
    assert!(recovered.entries.is_empty());
    assert!(recovered.sync_state().unwrap().objects.is_empty());
    assert_eq!(
        recovered.sync_state().unwrap().required_control,
        Some(anchor)
    );
    recovered.receive_sync(delivery(&owner)).unwrap();
    assert!(!recovered.sync_replay().unwrap().unwrap().ready);
    assert!(recovered.sync_replay().unwrap().unwrap().role.is_none());
    let mut premature = Entry::new();
    premature.title = "Cannot publish before admission".into();
    recovered.entries.push(premature);
    assert!(recovered.save_changes().is_err());
    assert!(recovered.sync_state().unwrap().outbox.is_empty());
    assert!(recovered.sync_state().unwrap().intent.is_none());
    recovered.entries.clear();
    owner
        .set_space_members(vec![
            (owner.device_certificate().unwrap(), Role::Owner),
            (certificate.clone(), Role::Editor),
        ])
        .unwrap();
    recovered.receive_sync(delivery(&owner)).unwrap();
    assert_eq!(recovered.entries[0].title, "Latest verified checkpoint");
    assert!(!recovered.sync_replay().unwrap().unwrap().history_verified);
    recovered.entries[0].notes = "Explicit edit by the recovered device".into();
    recovered.save_changes().unwrap();
    owner.receive_sync(delivery(&recovered)).unwrap();
    assert_eq!(
        owner.entries[0].notes,
        "Explicit edit by the recovered device"
    );
    assert_eq!(
        owner.sync_replay().unwrap().unwrap().projection.records[0].versions[0].author,
        certificate.body.device
    );
    assert!(source.synchronize(&mut NoNetwork).is_err());
}

#[cfg(unix)]
#[test]
fn temporary_and_backup_sidecars_never_truncate_symlink_targets() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("profile.lbx");
    let vault = create(&store, &path);
    let independent = directory.path().join("independent.txt");
    fs::write(&independent, b"preserve this file").unwrap();
    for sidecar in [
        sibling(&path, "tmp"),
        sibling(&path, "bak"),
        sibling(&sibling(&path, "bak"), "tmp"),
    ] {
        std::os::unix::fs::symlink(&independent, sidecar).unwrap();
    }
    vault.save().unwrap();
    assert_eq!(fs::read(&independent).unwrap(), b"preserve this file");
    assert!(
        !fs::symlink_metadata(sibling(&path, "bak"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!sibling(&path, "tmp").exists());
    assert!(vault.signing_error().is_none());
}

#[test]
fn initial_enrollment_recovers_a_committed_file_from_its_creation_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let path = directory.path().join("initial.lbx");
    witnesses.fail_writes_after(1, 1);
    let result = store.create_shared(
        &path,
        &password(),
        &SecretString::from("recovery".to_owned()),
        FAST,
        SharedProfile {
            username: "Owner".into(),
            space_name: "Test".into(),
            provider: ProviderConfiguration {
                base_url: "https://gateway.storjshare.io".into(),
                bucket: "fixture-bucket".into(),
                access_key: "access".into(),
                secret_key: "secret".into(),
            },
        },
    );
    assert!(matches!(
        result,
        Err(VaultError::ProfileProtection(anchor::Error::Unavailable))
    ));
    assert!(path.exists());
    let vault = store.open(&path, &password()).unwrap();
    assert!(vault.signing_error().is_none());
    assert_eq!(witnesses.snapshot(device(&vault)).unwrap().generation, 1);
    assert_eq!(vault.sync_state().unwrap().objects.len(), 1);
    assert!(
        vault
            .sync_state()
            .unwrap()
            .outbox
            .contains(&vault.space_anchor().unwrap())
    );
}

#[test]
fn directory_signer_lease_is_exclusive_across_processes() {
    use crate::infra::witness::WitnessStore;
    const CHILD_ROOT: &str = "LOCKBOX_TEST_WITNESS_PROCESS_ROOT";
    const CHILD_BLOCKED: &str = "LOCKBOX_TEST_WITNESS_PROCESS_BLOCKED";
    let device = Hash([0x42; 32]);
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let result = DirectoryWitness::new(root.into()).acquire(device);
        if std::env::var_os(CHILD_BLOCKED).is_some() {
            assert!(matches!(result, Err(anchor::Error::InUse)));
        } else {
            assert!(result.is_ok());
        }
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let store = DirectoryWitness::new(directory.path().to_owned());
    let lease = store.acquire(device).unwrap();
    let child = |blocked: bool| {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "infra::vault::protection_tests::directory_signer_lease_is_exclusive_across_processes"])
            .env(CHILD_ROOT, directory.path()).env_remove(CHILD_BLOCKED);
        if blocked {
            command.env(CHILD_BLOCKED, "1");
        }
        let output = command.output().unwrap();
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "signer lease child did not execute its assertion"
        );
        assert!(
            output.status.success(),
            "signer lease child process failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    child(true);
    drop(lease);
    child(false);
}

#[cfg(unix)]
#[test]
fn deleting_a_signer_lock_does_not_leave_its_old_lease_usable() {
    use crate::infra::witness::WitnessStore;
    let directory = tempfile::tempdir().unwrap();
    let device = Hash([0x43; 32]);
    let store = DirectoryWitness::new(directory.path().to_owned());
    let old = store.acquire(device).unwrap();
    fs::remove_file(directory.path().join(format!("{device}.lock"))).unwrap();
    let current = store.acquire(device).unwrap();
    assert_eq!(old.load(), Err(anchor::Error::Unavailable));
    assert!(current.load().unwrap().is_none());
}
