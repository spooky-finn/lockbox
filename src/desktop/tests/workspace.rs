use crate::desktop::test_support::{add, reopen, unlocked};

#[test]
fn creation_event_requires_a_successful_first_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    workspace.start_new();
    workspace.autosave_draft().unwrap();
    assert_eq!(workspace.record_created, 0);
    workspace.draft.as_mut().unwrap().entry.password = "fictional".into();
    assert!(workspace.autosave_draft().is_err());
    assert_eq!(workspace.record_created, 0);
    workspace.draft.as_mut().unwrap().entry.title = "Example".into();
    let tmp = workspace.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();
    assert!(workspace.autosave_draft().is_err());
    assert_eq!(workspace.record_created, 0);
    std::fs::remove_dir(tmp).unwrap();
    workspace.autosave_draft().unwrap();
    assert_eq!(workspace.record_created, 1);
    workspace.autosave_draft().unwrap();
    workspace.close_draft().unwrap();
    assert_eq!(workspace.record_created, 1);
}

#[test]
fn autosave_persists_changes_without_closing_or_resetting_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    workspace.start_new();
    let draft = workspace.draft.as_mut().unwrap();
    let id = draft.entry.id.clone();
    draft.entry.title = "Automatic".into();
    draft.entry.password = "fictional-password".into();
    draft.show_password = true;
    draft.icon_picker_open = true;
    workspace.autosave_draft().unwrap();
    assert_eq!(workspace.record_created, 1);

    let draft = workspace.draft.as_ref().unwrap();
    assert!(draft.show_password);
    assert!(draft.icon_picker_open);
    assert!(!draft.is_new);
    assert_eq!(reopen(&workspace).entries[0].password, "fictional-password");

    workspace.draft.as_mut().unwrap().entry.username = "fictional-user".into();
    workspace.autosave_draft().unwrap();
    assert_eq!(
        workspace.record_created, 1,
        "edits must not celebrate again"
    );
    let saved = reopen(&workspace);
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.entries[0].id, id);
    assert_eq!(saved.entries[0].username, "fictional-user");
    workspace.close_draft().unwrap();
    assert!(workspace.draft.is_none());
    assert_eq!(workspace.selected.as_deref(), Some(id.as_str()));
}

#[test]
fn autosave_retains_version_snapshots_authorship_and_local_time_without_noop_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    workspace.start_new();
    workspace.draft.as_mut().unwrap().entry.title = "Versioned".into();
    workspace.draft.as_mut().unwrap().entry.password = "first-fictional-password".into();
    workspace.autosave_draft().unwrap();
    workspace.draft.as_mut().unwrap().entry.password = "second-fictional-password".into();
    workspace.autosave_draft().unwrap();

    let history = workspace.vault.record_history().unwrap();
    assert_eq!(history.len(), 2);
    assert!(history[0].before.is_empty());
    assert_eq!(history[1].before[0].password, "first-fictional-password");
    assert_eq!(
        history[1].after.as_ref().unwrap().password,
        "second-fictional-password"
    );
    assert_eq!(history[1].actor, "Local user");
    assert_eq!(history[1].device, "Local vault");
    assert!(history.iter().all(|revision| revision.at.is_some()));
    assert!(history[0].at <= history[1].at);
    assert_ne!(history[0].revision, history[1].revision);

    workspace.draft.as_mut().unwrap().show_password = true;
    workspace.autosave_draft().unwrap();
    workspace.close_draft().unwrap();
    assert_eq!(workspace.vault.record_history().unwrap().len(), 2);
    let path = workspace.vault.path().to_path_buf();
    drop(workspace);
    let reopened =
        crate::vault::Vault::open(&path, &secrecy::SecretString::from("master".to_owned()))
            .unwrap();
    let persisted = reopened.record_history().unwrap();
    assert_eq!(
        serde_json::to_vec(&persisted).unwrap(),
        serde_json::to_vec(&history).unwrap()
    );
}

#[test]
fn closing_an_empty_editor_does_not_create_a_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    workspace.start_new();
    workspace.close_draft().unwrap();
    assert!(workspace.draft.is_none());
    assert!(reopen(&workspace).entries.is_empty());
}

