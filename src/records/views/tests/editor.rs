use crate::{
    desktop::{
        context::{Session, edit_draft, edit_unlocked, set_field},
        session::Screen,
        view_test_support::{mounted, update},
    },
    records::editor::Field,
};
use dioxus::prelude::*;

#[test]
fn submitting_the_dioxus_form_saves_and_finishes_without_browser_navigation() {
    use dioxus::dioxus_core::Mutation;
    use dioxus::html::{EventData, SerializedFormData, SerializedHtmlEventConverter};
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        edit_unlocked(state, |u| u.start_new());
        set_field(state, Field::Title, "From submit event".into());
    });
    let changes = dom.render_immediate_to_vec();
    let submit_id = changes
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "submit" => Some(*id),
            _ => None,
        })
        .expect("editor has a submit listener");
    let event = Event::new(
        EventData::Form(SerializedFormData::new(String::new(), vec![])).into_any(),
        true,
    );
    dom.runtime()
        .handle_event("submit", event.clone(), submit_id);
    assert!(!event.default_action_enabled());
    dom.in_scope(ScopeId::APP, || {
        crate::desktop::view_test_support::finish_save(consume_context())
    });
    dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("From submit event"));
    assert!(!html.contains("id=\"entry-title\""));
    assert!(!html.contains("type=\"submit\""));
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        assert_eq!(
            crate::desktop::test_support::reopen(u).entries[1].title,
            "From submit event"
        );
    });
}

#[test]
fn attachment_controls_render_names_and_sizes_without_file_contents() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_new());
        set_field(state, Field::Title, "With file".into());
        edit_draft(state, |d| {
            d.entry
                .add_attachment(
                    "test-file.txt".into(),
                    b"never render file contents".to_vec(),
                )
                .unwrap();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Добавить файл"));
    assert!(html.contains("Удалить файл"));
    assert!(html.contains("test-file.txt"));
    assert!(!html.contains("never render file contents"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.save_draft().unwrap())
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("test-file.txt"));
    assert!(html.contains("Сохранить файл"));
    assert!(!html.contains("never render file contents"));
}

#[test]
fn editor_keeps_shared_field_layout_without_manual_save_or_edit_banner() {
    use kuchikiki::traits::TendrilSink;
    let mut dom = mounted();
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_edit()));
    let document = kuchikiki::parse_html()
        .one(dioxus_ssr::render(&dom))
        .document_node;
    assert!(document.select_first(".draft.entry-detail").is_ok());
    assert_eq!(
        document
            .select(".draft .fields > .field-row")
            .unwrap()
            .count(),
        5
    );
    assert!(document.select_first(".draft > .entry-close").is_err());
    assert!(
        document
            .select_first(".draft .detail-heading button.record-save")
            .is_err()
    );
    assert!(document.select_first(".draft > footer").is_err());
}

#[test]
fn notes_accept_input_without_writing_the_value_back_to_the_webview() {
    use crate::{desktop::test_support::reopen, records::entry::Entry};
    use dioxus::dioxus_core::Mutation;
    use dioxus::html::{EventData, SerializedFormData, SerializedHtmlEventConverter};

    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        edit_unlocked(consume_context(), |u| {
            u.start_edit();
            u.draft.as_mut().unwrap().entry.notes = "Original note".into();
        });
    });
    let changes = dom.render_immediate_to_vec();
    let textarea = changes
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: "rows", id, ..
            } => Some(*id),
            _ => None,
        })
        .expect("notes textarea has a rows attribute");
    assert!(changes.edits.iter().any(|edit| {
        matches!(edit, Mutation::CreateTextNode { value, .. } if value == "Original note")
    }));

    let latest = "Edited note\nSecond line";
    let event = Event::new(
        EventData::Form(SerializedFormData::new(latest.into(), vec![])).into_any(),
        true,
    );
    dom.runtime().handle_event("input", event, textarea);
    let changes = dom.render_immediate_to_vec();
    assert!(changes.edits.iter().any(|edit| {
        matches!(edit, Mutation::SetAttribute { name: "rows", id, .. } if *id == textarea)
    }));
    assert!(changes.edits.iter().all(|edit| match edit {
        Mutation::SetAttribute {
            name: "value" | "initial_value",
            id,
            ..
        } => *id != textarea,
        Mutation::SetText { value, .. } | Mutation::CreateTextNode { value, .. } => value != latest,
        _ => true,
    }));

    dom.in_scope(ScopeId::APP, || {
        edit_unlocked(consume_context(), |u| {
            let draft = u.draft.as_ref().unwrap();
            let id = draft.entry.id.clone();
            assert!(draft.autosave_at.is_some());
            u.autosave_draft().unwrap();
            let saved = reopen(u);
            assert_eq!(
                saved
                    .entries
                    .iter()
                    .find(|entry| entry.id == id)
                    .unwrap()
                    .notes,
                latest
            );

            let mut next = Entry::new();
            next.notes = "Another record's note".into();
            u.start_draft(next, true);
        });
    });
    let changes = dom.render_immediate_to_vec();
    assert!(changes.edits.iter().any(|edit| {
        matches!(edit, Mutation::CreateTextNode { value, .. } if value == "Another record's note")
    }));
}

