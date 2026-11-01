use crate::{
    desktop::test_support::{add, reopen, unlocked},
    records::{entry::Entry, import::*},
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const FAST: KdfParams = KdfParams {
    memory_kib: 8 * 1024,
    passes: 1,
    lanes: 1,
};

/// A separate local vault with one folder, two entries and an attachment.
fn source(dir: &tempfile::TempDir) -> PathBuf {
    let path = dir.path().join("old.lbx");
    let mut vault = Vault::create(&path, &SecretString::from("old-pass".to_owned()), FAST).unwrap();
    let folder = Folder::new("Work".into(), None);
    let mut ssh = Entry::new();
    ssh.title = "ssh".into();
    ssh.password = "ssh-secret".into();
    ssh.folder_id = Some(folder.id.clone());
    ssh.add_attachment("id_ed25519.pub".into(), b"ssh-ed25519 AAAA".to_vec())
        .unwrap();
    let mut mail = Entry::new();
    mail.title = "Почта".into();
    mail.password = "mail-secret".into();
    vault.test_folders_mut().push(folder);
    vault.test_entries_mut().extend([ssh, mail]);
    vault.save().unwrap();
    path
}

fn shared(dir: &tempfile::TempDir) -> Unlocked {
    let vault = ProfileStore::new(Arc::new(MemoryWitness::default()))
        .create_shared(
            dir.path().join("shared.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            FAST,
            SharedProfile {
                username: "Alice".into(),
                space_name: "Test".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "test-bucket".into(),
                    access_key: "access".into(),
                    secret_key: "secret".into(),
                },
            },
        )
        .unwrap();
    Unlocked::new(vault)
}

fn run_import(unlocked: &mut Unlocked, path: &std::path::Path, password: &str) {
    unlocked.choose_import(path.to_path_buf());
    *unlocked.import.password = password.into();
    unlocked.start_import();
    assert!(unlocked.import.password.is_empty());
    let deadline = Instant::now() + Duration::from_secs(10);

    while unlocked.import.busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        unlocked.poll_import();
    }
    assert!(!unlocked.import.busy());
}

#[test]
fn local_import_copies_entries_folders_and_files_and_leaves_the_source_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = source(&dir);
    let before = std::fs::read(&path).unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "Existing", "", "");

    run_import(&mut unlocked, &path, "old-pass");

    assert_eq!(unlocked.error, None);
    assert_eq!(
        unlocked.import.notice.as_deref(),
        Some("Imported 2 entries and 1 folders; 0 duplicates skipped")
    );
    let saved = reopen(&unlocked);
    assert_eq!(saved.entries.len(), 3);
    assert_eq!(saved.folders.len(), 1);
    let ssh = saved.entries.iter().find(|e| e.title == "ssh").unwrap();
    assert_eq!(ssh.password, "ssh-secret");
    assert_eq!(ssh.folder_id.as_ref(), Some(&saved.folders[0].id));
    assert_eq!(ssh.attachments[0].data, b"ssh-ed25519 AAAA");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn repeating_an_import_adds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = source(&dir);
    let mut unlocked = unlocked(&dir);

    run_import(&mut unlocked, &path, "old-pass");
    run_import(&mut unlocked, &path, "old-pass");

    assert_eq!(
        unlocked.import.notice.as_deref(),
        Some("Nothing new to import; 2 duplicates skipped")
    );
    assert_eq!(reopen(&unlocked).entries.len(), 2);
}

#[test]
fn wrong_password_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = source(&dir);
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "Existing", "", "");

    run_import(&mut unlocked, &path, "wrong");

    assert_eq!(
        unlocked.error.as_deref(),
        Some("Could not import: incorrect master password")
    );
    assert_eq!(unlocked.import.notice, None);
    assert_eq!(reopen(&unlocked).entries.len(), 1);
}

#[test]
fn the_open_vault_cannot_import_itself() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    let own = unlocked.vault.path().to_path_buf();
    unlocked.choose_import(own);
    *unlocked.import.password = "master".into();
    unlocked.start_import();
    assert_eq!(
        unlocked.error.as_deref(),
        Some("This vault is already open")
    );
    assert!(!unlocked.import.busy());
}

