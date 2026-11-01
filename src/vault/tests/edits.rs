use crate::{
    sync::protocol::Hash,
    vault::{witness::MemoryWitness, *},
};

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

#[test]
fn local_import_batches_restore_existing_records_and_folders_after_failed_writes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local.lbx");
    let password = SecretString::from("test master".to_owned());
    let mut vault = Vault::create(
        &path,
        &password,
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
    )
    .unwrap();
    let parent = Folder::new("Existing".into(), None);
    let mut existing = Entry::new();
    existing.title = "Existing record".into();
    existing.folder_id = Some(parent.id.clone());
    existing
        .add_attachment("keep.bin".into(), vec![1, 2, 3])
        .unwrap();
    vault
        .append_records(
            std::slice::from_ref(&parent),
            std::slice::from_ref(&existing),
        )
        .unwrap();
    let before = fs::read(&path).unwrap();
    let history = vault.record_history().unwrap().len();
    let child = Folder::new("Imported".into(), Some(parent.id.clone()));
    let mut imported = Entry::new();
    imported.title = "Imported record".into();
    imported.folder_id = Some(parent.id.clone());
    let folders = std::slice::from_ref(&child);
    let entries = std::slice::from_ref(&imported);
    let temporary = path.with_extension("lbx.tmp");
    fs::create_dir(&temporary).unwrap();

    for (folders, entries) in [(folders, &[][..]), (&[][..], entries), (folders, entries)] {
        assert!(vault.append_records(folders, entries).is_err());
        assert_eq!(vault.entries(), std::slice::from_ref(&existing));
        assert_eq!(vault.folders(), std::slice::from_ref(&parent));
        assert_eq!(vault.record_history().unwrap().len(), history);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    fs::remove_dir(temporary).unwrap();
    vault.append_records(folders, entries).unwrap();
    let entries = [existing, imported];
    let folders = [parent, child];
    assert_eq!(vault.entries(), entries);
    assert_eq!(vault.folders(), folders);
    drop(vault);
    let reopened = Vault::open(&path, &password).unwrap();
    assert_eq!(reopened.entries(), entries);
    assert_eq!(reopened.folders(), folders);
}

#[test]
fn shared_import_batch_retry_preserves_existing_records_and_reserved_intent() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("profile.lbx");
    let mut vault = sharing_tests::create(&store, &path);
    let mut existing = Entry::new();
    existing.title = "Existing record".into();
    vault.upsert_entry(existing.clone()).unwrap();
    let objects = vault.sync_state().unwrap().objects.len();
    let folder = Folder::new("Imported".into(), None);
    let mut imported = Entry::new();
    imported.title = "Imported record".into();
    imported.folder_id = Some(folder.id.clone());
    let folders = std::slice::from_ref(&folder);
    let entries = std::slice::from_ref(&imported);
    let before = fs::read(&path).unwrap();
    let temporary = path.with_extension("lbx.tmp");
    fs::create_dir(&temporary).unwrap();

    assert!(vault.append_records(folders, entries).is_err());
    assert_eq!(vault.entries(), std::slice::from_ref(&existing));
    assert!(vault.folders().is_empty());
    assert!(vault.sync_state().unwrap().intent.is_some());
    assert_eq!(fs::read(&path).unwrap(), before);

    fs::remove_dir(temporary).unwrap();
    vault.append_records(folders, entries).unwrap();
    assert!(vault.sync_state().unwrap().intent.is_none());
    assert_eq!(vault.sync_state().unwrap().objects.len(), objects + 1);
    drop(vault);
    let reopened = store
        .open(&path, &SecretString::from("test master".to_owned()))
        .unwrap();
    assert_eq!(reopened.entries().len(), 2);
    for entry in [existing, imported] {
        assert!(reopened.entries().contains(&entry));
    }
    assert_eq!(reopened.folders(), [folder]);
}

