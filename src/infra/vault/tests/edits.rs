use super::*;
use crate::core::sync::Hash;
use crate::infra::witness::MemoryWitness;

#[test]
fn participant_aliases_persist_without_replicated_events_and_clear_to_own_name() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("profile.lbx");
    let mut vault = sharing_tests::create(&store, &path);
    let user = vault.device_certificate().unwrap().body.user;
    let objects = vault.sync_state().unwrap().objects.clone();
    vault.set_participant_name(user, "  Local alias  ").unwrap();
    assert_eq!(vault.participant_name(user), Some("Local alias"));
    assert_eq!(vault.sync_state().unwrap().objects, objects);
    drop(vault);
    let mut reopened = store
        .open(&path, &SecretString::from("test master".to_owned()))
        .unwrap();
    assert_eq!(reopened.participant_alias(user), Some("Local alias"));
    reopened.set_participant_name(user, " ").unwrap();
    assert_eq!(reopened.participant_alias(user), None);
    assert_eq!(reopened.participant_name(user), Some("Test user"));
    assert_eq!(reopened.sync_state().unwrap().objects, objects);
    drop(reopened);
    let reopened = store
        .open(&path, &SecretString::from("test master".to_owned()))
        .unwrap();
    assert_eq!(reopened.participant_alias(user), None);
}

#[test]
fn failed_alias_insert_replace_and_remove_restore_memory_and_disk() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("profile.lbx");
    let mut vault = sharing_tests::create(&store, &path);
    let user = vault.device_certificate().unwrap().body.user;
    vault.set_participant_name(user, "Previous").unwrap();
    let before = fs::read(&path).unwrap();
    let temporary = path.with_extension("lbx.tmp");
    fs::create_dir(&temporary).unwrap();
    for (user, name) in [(Hash([42; 32]), "New"), (user, "Replacement"), (user, "")] {
        assert!(vault.set_participant_name(user, name).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    assert_eq!(vault.participant_alias(Hash([42; 32])), None);
    assert_eq!(vault.participant_alias(user), Some("Previous"));
    fs::remove_dir(temporary).unwrap();
    vault.set_participant_name(user, "Retry").unwrap();
    assert_eq!(vault.participant_alias(user), Some("Retry"));
}

#[test]
fn invalid_aliases_are_rejected_before_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("profile.lbx");
    let mut vault = sharing_tests::create(&store, &path);
    let before = fs::read(&path).unwrap();
    for name in ["x".repeat(257), "Ж".repeat(129), "bad\0name".into()] {
        assert!(matches!(
            vault.set_participant_name(Hash([42; 32]), &name),
            Err(VaultError::InvalidParticipantName)
        ));
    }
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn shared_folder_commands_restore_readable_view_and_retry_retained_intent() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("profile.lbx");
    let mut vault = sharing_tests::create(&store, &path);
    let parent = Folder::new("Parent".into(), None);
    let child = Folder::new("Child".into(), Some(parent.id.clone()));
    vault.save_folder(parent.clone()).unwrap();
    vault.save_folder(child.clone()).unwrap();
    let mut entry = Entry::new();
    entry.title = "Record".into();
    entry.folder_id = Some(child.id.clone());
    vault.upsert_entry(entry).unwrap();
    let entries = vault.entries().to_vec();
    let folders = vault.folders().to_vec();
    let temporary = path.with_extension("lbx.tmp");
    fs::create_dir(&temporary).unwrap();
    assert!(vault.delete_folder(&child.id).is_err());
    assert_eq!(vault.entries(), entries);
    assert_eq!(vault.folders(), folders);
    assert!(vault.sync_state().unwrap().intent.is_some());
    fs::remove_dir(temporary).unwrap();
    vault.delete_folder(&child.id).unwrap();
    assert!(vault.sync_state().unwrap().intent.is_none());
    assert_eq!(vault.folders(), std::slice::from_ref(&parent));
    assert_eq!(
        vault.entries()[0].folder_id.as_deref(),
        Some(parent.id.as_str())
    );
    drop(vault);
    let reopened = store
        .open(&path, &SecretString::from("test master".to_owned()))
        .unwrap();
    assert_eq!(reopened.folders(), &[parent]);
    assert_eq!(
        reopened.entries()[0].folder_id,
        reopened.folders()[0].id.clone().into()
    );
}
