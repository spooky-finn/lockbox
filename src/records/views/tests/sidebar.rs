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
fn keyboard_navigation_saves_changed_drafts_before_switching() {
    let mut dom = mounted();
    let mut other_id = String::new();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let mut other = Entry::new();
            other.title = "Other keyboard record".into();
            other_id = other.id.clone();
            u.vault.test_entries_mut().push(other);
            u.start_edit();
            u.draft.as_mut().unwrap().entry.title = "Saved keyboard draft".into();
        })
    });
    update(&mut dom, |state| {
        select_tree_row(state, TreeAction::Entry(other_id.clone()));
        crate::desktop::view_test_support::finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert_eq!(u.selected.as_deref(), Some(other_id.as_str()));
        assert!(u.draft.is_none());
        assert_eq!(
            crate::desktop::test_support::reopen(u).entries[0].title,
            "Saved keyboard draft"
        );
    });
}

#[test]
fn folder_row_creates_a_record_in_the_clicked_folder_after_saving_pending_edits() {
    use crate::desktop::{context::Session, test_support::reopen, view_test_support::finish_save};
    use dioxus::{
        dioxus_core::{AttributeValue, Mutation},
        html::{EventData, SerializedHtmlEventConverter, SerializedMouseData},
    };

    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    let mut folder_id = String::new();
    dom.in_scope(ScopeId::APP, || {
        edit_unlocked(consume_context::<Session>(), |u| {
            let parent = Folder::new("Parent folder".into(), None);
            let folder = Folder::new("Target folder".into(), Some(parent.id.clone()));
            folder_id = folder.id.clone();
            u.vault.save_folder(parent).unwrap();
            u.vault.save_folder(folder).unwrap();
            u.directories.collapsed.insert(folder_id.clone());
            u.start_edit();
            u.draft.as_mut().unwrap().entry.title = "Saved before switching".into();
        });
    });
    let edits = dom.render_immediate_to_vec().edits;
    let button = edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: "aria-label",
                value: AttributeValue::Text(label),
                id,
                ..
            } if label == "Новая запись в папке Target folder" => Some(*id),
            _ => None,
        })
        .unwrap();
    dom.runtime().handle_event(
        "click",
        Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        ),
        button,
    );
    update(&mut dom, |state| {
        finish_save(state);
        edit_unlocked(state, |u| {
            assert_eq!(reopen(u).entries[0].title, "Saved before switching");
            assert!(u.directories.collapsed.contains(&folder_id));

            let draft = u.draft.as_mut().unwrap();
            assert!(draft.is_empty());
            assert_eq!(draft.entry.folder_id.as_ref(), Some(&folder_id));
            draft.entry.title = "New folder record".into();
            u.request_save();
        });
        finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        let saved = reopen(u);
        let entry = saved
            .entries
            .iter()
            .find(|entry| entry.title == "New folder record")
            .unwrap();
        assert_eq!(entry.folder_id.as_ref(), Some(&folder_id));
    });
}
