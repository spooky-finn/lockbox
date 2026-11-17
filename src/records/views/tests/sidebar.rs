use super::{TreeAction, select_tree_row};
use crate::{
    desktop::{
        context::edit_unlocked,
        session::Screen,
        view_test_support::{mounted, update},
    },
    records::{entry::Entry, folder::Folder},
};
use dioxus::prelude::*;

#[test]
fn keyboard_selection_opens_records_and_preserves_folder_disclosure() {
    let mut dom = mounted();
    let mut folder_id = String::new();
    let mut entry_id = String::new();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let folder = Folder::new("Keyboard folder".into(), None);
            folder_id = folder.id.clone();
            entry_id = u.selected.clone().unwrap();
            u.directories.collapsed.insert(folder_id.clone());
            u.vault.test_folders_mut().push(folder);
        })
    });
    update(&mut dom, |state| {
        let action =
            serde_json::from_value(serde_json::json!({"kind": "directory", "id": folder_id}))
                .unwrap();
        select_tree_row(state, action);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(u.selected.is_none());
        assert_eq!(u.directories.selected.as_deref(), Some(folder_id.as_str()));
        assert!(u.directories.collapsed.contains(&folder_id));
    });
    update(&mut dom, |state| {
        select_tree_row(state, TreeAction::Toggle(folder_id.clone()));
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(!u.directories.collapsed.contains(&folder_id));
    });
    update(&mut dom, |state| {
        select_tree_row(state, TreeAction::Entry(entry_id.clone()));
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert_eq!(u.selected.as_deref(), Some(entry_id.as_str()));
    });
}

#[test]
fn keyboard_navigation_preserves_changed_drafts_and_pending_destination() {
    let mut dom = mounted();
    let mut other_id = String::new();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let mut other = Entry::new();
            other.title = "Other keyboard record".into();
            other_id = other.id.clone();
            u.vault.test_entries_mut().push(other);
            u.start_edit();
            u.draft.as_mut().unwrap().entry.title = "Unsaved keyboard draft".into();
        })
    });
    update(&mut dom, |state| {
        select_tree_row(state, TreeAction::Entry(other_id.clone()));
        let (destination, original_id) = {
            let Screen::Unlocked(u) = &*state.peek() else {
                panic!("locked");
            };
            assert_ne!(u.selected.as_deref(), Some(other_id.as_str()));
            assert_eq!(
                u.draft.as_ref().unwrap().entry.title,
                "Unsaved keyboard draft"
            );
            assert_eq!(u.vault.entries()[0].title, "GitLab");
            (u.pending_navigation.clone(), u.selected.clone().unwrap())
        };
        assert!(destination.is_some());
        select_tree_row(state, TreeAction::Entry(original_id));
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(u.pending_navigation == destination);
    });
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.resolve_navigation(false))
    });
    update(&mut dom, |state| {
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert_eq!(u.selected.as_deref(), Some(other_id.as_str()));
        assert!(u.draft.is_none());
        assert_eq!(u.vault.entries()[0].title, "GitLab");
    });
}
