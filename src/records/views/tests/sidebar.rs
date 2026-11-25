use super::{TreeAction, TreeDrop, move_in_vault, select_tree_row};
use crate::{
    desktop::{
        context::edit_unlocked,
        session::Screen,
        view_test_support::{mounted, update},
    },
    records::{entry::Entry, folder::Folder},
};
use dioxus::prelude::*;

fn tree_drop(kind: &str, id: &str, parent: Option<&str>) -> TreeDrop {
    serde_json::from_value(
        serde_json::json!({ "item": { "kind": kind, "id": id }, "parent": parent }),
    )
    .unwrap()
}

#[test]
fn tree_drops_persist_entry_and_folder_moves_and_reveal_the_destination() {
    use crate::desktop::test_support::reopen;

    let mut dom = mounted();
    update(&mut dom, |state| {
        let target = Folder::new("Target".into(), None);
        let source = Folder::new("Source".into(), None);
        let child = Folder::new("Child".into(), Some(source.id.clone()));
        let mut id = String::new();
        edit_unlocked(state, |u| {
            for folder in [&target, &source, &child] {
                u.vault.save_folder(folder.clone()).unwrap();
            }
            u.directories.collapsed.insert(target.id.clone());
            id = u.selected.clone().unwrap();
        });

        move_in_vault(state, tree_drop("entry", &id, Some(&target.id)));
        {
            let Screen::Unlocked(u) = &*state.peek() else {
                panic!("locked")
            };
            let entry = &reopen(u).entries[0];
            assert_eq!(entry.id, id);
            assert_eq!(entry.password, "private-test-password");
            assert_eq!(entry.folder_id.as_ref(), Some(&target.id));
            assert_eq!(u.directories.selected.as_ref(), Some(&target.id));
            assert!(!u.directories.collapsed.contains(&target.id));
        }

        move_in_vault(state, tree_drop("folder", &source.id, Some(&target.id)));
        move_in_vault(state, tree_drop("entry", &id, None));
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked")
        };
        let saved = reopen(u);
        assert!(saved.entries[0].folder_id.is_none());
        assert!(u.directories.selected.is_none());
        assert_eq!(
            saved
                .folders
                .iter()
                .find(|f| f.id == source.id)
                .unwrap()
                .parent_id
                .as_ref(),
            Some(&target.id)
        );
        assert_eq!(
            saved
                .folders
                .iter()
                .find(|f| f.id == child.id)
                .unwrap()
                .parent_id
                .as_ref(),
            Some(&source.id)
        );
    });
}

#[test]
fn tree_drops_reject_cycles_and_leave_an_open_draft_alone() {
    let mut dom = mounted();
    let parent = Folder::new("Parent".into(), None);
    let child = Folder::new("Child".into(), Some(parent.id.clone()));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.save_folder(parent.clone()).unwrap();
            u.vault.save_folder(child.clone()).unwrap();
        });
        move_in_vault(state, tree_drop("folder", &parent.id, Some(&child.id)));
        edit_unlocked(state, |u| {
            assert!(u.vault.folders()[0].parent_id.is_none());
            assert!(
                u.error
                    .as_deref()
                    .unwrap()
                    .contains("A folder cannot contain itself")
            );
            u.start_edit();
            u.draft.as_mut().unwrap().entry.title = "Unsaved change".into();
        });
    });
    assert!(dioxus_ssr::render(&dom).contains("data-tree-can-drag=\"false\""));
    update(&mut dom, |state| {
        let id = match &*state.peek() {
            Screen::Unlocked(u) => u.selected.clone().unwrap(),
            Screen::Locked(_) => panic!("locked"),
        };
        move_in_vault(state, tree_drop("entry", &id, Some(&child.id)));
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked")
        };
        assert_eq!(u.draft.as_ref().unwrap().entry.title, "Unsaved change");
        assert!(u.selected_entry().unwrap().folder_id.is_none());
    });
}

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
fn first_autosave_selects_and_reveals_the_new_entry_while_editing() {
    use crate::desktop::{
        context::navigate, view_test_support::finish_save, workspace::Destination,
    };
    use kuchikiki::traits::TendrilSink;

    let mut dom = mounted();
    update(&mut dom, |state| {
        let parent = Folder::new("Parent folder".into(), None);
        let folder = Folder::new("Target folder".into(), Some(parent.id.clone()));
        let folder_id = folder.id.clone();
        edit_unlocked(state, |u| {
            u.directories
                .collapsed
                .extend([parent.id.clone(), folder.id.clone()]);
            u.vault.save_folder(parent).unwrap();
            u.vault.save_folder(folder).unwrap();
            u.search = "GitLab".into();
        });
        navigate(state, Destination::NewEntryInFolder(folder_id));
    });
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.draft.as_mut().unwrap().entry.title = "New folder record".into();
            u.request_autosave();
        });
        finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(u.draft.is_some());
    });

    let html = dioxus_ssr::render(&dom);
    let document = kuchikiki::parse_html().one(html).document_node;
    let mut selected = document
        .select(".entry-row.selected[aria-current='true'] .entry-row-title")
        .unwrap();
    assert_eq!(
        selected.next().unwrap().text_contents(),
        "New folder record"
    );
    assert!(selected.next().is_none());
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
