use crate::app::test_support::{add, reopen, unlocked};

#[test]
fn new_and_edited_entries_are_written_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    let id = add(&mut unlocked, "GitLab", "a8ee", "");
    assert_eq!(unlocked.selected.as_deref(), Some(id.as_str()));
    assert!(unlocked.draft.is_none());

    unlocked.start_edit();
    unlocked.draft.as_mut().unwrap().entry.password = "s3cret".into();
    unlocked.save_draft().unwrap();

    let saved = reopen(&unlocked);
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.entries[0].password, "s3cret");
}

#[test]
fn entry_without_title_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    unlocked.start_new();
    assert!(unlocked.save_draft().is_err());
    assert!(unlocked.draft.is_some());
    assert!(reopen(&unlocked).entries.is_empty());
}

#[test]
fn failed_save_keeps_the_draft_and_the_vault_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    let selected = add(&mut unlocked, "GitLab", "a8ee", "");
    add(&mut unlocked, "Second", "", "");
    unlocked.select(selected.clone());
    let previous = unlocked.vault.entries.clone();
    // Block atomic replacement without removing the saved vault.
    std::fs::create_dir(unlocked.vault.path().with_extension("lbx.tmp")).unwrap();
    unlocked.start_new();
    unlocked.draft.as_mut().unwrap().entry.title = "Почта".into();
    assert!(unlocked.save_draft().is_err());
    assert_eq!(unlocked.draft.as_ref().unwrap().entry.title, "Почта");
    assert_eq!(unlocked.selected.as_deref(), Some(selected.as_str()));
    assert_eq!(unlocked.vault.entries, previous);
    assert_eq!(reopen(&unlocked).entries, previous);
}

#[test]
fn failed_update_preserves_order_selection_and_attachment_draft() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "First", "", "");
    let selected = add(&mut unlocked, "Middle", "", "");
    add(&mut unlocked, "Last", "", "");
    unlocked.select(selected.clone());
    let previous = unlocked.vault.entries.clone();
    unlocked.start_edit();
    let entry = &mut unlocked.draft.as_mut().unwrap().entry;
    entry.password = "changed".into();
    entry
        .add_attachment("draft.bin".into(), vec![0, 255])
        .unwrap();
    let tmp = unlocked.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();

    assert!(unlocked.save_draft().is_err());
    assert_eq!(unlocked.selected.as_deref(), Some(selected.as_str()));
    assert_eq!(unlocked.vault.entries, previous);
    assert_eq!(reopen(&unlocked).entries, previous);
    let draft = &unlocked.draft.as_ref().unwrap().entry;
    assert_eq!(draft.password, "changed");
    assert_eq!(draft.attachments[0].data, [0, 255]);

    std::fs::remove_dir(tmp).unwrap();
    unlocked.save_draft().unwrap();
    let saved = reopen(&unlocked);
    assert_eq!(saved.entries[1].password, "changed");
    assert_eq!(saved.entries[1].attachments[0].data, [0, 255]);
}

#[test]
fn failed_delete_restores_original_position_and_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "First", "", "");
    let selected = add(&mut unlocked, "Middle", "", "");
    add(&mut unlocked, "Last", "", "");
    unlocked.select(selected.clone());
    unlocked.confirm_delete = true;
    let previous = unlocked.vault.entries.clone();
    let tmp = unlocked.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();

    assert!(unlocked.delete_selected().is_err());
    assert_eq!(unlocked.selected.as_deref(), Some(selected.as_str()));
    assert!(unlocked.confirm_delete);
    assert_eq!(unlocked.vault.entries, previous);
    assert_eq!(reopen(&unlocked).entries, previous);

    std::fs::remove_dir(tmp).unwrap();
    unlocked.delete_selected().unwrap();
    assert_eq!(reopen(&unlocked).entries.len(), 2);
}

#[test]
fn shared_failed_save_retains_view_and_retries_reserved_intent() {
    use crate::{
        app::workspace::Unlocked,
        infra::{
            vault::{KdfParams, ProfileStore, ProviderConfiguration, SharedProfile},
            witness::MemoryWitness,
        },
    };
    use secrecy::SecretString;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let profiles = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let vault = profiles
        .create_shared(
            dir.path().join("shared.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
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
    let mut unlocked = Unlocked::new(vault);
    let selected = add(&mut unlocked, "Existing", "", "");
    let previous = unlocked.vault.entries.clone();
    unlocked.start_new();
    unlocked.draft.as_mut().unwrap().entry.title = "Retry".into();
    let tmp = unlocked.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();

    assert!(unlocked.save_draft().is_err());
    assert_eq!(unlocked.vault.entries, previous);
    assert_eq!(unlocked.selected.as_deref(), Some(selected.as_str()));
    assert_eq!(unlocked.draft.as_ref().unwrap().entry.title, "Retry");
    assert!(unlocked.vault.sync_state().unwrap().intent.is_some());
    let objects = unlocked.vault.sync_state().unwrap().objects.len();

    std::fs::remove_dir(tmp).unwrap();
    unlocked.save_draft().unwrap();
    assert_eq!(unlocked.vault.entries.len(), 2);
    assert!(unlocked.vault.sync_state().unwrap().intent.is_none());
    assert_eq!(
        unlocked.vault.sync_state().unwrap().objects.len(),
        objects + 1
    );
}

#[test]
fn deleted_entry_is_removed_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "GitLab", "a8ee", "");
    add(&mut unlocked, "Почта", "inbox", "");
    unlocked.delete_selected().unwrap();
    assert!(unlocked.selected.is_none());
    let saved = reopen(&unlocked);
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.entries[0].title, "GitLab");
}

#[test]
fn attachment_drafts_cancel_save_and_rollback_with_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "Files", "", "");
    unlocked.start_edit();
    unlocked
        .draft
        .as_mut()
        .unwrap()
        .entry
        .add_attachment("cancel.bin".into(), vec![0, 255])
        .unwrap();
    unlocked.draft = None;
    assert!(reopen(&unlocked).entries[0].attachments.is_empty());
    unlocked.start_edit();
    unlocked
        .draft
        .as_mut()
        .unwrap()
        .entry
        .add_attachment("saved.bin".into(), vec![0, 255])
        .unwrap();
    unlocked.save_draft().unwrap();
    assert_eq!(reopen(&unlocked).entries[0].attachments[0].data, [0, 255]);
    unlocked.start_edit();
    unlocked.draft.as_mut().unwrap().entry.attachments.clear();
    std::fs::remove_dir_all(dir.path()).unwrap();
    assert!(unlocked.save_draft().is_err());
    assert!(
        unlocked
            .draft
            .as_ref()
            .unwrap()
            .entry
            .attachments
            .is_empty()
    );
    assert_eq!(unlocked.vault.entries[0].attachments[0].data, [0, 255]);
}
