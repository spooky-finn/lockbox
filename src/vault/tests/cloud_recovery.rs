use super::*;
use crate::{
    access::mnemonic::RecoveryPhrase,
    records::entry::Entry,
    sync::transport::{Page, Result as TransportResult},
    vault::{SharedProfile, witness::MemoryWitness},
};
use std::sync::Arc;

const FAST: KdfParams = KdfParams {
    memory_kib: 8192,
    passes: 1,
    lanes: 1,
};
fn password() -> SecretString {
    SecretString::from("fictional master password".to_owned())
}
fn provider() -> ProviderConfiguration {
    ProviderConfiguration {
        base_url: "https://gateway.storjshare.io".into(),
        bucket: "fictional-bucket".into(),
        access_key: "fictional-access".into(),
        secret_key: "fictional-secret".into(),
    }
}
fn store() -> ProfileStore {
    ProfileStore::new(Arc::new(MemoryWitness::default()))
}
#[derive(Clone, Default)]
struct Memory {
    objects: BTreeMap<(Hash, Hash), Vec<u8>>,
    backups: BTreeMap<(Hash, Hash), Vec<u8>>,
    /// Listed recovery keys whose GET fails, as a hostile bucket can arrange.
    recovery_errors: BTreeMap<Hash, TransportError>,
    fail_backup: bool,
    hide_objects: bool,
}
impl Storage for Memory {
    fn list(&mut self, namespace: Hash, _: Option<&str>) -> TransportResult<Page> {
        Ok(Page {
            objects: if self.hide_objects {
                vec![]
            } else {
                self.objects
                    .keys()
                    .filter(|(n, _)| *n == namespace)
                    .map(|(_, h)| *h)
                    .rev()
                    .collect()
            },
            next: None,
        })
    }
    fn get(&mut self, namespace: Hash, hash: Hash) -> TransportResult<Vec<u8>> {
        self.objects
            .get(&(namespace, hash))
            .cloned()
            .ok_or(TransportError::NotFound)
    }
    fn put(&mut self, namespace: Hash, hash: Hash, bytes: &[u8]) -> TransportResult<()> {
        self.objects.insert((namespace, hash), bytes.to_vec());
        Ok(())
    }
    fn list_recovery(&mut self, directory: Hash, cursor: Option<&str>) -> TransportResult<Page> {
        // Pages of at most 1,000 keys, as the trait requires.
        let start = cursor.map_or(0, |cursor| cursor.parse().unwrap());
        let keys: Vec<_> = self
            .backups
            .keys()
            .filter(|(d, _)| *d == directory)
            .map(|(_, h)| *h)
            .rev()
            .collect();
        let end = keys.len().min(start + 1000);
        Ok(Page {
            objects: keys[start..end].to_vec(),
            next: (end < keys.len()).then(|| end.to_string()),
        })
    }
    fn get_recovery(&mut self, directory: Hash, hash: Hash) -> TransportResult<Vec<u8>> {
        if let Some(error) = self.recovery_errors.get(&hash) {
            return Err(*error);
        }
        self.backups
            .get(&(directory, hash))
            .cloned()
            .ok_or(TransportError::NotFound)
    }
    fn put_recovery(&mut self, directory: Hash, hash: Hash, bytes: &[u8]) -> TransportResult<()> {
        if self.fail_backup {
            return Err(TransportError::Unavailable);
        }
        self.backups.insert((directory, hash), bytes.to_vec());
        Ok(())
    }
}
fn create(path: &Path, phrase: &RecoveryPhrase, name: &str) -> Vault {
    store()
        .create_shared_with_phrase(
            path,
            &password(),
            FAST,
            SharedProfile {
                username: "Owner".into(),
                space_name: name.into(),
                provider: provider(),
            },
            phrase,
        )
        .unwrap()
}
fn discover(root: &SigningKey, storage: &mut Memory) -> Vec<CloudSpace> {
    ProfileStore::discover_cloud_spaces(root, storage, &Cancellation::default()).unwrap()
}
fn restore(
    path: &Path,
    phrase: &RecoveryPhrase,
    space: CloudSpace,
    storage: &mut Memory,
) -> Result<Vault> {
    store().restore_cloud_space(
        path,
        &password(),
        FAST,
        CloudRestore {
            root: &phrase.signing_key(),
            space,
            provider: provider(),
            phrase: Some(phrase),
            trust: CloudRecoveryTrust::TrustProviderSnapshot,
            cancel: Cancellation::default(),
        },
        storage,
    )
}
fn add(vault: &mut Vault, title: &str) -> Entry {
    let mut entry = Entry::new();
    entry.title = title.into();
    entry.password = "secret account value".into();
    vault.upsert_entry(entry.clone()).unwrap();
    entry
}

