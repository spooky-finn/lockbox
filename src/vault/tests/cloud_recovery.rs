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
#[derive(Default)]
struct Memory {
    objects: BTreeMap<(Hash, Hash), Vec<u8>>,
    backups: BTreeMap<(Hash, Hash), Vec<u8>>,
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
    fn list_recovery(&mut self, directory: Hash, _: Option<&str>) -> TransportResult<Page> {
        Ok(Page {
            objects: self
                .backups
                .keys()
                .filter(|(d, _)| *d == directory)
                .map(|(_, h)| *h)
                .rev()
                .collect(),
            next: None,
        })
    }
    fn get_recovery(&mut self, directory: Hash, hash: Hash) -> TransportResult<Vec<u8>> {
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