#[test]
fn keyboard_finishes_inputs_and_keeps_notes_newlines_and_composition() {
    use dioxus::dioxus_core::Mutation;
    use dioxus::html::{EventData, SerializedHtmlEventConverter, SerializedKeyboardData};
    use std::collections::HashSet;
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    // Form-level Enter is the bubbling event from a multiline note. Single-line
    // fields handle Enter themselves; IME confirmation must never finish editing.
    for (on_input, key, modifiers, composing, finishes) in [
        (true, Key::Enter, Modifiers::empty(), false, true),
        (true, Key::Enter, Modifiers::empty(), true, false),
        (true, Key::Tab, Modifiers::empty(), false, false),
        (false, Key::Enter, Modifiers::empty(), false, false),
        (false, Key::Enter, Modifiers::CONTROL, false, true),
        (false, Key::Enter, Modifiers::META, false, true),
    ] {
        let mut dom = mounted();
        dom.in_scope(ScopeId::APP, || {
            let state = consume_context::<Session>();
            edit_unlocked(state, |u| u.start_new());
            set_field(state, Field::Title, "Keyboard entry".into());
            set_field(state, Field::Notes, "Line one\nLine two".into());
        });
        let changes = dom.render_immediate_to_vec();
        let input_ids: HashSet<_> = changes
            .edits
            .iter()
            .filter_map(|edit| match edit {
                Mutation::NewEventListener { name, id } if name == "input" => Some(*id),
                _ => None,
            })
            .collect();
        let target = changes
            .edits
            .iter()
            .find_map(|edit| match edit {
                Mutation::NewEventListener { name, id }
                    if (on_input && name == "keydown" && input_ids.contains(id))
                        || (!on_input && name == "submit") =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .unwrap();
        let event = Event::new(
            EventData::Keyboard(SerializedKeyboardData::new(
                key,
                Code::Unidentified,
                Location::Standard,
                false,
                modifiers,
                composing,
            ))
            .into_any(),
            true,
        );
        dom.runtime().handle_event("keydown", event.clone(), target);
        dom.in_scope(ScopeId::APP, || {
            crate::desktop::view_test_support::finish_save(consume_context())
        });
        dom.render_immediate_to_vec();
        assert_eq!(event.default_action_enabled(), !finishes);
        dom.in_scope(ScopeId::APP, || {
            let state = consume_context::<Session>();
            let screen = state.peek();
            let Screen::Unlocked(u) = &*screen else {
                panic!("locked");
            };
            assert_eq!(u.draft.is_none(), finishes);
            if finishes {
                let saved = crate::desktop::test_support::reopen(u);
                let entry = saved
                    .entries
                    .iter()
                    .find(|e| e.title == "Keyboard entry")
                    .unwrap();
                assert_eq!(entry.notes, "Line one\nLine two");
            }
        });
    }
}

#[test]
fn editing_an_entry_keeps_sidebar_navigation_available() {
    use kuchikiki::traits::TendrilSink;
    let mut dom = mounted();
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_edit()));
    let document = kuchikiki::parse_html()
        .one(dioxus_ssr::render(&dom))
        .document_node;
    assert!(document.select_first(".entry-row[disabled]").is_err());
    assert!(
        document
            .select_first(".sidebar-footer button[disabled]")
            .is_err()
    );
    assert!(
        document
            .select_first(".sidebar-heading button[disabled]")
            .is_err()
    );
    assert!(document.select_first(".draft > .entry-close").is_err());
}