#[test]
fn fresh_install_recovers_without_old_profile_identity_witness_or_device_secrets() {
    let original = tempfile::tempdir().unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let phrase = RecoveryPhrase::generate().unwrap();
    let root = phrase.signing_key();
    let mut storage = Memory::default();
    let mut owner = create(&original.path().join("owner.lbx"), &phrase, "Private space");
    let old_device = owner.device_certificate().unwrap().body.device;
    let folder = crate::records::folder::Folder::new("Recovered folder".into(), None);
    owner.save_folder(folder.clone()).unwrap();
    let mut entry = add(&mut owner, "Account");
    entry.folder_id = Some(folder.id.clone());
    entry
        .add_attachment("backup.txt".into(), b"private attachment".to_vec())
        .unwrap();
    owner.upsert_entry(entry.clone()).unwrap();
    let deleted = add(&mut owner, "Deleted account");
    owner.delete_entry(&deleted.id).unwrap();
    owner.synchronize(&mut storage).unwrap();
    let expected = owner.sync_replay().unwrap().unwrap();
    let old_anchor = owner.recovery_anchor().unwrap();
    drop(owner);
    original.close().unwrap();
    let space = discover(&root, &mut storage).remove(0);
    storage.hide_objects = true; // Known frontier/dependencies must work without LIST.
    let path = fresh.path().join("restored.lbx");
    let mut recovered = restore(&path, &phrase, space, &mut storage).unwrap();
    assert_ne!(
        recovered.device_certificate().unwrap().body.device,
        old_device
    );
    assert_eq!(recovered.entries(), &[entry]);
    assert_eq!(recovered.folders(), &[folder]);
    let actual = recovered.sync_replay().unwrap().unwrap();
    assert!(actual.projection == expected.projection);
    assert_eq!(actual.history.len(), expected.history.len());
    assert!(actual.history_verified);
    assert_eq!(recovered.trash().unwrap()[0].id, deleted.id);
    assert_ne!(
        recovered.recovery_anchor().unwrap().control,
        old_anchor.control
    );
    add(&mut recovered, "After recovery");
    recovered.synchronize(&mut storage).unwrap();
    drop(recovered);
    let reopened = store(); // Different installation must remain read-only.
    let readable = reopened.open(&path, &password()).unwrap();
    assert_eq!(readable.entries().len(), 2);
    assert!(readable.signing_error().is_some());
    drop(readable);
    let again = tempfile::tempdir().unwrap();
    let second = restore(
        &again.path().join("again.lbx"),
        &phrase,
        discover(&root, &mut storage).remove(0),
        &mut storage,
    )
    .unwrap();
    assert_eq!(second.entries().len(), 2);
    assert!(second.sync_replay().unwrap().unwrap().history_verified);
}

#[test]
fn failed_backup_reuses_persisted_capsule_across_restart_and_does_not_claim_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owner.lbx");
    let phrase = RecoveryPhrase::generate().unwrap();
    let mut storage = Memory::default();
    let profiles = store();
    let mut owner = profiles
        .create_shared_with_phrase(
            &path,
            &password(),
            FAST,
            SharedProfile {
                username: "Owner".into(),
                space_name: "Space".into(),
                provider: provider(),
            },
            &phrase,
        )
        .unwrap();
    add(&mut owner, "Published before capsule");
    storage.fail_backup = true;
    assert!(owner.synchronize(&mut storage).is_err());
    let pending = owner
        .profile
        .as_ref()
        .unwrap()
        .cloud_backup
        .as_ref()
        .unwrap()
        .upload
        .clone()
        .unwrap();
    assert!(!pending.acknowledged);
    assert!(storage.backups.is_empty());
    drop(owner);
    let mut owner = profiles.open(&path, &password()).unwrap();
    add(&mut owner, "Edited while backup offline");
    storage.fail_backup = false;
    owner.synchronize(&mut storage).unwrap();
    assert!(
        storage
            .backups
            .values()
            .any(|bytes| bytes == &pending.bytes.0)
    );
    owner.synchronize(&mut storage).unwrap();
    assert_eq!(storage.backups.len(), 2);
    owner.synchronize(&mut storage).unwrap();
    assert_eq!(storage.backups.len(), 2);
    assert_eq!(discover(&phrase.signing_key(), &mut storage).len(), 1);
}

