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
    add(&mut unlocked, "GitLab", "a8ee", "");
    // The temporary file cannot be created inside a directory that is gone.
    std::fs::remove_dir_all(dir.path()).unwrap();
    unlocked.start_new();
    unlocked.draft.as_mut().unwrap().entry.title = "Почта".into();
    assert!(unlocked.save_draft().is_err());
    assert!(unlocked.draft.is_some());
    assert_eq!(unlocked.vault.entries.len(), 1);
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