#[test]
fn failed_autosave_preserves_edits_and_close_retries_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    add(&mut workspace, "Original", "", "");
    workspace.start_edit();
    workspace.draft.as_mut().unwrap().entry.password = "updated".into();
    let tmp = workspace.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();

    assert!(workspace.autosave_draft().is_err());
    assert!(workspace.close_draft().is_err());
    assert_eq!(workspace.draft.as_ref().unwrap().entry.password, "updated");
    assert_eq!(reopen(&workspace).entries[0].password, "");

    std::fs::remove_dir(tmp).unwrap();
    workspace.close_draft().unwrap();
    assert_eq!(reopen(&workspace).entries[0].password, "updated");
    assert!(workspace.draft.is_none());
}

#[test]
fn invalid_autosave_keeps_the_last_valid_record_and_the_editor_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    add(&mut workspace, "Original", "", "");
    workspace.start_edit();
    workspace.draft.as_mut().unwrap().entry.title.clear();
    assert!(workspace.autosave_draft().is_err());
    assert!(workspace.close_draft().is_err());
    assert_eq!(reopen(&workspace).entries[0].title, "Original");
    assert!(workspace.draft.is_some());
}

#[test]
fn clearing_an_autosaved_new_record_does_not_discard_the_invalid_edit_on_close() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    workspace.start_new();
    workspace.draft.as_mut().unwrap().entry.title = "Created".into();
    workspace.autosave_draft().unwrap();
    workspace.draft.as_mut().unwrap().entry.title.clear();
    assert!(workspace.close_draft().is_err());
    assert!(workspace.draft.is_some());
    assert_eq!(reopen(&workspace).entries[0].title, "Created");
}

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
    let previous = unlocked.vault.entries().to_vec();
    // Block atomic replacement without removing the saved vault.
    std::fs::create_dir(unlocked.vault.path().with_extension("lbx.tmp")).unwrap();
    unlocked.start_new();
    unlocked.draft.as_mut().unwrap().entry.title = "Почта".into();
    assert!(unlocked.save_draft().is_err());
    assert_eq!(unlocked.draft.as_ref().unwrap().entry.title, "Почта");
    assert_eq!(unlocked.selected.as_deref(), Some(selected.as_str()));
    assert_eq!(unlocked.vault.entries(), previous);
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
    let previous = unlocked.vault.entries().to_vec();
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
    assert_eq!(unlocked.vault.entries(), previous);
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
    let previous = unlocked.vault.entries().to_vec();
    let tmp = unlocked.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();

    assert!(unlocked.delete_selected().is_err());
    assert_eq!(unlocked.selected.as_deref(), Some(selected.as_str()));
    assert!(unlocked.confirm_delete);
    assert_eq!(unlocked.vault.entries(), previous);
    assert_eq!(reopen(&unlocked).entries, previous);

    std::fs::remove_dir(tmp).unwrap();
    unlocked.delete_selected().unwrap();
    assert_eq!(reopen(&unlocked).entries.len(), 2);
}

#[test]
fn shared_failed_save_retains_view_and_retries_reserved_intent() {
    use crate::desktop::workspace::Unlocked;
    use crate::vault::KdfParams;
    use crate::vault::ProfileStore;
    use crate::vault::ProviderConfiguration;
    use crate::vault::SharedProfile;
    use crate::vault::witness::MemoryWitness;
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
    let previous = unlocked.vault.entries().to_vec();
    unlocked.start_new();
    unlocked.draft.as_mut().unwrap().entry.title = "Retry".into();
    let tmp = unlocked.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();

    assert!(unlocked.save_draft().is_err());
    assert_eq!(unlocked.vault.entries(), previous);
    assert_eq!(unlocked.selected.as_deref(), Some(selected.as_str()));
    assert_eq!(unlocked.draft.as_ref().unwrap().entry.title, "Retry");
    assert!(unlocked.vault.sync_state().unwrap().intent.is_some());
    let objects = unlocked.vault.sync_state().unwrap().objects.len();

    std::fs::remove_dir(tmp).unwrap();
    unlocked.save_draft().unwrap();
    assert_eq!(unlocked.vault.entries().len(), 2);
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
    assert_eq!(unlocked.vault.entries()[0].attachments[0].data, [0, 255]);
}