#[test]
fn wrong_root_missing_data_and_stale_independent_anchor_never_create_replacement_files() {
    let dir = tempfile::tempdir().unwrap();
    let phrase = RecoveryPhrase::generate().unwrap();
    let mut storage = Memory::default();
    let mut owner = create(&dir.path().join("owner.lbx"), &phrase, "Space");
    let stale = owner.recovery_anchor().unwrap();
    add(&mut owner, "Current");
    owner.synchronize(&mut storage).unwrap();
    assert!(matches!(
        ProfileStore::discover_cloud_spaces(
            &SigningKey::from_bytes(&[9; 32]),
            &mut storage,
            &Cancellation::default()
        ),
        Err(VaultError::CloudRecoveryMissing)
    ));
    let path = dir.path().join("must-not-exist.lbx");
    let space = discover(&phrase.signing_key(), &mut storage).remove(0);
    assert!(
        store()
            .restore_cloud_space(
                &path,
                &password(),
                FAST,
                CloudRestore {
                    root: &phrase.signing_key(),
                    space: space.clone(),
                    provider: provider(),
                    phrase: Some(&phrase),
                    trust: CloudRecoveryTrust::Independent(stale),
                    cancel: Cancellation::default(),
                },
                &mut storage
            )
            .is_err()
    );
    assert!(!path.exists());
    storage.objects.clear();
    assert!(restore(&path, &phrase, space, &mut storage).is_err());
    assert!(!path.exists());
}

#[test]
fn discovery_separates_multiple_spaces_and_rejects_competing_signed_capsules() {
    let dir = tempfile::tempdir().unwrap();
    let phrase = RecoveryPhrase::generate().unwrap();
    let mut storage = Memory::default();
    let mut one = create(&dir.path().join("one.lbx"), &phrase, "First");
    let mut two = create(&dir.path().join("two.lbx"), &phrase, "Second");
    one.synchronize(&mut storage).unwrap();
    two.synchronize(&mut storage).unwrap();
    assert_eq!(discover(&phrase.signing_key(), &mut storage).len(), 2);
    let profile = one.profile.as_ref().unwrap();
    let mut backup = profile.cloud_backup.clone().unwrap();
    backup.upload = None; // Independent initial publication creates a competing head.
    let forged_branch = capsule::prepare(
        one.sync_state().unwrap(),
        &profile.sync_secret(),
        &backup,
        "Owner",
        "First",
    )
    .unwrap()
    .unwrap();
    storage
        .put_recovery(
            backup.target.body.directory,
            forged_branch.hash,
            &forged_branch.bytes.0,
        )
        .unwrap();
    assert!(matches!(
        ProfileStore::discover_cloud_spaces(
            &phrase.signing_key(),
            &mut storage,
            &Cancellation::default()
        ),
        Err(VaultError::CloudRecoveryConflict)
    ));
}