#[test]
fn local_folder_upserts_restore_position_after_validation_and_write_failures() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-folders.lbx");
    let password = SecretString::from("master".to_owned());
    let mut vault = Vault::create(
        &path,
        &password,
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
    )
    .unwrap();
    let first = Folder::new("First".into(), None);
    let second = Folder::new("Second".into(), None);
    let original = [first.clone(), second.clone()];
    let mut entry = Entry::new();
    entry.title = "Keep this record".into();
    entry.password = "fictional secret".into();
    entry.folder_id = Some(first.id.clone());
    entry
        .add_attachment("keep.bin".into(), vec![0, 255, 42])
        .unwrap();
    vault
        .append_records(&original, std::slice::from_ref(&entry))
        .unwrap();
    let before = fs::read(&path).unwrap();
    let history = vault.record_history().unwrap().len();

    let mut duplicate = first.clone();
    duplicate.name = " Second ".into();
    let mut cycle = second.clone();
    cycle.parent_id = Some(second.id.clone());
    for invalid in [
        duplicate,
        Folder::new("Missing parent".into(), Some("missing".into())),
        cycle,
    ] {
        assert!(matches!(
            vault.save_folder(invalid),
            Err(VaultError::InvalidFolders(_))
        ));
        assert_eq!(vault.folders(), original);
        assert_eq!(vault.entries(), std::slice::from_ref(&entry));
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let mut renamed = first;
    renamed.name = "Renamed".into();
    let inserted = Folder::new("Inserted".into(), Some(renamed.id.clone()));
    let temporary = path.with_extension("lbx.tmp");
    fs::create_dir(&temporary).unwrap();
    for folder in [&renamed, &inserted] {
        assert!(vault.save_folder(folder.clone()).is_err());
        assert_eq!(vault.folders(), original);
        assert_eq!(vault.entries(), std::slice::from_ref(&entry));
        assert_eq!(vault.record_history().unwrap().len(), history);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    fs::remove_dir(temporary).unwrap();
    vault.save_folder(renamed.clone()).unwrap();
    vault.save_folder(inserted.clone()).unwrap();
    let expected = [renamed, second, inserted];
    assert_eq!(vault.folders(), expected);
    drop(vault);
    let reopened = Vault::open(&path, &password).unwrap();
    assert_eq!(reopened.folders(), expected);
    assert_eq!(reopened.entries(), &[entry]);
}

#[test]
fn shared_folder_upserts_retry_retained_intents_after_restoring_readable_view() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("shared-folders.lbx");
    let mut vault = sharing_tests::create(&store, &path);
    let parent = Folder::new("Parent".into(), None);
    vault.save_folder(parent.clone()).unwrap();
    let mut entry = Entry::new();
    entry.title = "Preserved record".into();
    entry.folder_id = Some(parent.id.clone());
    vault.upsert_entry(entry.clone()).unwrap();
    let mut renamed = parent;
    renamed.name = "Renamed parent".into();
    let child = Folder::new("Child".into(), Some(renamed.id.clone()));
    let temporary = path.with_extension("lbx.tmp");
    for folder in [&renamed, &child] {
        let folders = vault.folders().to_vec();
        let objects = vault.sync_state().unwrap().objects.len();
        fs::create_dir(&temporary).unwrap();
        assert!(vault.save_folder(folder.clone()).is_err());
        assert_eq!(vault.folders(), folders);
        assert_eq!(vault.entries(), std::slice::from_ref(&entry));
        assert!(vault.sync_state().unwrap().intent.is_some());
        fs::remove_dir(&temporary).unwrap();
        vault.save_folder(folder.clone()).unwrap();
        assert!(vault.sync_state().unwrap().intent.is_none());
        assert_eq!(vault.sync_state().unwrap().objects.len(), objects + 1);
        assert!(vault.folders().contains(folder));
    }
    drop(vault);
    let reopened = store
        .open(&path, &SecretString::from("test master".to_owned()))
        .unwrap();
    assert_eq!(reopened.folders().len(), 2);
    assert!(reopened.folders().contains(&renamed));
    assert!(reopened.folders().contains(&child));
    assert_eq!(reopened.entries(), &[entry]);
}
