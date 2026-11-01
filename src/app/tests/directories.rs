use super::*;
use crate::app::{
    test_support::{reopen, unlocked},
    tree::{self, Node},
};

fn add_folder(workspace: &mut Unlocked, name: &str, parent: Option<String>) -> String {
    workspace.select_directory(parent);
    workspace.start_directory();
    workspace.directories.draft.as_mut().unwrap().name = name.into();
    workspace.directories.save(&mut workspace.vault).unwrap();
    workspace.directories.selected.clone().unwrap()
}

fn add_record(workspace: &mut Unlocked, title: &str) -> String {
    workspace.start_new();
    let draft = &mut workspace.draft.as_mut().unwrap().entry;
    draft.title = title.into();
    draft.password = "kept-secret".into();
    let id = draft.id.clone();
    workspace.save_draft().unwrap();
    id
}

#[test]
fn nested_folders_and_record_locations_survive_password_and_key_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let parent = add_folder(&mut workspace, "Работа", None);
    let child = add_folder(&mut workspace, "Инфраструктура", Some(parent.clone()));
    add_record(&mut workspace, "GitLab");
    let saved = reopen(&workspace);
    assert_eq!(saved.folders.len(), 2);
    assert_eq!(saved.folders[1].parent_id.as_deref(), Some(parent.as_str()));
    assert_eq!(saved.entries[0].folder_id.as_deref(), Some(child.as_str()));
    let key = workspace.vault.export_key();
    let path = workspace.vault.path().to_path_buf();
    // The session holds the file lock; a second open succeeds once it closes.
    drop(workspace);
    let biometric = Vault::open_with_key(path, &key).unwrap();
    assert_eq!(biometric.folders, saved.folders);
    assert_eq!(biometric.entries, saved.entries);
}

#[test]
fn renaming_reparenting_and_moving_records_keep_stable_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let parent = add_folder(&mut workspace, "Работа", None);
    let child = add_folder(&mut workspace, "Сервисы", Some(parent));
    let record = add_record(&mut workspace, "GitLab");
    workspace.select_directory(Some(child.clone()));
    workspace.edit_directory();
    let draft = workspace.directories.draft.as_mut().unwrap();
    draft.name = "Личное".into();
    draft.parent_id = None;
    workspace.directories.save(&mut workspace.vault).unwrap();
    assert_eq!(
        workspace.directories.selected.as_deref(),
        Some(child.as_str())
    );
    workspace.select(record.clone());
    workspace.start_edit();
    workspace.draft.as_mut().unwrap().entry.folder_id = None;
    workspace.save_draft().unwrap();
    let saved = reopen(&workspace);
    assert_eq!(saved.entries[0].id, record);
    assert!(saved.entries[0].folder_id.is_none());
    assert_eq!(saved.folders[1].id, child);
    assert_eq!(saved.folders[1].name, "Личное");
    assert!(saved.folders[1].parent_id.is_none());
}

#[test]
fn deleting_a_subtree_moves_every_record_to_the_parent_without_losing_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let parent = add_folder(&mut workspace, "Работа", None);
    let child = add_folder(&mut workspace, "Проекты", Some(parent.clone()));
    add_record(&mut workspace, "GitLab");
    workspace.select_directory(Some(child.clone()));
    add_folder(&mut workspace, "Архив", Some(child.clone()));
    add_record(&mut workspace, "Mail");
    workspace.select_directory(Some(child));
    workspace.directories.delete(&mut workspace.vault).unwrap();
    let saved = reopen(&workspace);
    assert_eq!(saved.folders.len(), 1);
    assert_eq!(saved.entries.len(), 2);
    for entry in &saved.entries {
        assert_eq!(entry.folder_id.as_deref(), Some(parent.as_str()));
        assert_eq!(entry.password, "kept-secret");
    }
    workspace.directories.delete(&mut workspace.vault).unwrap();
    let root = reopen(&workspace);
    assert!(root.folders.is_empty());
    assert!(root.entries.iter().all(|entry| entry.folder_id.is_none()));
}

#[test]
fn failed_folder_save_keeps_the_draft_and_original_hierarchy() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let parent = add_folder(&mut workspace, "Работа", None);
    let previous = workspace.vault.folders.clone();
    workspace.edit_directory();
    workspace.directories.draft.as_mut().unwrap().name = "Unsaved".into();
    std::fs::remove_dir_all(dir.path()).unwrap();
    assert!(workspace.directories.save(&mut workspace.vault).is_err());
    assert_eq!(workspace.vault.folders, previous);
    assert_eq!(
        workspace.directories.draft.as_ref().unwrap().name,
        "Unsaved"
    );
    assert_eq!(
        workspace.directories.selected.as_deref(),
        Some(parent.as_str())
    );
}

#[test]
fn failed_folder_deletion_restores_hierarchy_and_record_assignments() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let parent = add_folder(&mut workspace, "Работа", None);
    let child = add_folder(&mut workspace, "Сервисы", Some(parent.clone()));
    add_record(&mut workspace, "GitLab");
    workspace.select_directory(Some(parent.clone()));
    let previous = workspace.vault.folders.clone();
    std::fs::remove_dir_all(dir.path()).unwrap();
    assert!(workspace.directories.delete(&mut workspace.vault).is_err());
    assert_eq!(workspace.vault.folders, previous);
    assert_eq!(
        workspace.vault.entries[0].folder_id.as_deref(),
        Some(child.as_str())
    );
    assert_eq!(
        workspace.directories.selected.as_deref(),
        Some(parent.as_str())
    );
}

#[test]
fn cycles_and_duplicate_siblings_are_rejected_before_changing_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let parent = add_folder(&mut workspace, "Работа", None);
    let child = add_folder(&mut workspace, "Сервисы", Some(parent.clone()));
    workspace.select_directory(Some(parent));
    workspace.edit_directory();
    workspace.directories.draft.as_mut().unwrap().parent_id = Some(child);
    assert!(workspace.directories.save(&mut workspace.vault).is_err());
    assert!(reopen(&workspace).folders[0].parent_id.is_none());
    workspace.directories.draft = None;
    workspace.select_directory(None);
    workspace.start_directory();
    workspace.directories.draft.as_mut().unwrap().name = " работа ".into();
    assert!(workspace.directories.save(&mut workspace.vault).is_err());
    assert_eq!(reopen(&workspace).folders.len(), 2);
}

#[test]
fn search_keeps_ancestors_and_matches_folder_names_and_hidden_record_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let parent = add_folder(&mut workspace, "Работа", None);
    add_folder(&mut workspace, "Сервисы", Some(parent));
    add_record(&mut workspace, "GitLab");
    workspace.start_edit();
    workspace.draft.as_mut().unwrap().entry.username = "hidden-alice".into();
    workspace.save_draft().unwrap();
    for query in ["hidden-ALICE", "РАБОТА", "сервисы"] {
        let nodes = tree::project(&workspace.vault, query);
        let Node::Folder(parent, children) = &nodes[0] else {
            panic!("ancestor missing")
        };
        assert_eq!(parent.name, "Работа");
        let Node::Folder(child, children) = &children[0] else {
            panic!("child missing")
        };
        assert_eq!(child.name, "Сервисы");
        assert!(matches!(&children[0], Node::Record(entry) if entry.title == "GitLab"));
    }
    assert!(tree::project(&workspace.vault, "missing").is_empty());
}