#[test]
fn controller_handoff_continues_recovery_and_inherits_available_old_history() {
    use crate::{sync::protocol::Role, vault::DeviceRecovery};
    let dir = tempfile::tempdir().unwrap();
    let phrase = RecoveryPhrase::generate().unwrap();
    let mut storage = Memory::default();
    let profiles = store();
    let mut old = profiles
        .create_shared_with_phrase(
            dir.path().join("old.lbx"),
            &password(),
            FAST,
            SharedProfile {
                username: "Owner".into(),
                space_name: "Space".into(),
                provider: provider(),
            },
            &phrase,
        )
        .unwrap();
    let original = add(&mut old, "Before second device");
    old.synchronize(&mut storage).unwrap();
    let mut peer = profiles
        .restore_device(
            dir.path().join("peer.lbx"),
            &password(),
            FAST,
            DeviceRecovery {
                source: &old,
                recovery_password: &password(),
                descriptor: old.space_descriptor().unwrap(),
                control_anchor: old.space_anchor().unwrap(),
                configuration: SharedProfile {
                    username: "Other owner".into(),
                    space_name: "Space".into(),
                    provider: provider(),
                },
            },
        )
        .unwrap();
    let invitation = old
        .issue_invitation(peer.device_certificate().unwrap(), Role::Owner, None)
        .unwrap();
    let acceptance = peer
        .accept_invitation(invitation, &old.space_descriptor().unwrap())
        .unwrap();
    peer.join_invitation(old.admit_invitation(acceptance, 0).unwrap())
        .unwrap();
    old.synchronize(&mut storage).unwrap();
    peer.synchronize(&mut storage).unwrap();
    let offer = old
        .begin_control_transfer(peer.device_certificate().unwrap().body.device)
        .unwrap();
    let approval = peer.accept_control_transfer(&offer).unwrap();
    old.finish_control_transfer(approval).unwrap();
    old.synchronize(&mut storage).unwrap();
    peer.synchronize(&mut storage).unwrap();
    assert!(peer.enable_cloud_recovery(&password()).unwrap());
    peer.synchronize(&mut storage).unwrap();
    let spaces = discover(&phrase.signing_key(), &mut storage);
    assert_eq!(spaces.len(), 1);
    let restored = restore(
        &dir.path().join("restored.lbx"),
        &phrase,
        spaces.into_iter().next().unwrap(),
        &mut storage,
    )
    .unwrap();
    assert_eq!(restored.entries(), &[original]);
    assert!(restored.sync_replay().unwrap().unwrap().history_verified);
    assert!(!restored.record_history().unwrap().is_empty());
}