#[test]
fn navigation_prompts_only_for_changed_drafts_and_respects_each_choice() {
    use crate::desktop::workspace::Destination;
    let dir = tempfile::tempdir().unwrap();
    let mut u = unlocked(&dir);
    let first = add(&mut u, "First", "original", "");
    let second = add(&mut u, "Second", "", "");
    u.select(first.clone());
    u.start_edit();
    u.navigate(Destination::Entry(second.clone()));
    assert!(u.draft.is_none());
    assert!(u.pending_navigation.is_none());
    u.select(first.clone());
    u.start_edit();
    u.draft.as_mut().unwrap().entry.username = "changed".into();
    u.navigate(Destination::Directory(None));
    assert!(u.pending_navigation.is_some());
    assert_eq!(u.selected.as_ref(), Some(&first));
    assert_eq!(reopen(&u).entries[0].username, "original");
    u.pending_navigation = None; // Cancel keeps the edit and selection.
    assert_eq!(u.draft.as_ref().unwrap().entry.username, "changed");
    u.navigate(Destination::Entry(second.clone()));
    u.resolve_navigation(false);
    assert_eq!(u.selected.as_ref(), Some(&second));
    assert_eq!(reopen(&u).entries[0].username, "original");
    u.select(first);
    u.start_edit();
    u.draft.as_mut().unwrap().entry.username = "saved".into();
    u.navigate(Destination::Close);
    u.resolve_navigation(true);
    assert!(u.selected.is_none());
    assert!(u.draft.is_none());
    assert_eq!(reopen(&u).entries[0].username, "saved");
}

#[test]
fn failed_navigation_save_retains_the_destination_and_draft() {
    use crate::desktop::workspace::Destination;
    let dir = tempfile::tempdir().unwrap();
    let mut u = unlocked(&dir);
    add(&mut u, "First", "", "");
    u.start_edit();
    u.draft.as_mut().unwrap().entry.title.clear();
    u.navigate(Destination::Close);
    u.resolve_navigation(true);
    assert!(u.pending_navigation.is_some());
    assert!(u.draft.is_some());
    assert!(u.error.is_some());
    u.draft.as_mut().unwrap().entry.title = "Valid".into();
    std::fs::remove_dir_all(dir.path()).unwrap();
    u.resolve_navigation(true);
    assert!(u.pending_navigation.is_some());
    assert!(u.draft.is_some());
    assert!(u.error.is_some());
}

#[test]
fn navigation_flushes_pending_edits_before_leaving_the_editor() {
    use crate::desktop::workspace::Page;
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let other = add(&mut workspace, "Other", "", "");
    for destination in 0..7 {
        workspace.start_new();
        let draft = workspace.draft.as_mut().unwrap();
        let title = format!("Pending {destination}");
        draft.entry.title = title.clone();
        match destination {
            0 => workspace.select(other.clone()),
            1 => workspace.select_directory(None),
            2 => workspace.open_settings(),
            3 => workspace.open_sharing(),
            4 => workspace.open_records_page(Page::Trash),
            5 => workspace.start_new(),
            _ => workspace.start_directory(),
        }
        assert!(
            reopen(&workspace)
                .entries
                .iter()
                .any(|entry| entry.title == title)
        );
        if destination == 5 {
            assert!(workspace.draft.as_ref().unwrap().is_empty());
            workspace.close_draft().unwrap();
        } else {
            assert!(workspace.draft.is_none());
        }
    }
}

#[test]
fn navigation_keeps_the_editor_and_destination_when_saving_fails() {
    use crate::desktop::workspace::Page;
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let other = add(&mut workspace, "Other", "", "");
    workspace.start_new();
    workspace.draft.as_mut().unwrap().entry.title = "Pending".into();
    let tmp = workspace.vault.path().with_extension("lbx.tmp");
    std::fs::create_dir(&tmp).unwrap();
    for destination in 0..7 {
        match destination {
            0 => workspace.select(other.clone()),
            1 => workspace.select_directory(None),
            2 => workspace.open_settings(),
            3 => workspace.open_sharing(),
            4 => workspace.open_records_page(Page::Trash),
            5 => workspace.start_new(),
            _ => workspace.start_directory(),
        }
        assert!(workspace.page == Page::Vault);
        assert!(workspace.error.is_some());
        assert_eq!(workspace.draft.as_ref().unwrap().entry.title, "Pending");
        assert!(workspace.directories.draft.is_none());
    }
    assert_eq!(reopen(&workspace).entries.len(), 1);
    std::fs::remove_dir(tmp).unwrap();
    workspace.open_settings();
    assert!(workspace.page == Page::Settings);
    assert!(workspace.draft.is_none());
    assert_eq!(reopen(&workspace).entries.len(), 2);
}