#[test]
fn result_waits_while_an_entry_is_being_edited() {
    let dir = tempfile::tempdir().unwrap();
    let path = source(&dir);
    let mut unlocked = unlocked(&dir);
    unlocked.choose_import(path);
    *unlocked.import.password = "old-pass".into();
    unlocked.start_import();
    unlocked.start_new();
    let deadline = Instant::now() + Duration::from_secs(10);

    while !unlocked.import.needs_poll() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    unlocked.poll_import();
    assert!(unlocked.import.busy());
    assert!(unlocked.vault.entries().is_empty());

    unlocked.draft = None;
    unlocked.poll_import();
    assert!(!unlocked.import.busy());
    assert_eq!(unlocked.vault.entries().len(), 2);
}

#[test]
fn shared_import_is_journaled_as_signed_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = source(&dir);
    let mut unlocked = shared(&dir);
    let objects = unlocked.vault.sync_state().unwrap().objects.len();

    run_import(&mut unlocked, &path, "old-pass");

    assert_eq!(unlocked.error, None);
    assert_eq!(unlocked.vault.entries().len(), 2);
    assert_eq!(unlocked.vault.folders().len(), 1);
    let state = unlocked.vault.sync_state().unwrap();
    assert!(state.objects.len() > objects);
    assert!(state.intent.is_none());
}

#[test]
fn failed_batch_is_rolled_back_and_retry_completes() {
    let dir = tempfile::tempdir().unwrap();
    let path = source(&dir);
    let records = Vault::read_records(&path, &SecretString::from("old-pass".to_owned())).unwrap();
    let mut unlocked = unlocked(&dir);
    let tmp = unlocked.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();

    let error = unlocked.import_records(&records).unwrap_err();
    assert!(error.starts_with("Imported 0 of 2 entries"), "{error}");
    assert!(unlocked.vault.entries().is_empty());
    assert!(unlocked.vault.folders().is_empty());

    std::fs::remove_dir(tmp).unwrap();
    let summary = unlocked.import_records(&records).unwrap();
    assert_eq!(summary.entries, 2);
    assert_eq!(reopen(&unlocked).entries.len(), 2);
}

#[test]
fn large_imports_are_split_into_bounded_batches() {
    let mut entries = Vec::new();

    for index in 0..BATCH_MUTATIONS + 3 {
        let mut entry = Entry::new();
        entry.title = format!("entry {index}");
        entries.push(entry);
    }

    let mut big = Entry::new();
    big.title = "big".into();
    big.add_attachment("blob".into(), vec![7; 900 * 1024])
        .unwrap();
    entries.insert(1, big.clone());
    entries.insert(2, big);

    let batches = entry_batches(&entries);
    assert_eq!(
        batches.iter().map(|b| b.len()).sum::<usize>(),
        entries.len()
    );
    assert!(batches.iter().all(|b| b.len() <= BATCH_MUTATIONS));
    assert!(batches.iter().all(|b| {
        b.len() == 1
            || b.iter()
                .map(|e| serde_json::to_vec(e).unwrap().len())
                .sum::<usize>()
                <= BATCH_BYTES
    }));
    assert!(batches.len() >= 3);
}

#[test]
fn batch_byte_boundaries_match_json_escaping_unicode_and_attachments() {
    let mut first = Entry::new();
    first.title = "Quoted \"title\" and backslash \\".into();
    first
        .add_attachment("ключ.bin".into(), vec![0, 255, 42])
        .unwrap();
    let mut second = Entry::new();
    second.title = "Unicode 🗝️".into();
    second.username = "名\n\t\"\\".into();
    let mut third = Entry::new();
    third.title = "Third".into();
    let encoded_len = |entry: &Entry| serde_json::to_vec(entry).unwrap().len();
    let padding = BATCH_BYTES - encoded_len(&first) - encoded_len(&second);
    first.notes = "a".repeat(padding / 2);
    second.notes = "b".repeat(padding - padding / 2);
    assert_eq!(encoded_len(&first) + encoded_len(&second), BATCH_BYTES);
    let mut entries = [first, second, third];

    assert_eq!(
        entry_batches(&entries)
            .iter()
            .map(|batch| batch.len())
            .collect::<Vec<_>>(),
        [2, 1]
    );
    entries[1].notes.push('x');
    assert_eq!(
        encoded_len(&entries[0]) + encoded_len(&entries[1]),
        BATCH_BYTES + 1
    );
    assert_eq!(
        entry_batches(&entries)
            .iter()
            .map(|batch| batch.len())
            .collect::<Vec<_>>(),
        [1, 2]
    );
}