#[test]
fn cancelled_restore_and_failed_local_backup_commit_publish_no_new_recovery_objects() {
    let dir = tempfile::tempdir().unwrap();
    let phrase = RecoveryPhrase::generate().unwrap();
    let mut storage = Memory::default();
    let path = dir.path().join("owner.lbx");
    let mut owner = create(&path, &phrase, "Space");
    add(&mut owner, "Initial");
    owner.synchronize(&mut storage).unwrap();
    let space = discover(&phrase.signing_key(), &mut storage).remove(0);
    let cancel = Cancellation::default();
    cancel.cancel();
    let target = dir.path().join("cancelled.lbx");
    assert!(
        store()
            .restore_cloud_space(
                &target,
                &password(),
                FAST,
                CloudRestore {
                    root: &phrase.signing_key(),
                    space,
                    provider: provider(),
                    phrase: Some(&phrase),
                    trust: CloudRecoveryTrust::TrustProviderSnapshot,
                    cancel,
                },
                &mut storage
            )
            .is_err()
    );
    assert!(!target.exists());
    add(&mut owner, "Changed");
    let temporary = super::super::file::sibling(&path, "tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert!(owner.prepare_cloud_backup().is_err());
    assert_eq!(storage.backups.len(), 1);
    let bytes = owner
        .profile
        .as_ref()
        .unwrap()
        .cloud_backup
        .as_ref()
        .unwrap()
        .upload
        .as_ref()
        .unwrap()
        .bytes
        .0
        .clone();
    std::fs::remove_dir(temporary).unwrap();
    owner.synchronize(&mut storage).unwrap();
    assert_eq!(storage.backups.len(), 2);
    assert!(storage.backups.values().any(|uploaded| uploaded == &bytes));
}

/// Honest capsule chain, oldest first: one capsule per published change.
fn capsule_chain(dir: &Path, phrase: &RecoveryPhrase, storage: &mut Memory) -> (Hash, Vec<Hash>) {
    let mut owner = create(&dir.join("owner.lbx"), phrase, "Space");
    let mut chain = Vec::new();
    for title in ["First", "Second", "Third"] {
        add(&mut owner, title);
        owner.synchronize(storage).unwrap();
        let backup = owner
            .profile
            .as_ref()
            .unwrap()
            .cloud_backup
            .as_ref()
            .unwrap();
        chain.push(backup.upload.as_ref().unwrap().hash);
    }
    assert_eq!(storage.backups.len(), chain.len());
    (capsule::directory(&phrase.signing_key()), chain)
}

fn head(root: &SigningKey, storage: &mut Memory) -> Result<Hash> {
    let mut spaces = ProfileStore::discover_cloud_spaces(root, storage, &Cancellation::default())?;
    assert_eq!(spaces.len(), 1);
    Ok(spaces.remove(0).hash)
}

#[test]
fn junk_in_the_recovery_directory_is_skipped_and_never_changes_the_head() {
    let dir = tempfile::tempdir().unwrap();
    let phrase = RecoveryPhrase::generate().unwrap();
    let root = phrase.signing_key();
    let mut honest = Memory::default();
    let (directory, chain) = capsule_chain(dir.path(), &phrase, &mut honest);
    let newest = *chain.last().unwrap();
    assert_eq!(head(&root, &mut honest).unwrap(), newest);
    let junk = |seed: usize| Hash::digest(b"junk\0", &seed.to_le_bytes());
    let max = crate::sync::protocol::MAX_OBJECT_BYTES;

    // Valid-size junk whose total exceeds the cache budget.
    let mut flood = honest.clone();
    for seed in 0..17 {
        flood
            .backups
            .insert((directory, junk(seed)), vec![b'x'; max]);
    }
    // Junk keys alone exceed the authenticated object budget.
    let mut keys = honest.clone();
    for seed in 0..=crate::sync::protocol::MAX_OBJECTS {
        keys.backups.insert((directory, junk(seed)), Vec::new());
    }
    // An object no honest capsule can be, and listed keys whose GET fails.
    let mut oversized = honest.clone();
    oversized
        .backups
        .insert((directory, junk(0)), vec![b'x'; max + 1]);
    let mut failing = honest.clone();
    for (seed, error) in [(0, TransportError::NotFound), (1, TransportError::Budget)] {
        failing.backups.insert((directory, junk(seed)), Vec::new());
        failing.recovery_errors.insert(junk(seed), error);
    }
    for mut storage in [flood, keys, oversized, failing] {
        assert_eq!(head(&root, &mut storage).unwrap(), newest);
    }
}

#[test]
fn missing_or_junk_predecessors_are_skipped_without_selecting_an_older_head() {
    let dir = tempfile::tempdir().unwrap();
    let phrase = RecoveryPhrase::generate().unwrap();
    let root = phrase.signing_key();
    let mut honest = Memory::default();
    let (directory, chain) = capsule_chain(dir.path(), &phrase, &mut honest);
    let [oldest, middle, newest] = chain[..] else {
        unreachable!()
    };

    // The dangling link ends the chain; the newer capsules still verify.
    let mut gone = honest.clone();
    gone.backups.remove(&(directory, oldest));
    assert_eq!(head(&root, &mut gone).unwrap(), newest);
    let mut replaced = honest.clone();
    replaced
        .backups
        .insert((directory, oldest), b"junk".to_vec());
    assert_eq!(head(&root, &mut replaced).unwrap(), newest);
    let mut vanished = honest.clone();
    vanished
        .recovery_errors
        .insert(oldest, TransportError::NotFound);
    assert_eq!(head(&root, &mut vanished).unwrap(), newest);

    // A gap leaves two unordered capsules: no automatic winner, never the older.
    let mut gap = honest.clone();
    gap.backups.remove(&(directory, middle));
    assert!(matches!(
        head(&root, &mut gap),
        Err(VaultError::CloudRecoveryConflict)
    ));
    // A provider failure is not junk: it must not hide the newest capsule.
    for error in [
        TransportError::Unavailable,
        TransportError::AccessDenied,
        TransportError::InvalidResponse,
    ] {
        let mut flaky = honest.clone();
        flaky.recovery_errors.insert(newest, error);
        assert!(head(&root, &mut flaky).is_err());
    }
}
