use crate::desktop::{
    context::edit_unlocked,
    runtime,
    view_test_support::{mounted, update},
};

#[test]
fn selected_entry_hides_secrets_and_escapes_user_text() {
    use kuchikiki::traits::TendrilSink;

    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.test_entries_mut()[0].title = "<script>bad()</script>".into();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("alice"));
    assert!(html.contains("bad()"));
    assert!(!html.contains("<script>bad()"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("JBSWY3DPEHPK3PXP"));
    assert!(html.contains("Код TOTP"));

    let document = kuchikiki::parse_html().one(html).document_node;
    for selector in [
        "dd > button.copy-value.field-value",
        "dd > button.copy-value + button[aria-pressed]",
        "dd > button.url-value + a.open-record-url[href='https://gitlab.com/']",
        "button.copy-value > .totp-code",
    ] {
        assert!(document.select_first(selector).is_ok(), "{selector}");
    }
    assert!(document.select_first(".field-copy").is_err());
}

#[test]
fn showing_then_locking_removes_the_entry_and_password() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.show_password = true)
    });
    assert!(dioxus_ssr::render(&dom).contains("private-test-password"));
    update(&mut dom, |_| runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Парольная фраза"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("alice"));
    assert!(!html.contains("entry-detail"));
}

#[test]
fn history_hides_secrets_and_trash_restores_record() {
    use crate::desktop::workspace::Page;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.save_changes().unwrap();
            u.vault.test_entries_mut()[0].password = "new-history-secret".into();
            u.vault.save_changes().unwrap();
            u.open_records_page(Page::History);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("История записи"));
    assert!(html.contains("Автор: Local user"));
    assert!(html.contains("Сохранено (UTC):"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("new-history-secret"));
    assert!(!html.contains("JBSWY3DPEHPK3PXP"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.delete_selected().unwrap();
            u.open_records_page(Page::Trash);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("GitLab"));
    assert!(html.contains("Восстановить"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let id = u.vault.trash().unwrap()[0].id.clone();
            u.restore_record(&id);
            assert_eq!(u.vault.entries().len(), 1);
        })
    });
    assert!(!dioxus_ssr::render(&dom).contains("new-history-secret"));
}

#[test]
fn trash_dialog_escape_keeps_the_record_open_and_confirmation_trashes_it() {
    use crate::desktop::context::Session;
    use dioxus::{
        dioxus_core::Mutation,
        html::{
            EventData, SerializedCancelData, SerializedHtmlEventConverter, SerializedKeyboardData,
            SerializedMouseData,
        },
        prelude::*,
    };

    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.vault.save_changes().unwrap());
    });
    let request_confirmation = |dom: &mut VirtualDom| {
        dom.in_scope(ScopeId::APP, || {
            edit_unlocked(consume_context::<Session>(), |u| u.confirm_delete = true);
        });
        dom.render_immediate_to_vec()
    };
    let dialog = request_confirmation(&mut dom)
        .edits
        .into_iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "cancel" => Some(id),
            _ => None,
        })
        .unwrap();
    dom.runtime().handle_event(
        "keydown",
        Event::new(
            EventData::Keyboard(SerializedKeyboardData::new(
                Key::Escape,
                Code::Escape,
                Location::Standard,
                false,
                Modifiers::empty(),
                false,
            ))
            .into_any(),
            true,
        ),
        dialog,
    );
    dom.runtime().handle_event(
        "cancel",
        Event::new(EventData::Cancel(SerializedCancelData {}).into_any(), false),
        dialog,
    );
    dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("entry-detail"));
    assert!(!html.contains("entry-trash-dialog"));

    let confirm = request_confirmation(&mut dom)
        .edits
        .into_iter()
        .rev()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "click" => Some(id),
            _ => None,
        })
        .unwrap();
    dom.runtime().handle_event(
        "click",
        Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        ),
        confirm,
    );
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            assert!(u.selected.is_none());
            assert_eq!(u.vault.trash().unwrap()[0].title, "GitLab");
        });
    });
}

#[test]
fn history_follows_selection_and_keeps_other_record_contents_hidden() {
    use crate::{desktop::workspace::Page, records::entry::Entry};
    let mut dom = mounted();
    let mut other_id = String::new();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.save_changes().unwrap();
            let mut other = Entry::new();
            other.title = "Other history record".into();
            other.username = "other-history-login".into();
            other.password = "other-history-secret".into();
            other_id = other.id.clone();
            u.vault.upsert_entry(other).unwrap();
            u.open_records_page(Page::History);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("alice"));
    assert!(!html.contains("other-history-login"));
    assert!(!html.contains("private-test-password"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.selected = Some(other_id))
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("other-history-login"));
    assert!(!html.contains("other-history-secret"));
    for selected in [Some("missing".to_owned()), None] {
        update(&mut dom, |state| {
            edit_unlocked(state, |u| u.selected = selected)
        });
        let html = dioxus_ssr::render(&dom);
        assert!(!html.contains("class=\"history-change\""));
        assert!(!html.contains("other-history-login"));
        assert!(
            html.contains(
                crate::settings::language::Language::Russian
                    .text(crate::desktop::i18n::Text::NoRecordHistory)
            )
        );
    }
}

#[test]
fn double_click_edits_and_escape_saves_then_closes_the_record_on_second_press() {
    use crate::desktop::{context::Session, context::set_field, session::Screen};
    use crate::records::editor::Field;
    use dioxus::html::{
        EventData, SerializedHtmlEventConverter, SerializedKeyboardData, SerializedMouseData,
    };
    use dioxus::{dioxus_core::Mutation, prelude::*};
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = VirtualDom::new(crate::desktop::view_test_support::fixture);
    let initial = dom.rebuild_to_vec();
    let double_click = initial
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "dblclick" => Some(*id),
            _ => None,
        })
        .unwrap();
    let escape_target = initial
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: "class",
                value: dioxus::dioxus_core::AttributeValue::Text(class),
                id,
                ..
            } if class.starts_with("workspace") => Some(*id),
            _ => None,
        })
        .unwrap();
    dom.runtime().handle_event(
        "dblclick",
        Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        ),
        double_click,
    );
    dom.render_immediate_to_vec();
    assert!(dioxus_ssr::render(&dom).contains("id=\"entry-title\""));
    dom.in_scope(ScopeId::APP, || {
        set_field(consume_context::<Session>(), Field::Title, "Changed".into());
    });
    dom.render_immediate_to_vec();
    let escape = || {
        Event::new(
            EventData::Keyboard(SerializedKeyboardData::new(
                Key::Escape,
                Code::Escape,
                Location::Standard,
                false,
                Modifiers::empty(),
                false,
            ))
            .into_any(),
            true,
        )
    };
    dom.runtime()
        .handle_event("keydown", escape(), escape_target);
    dom.in_scope(ScopeId::APP, || {
        crate::desktop::view_test_support::finish_save(consume_context())
    });
    dom.render_immediate_to_vec();
    assert!(!dioxus_ssr::render(&dom).contains("record-save-dialog"));
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.peek();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        assert!(u.draft.is_none());
        assert!(u.selected.is_some());
        assert_eq!(
            crate::desktop::test_support::reopen(u).entries[0].title,
            "Changed"
        );
    });
    dom.runtime()
        .handle_event("keydown", escape(), escape_target);
    dom.render_immediate_to_vec();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.peek();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        assert!(u.selected.is_none());
    });
}

#[test]
fn notes_render_as_selectable_plain_text() {
    use kuchikiki::traits::TendrilSink;

    let mut dom = mounted();
    let text = "First <line>\nSelected part\nLast line";
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.test_entries_mut()[0].notes = text.into();
        });
    });

    let document = kuchikiki::parse_html()
        .one(dioxus_ssr::render(&dom))
        .document_node;
    let note = document.select_first(".notes-value").unwrap();
    assert_eq!(note.name.local.as_ref(), "span");
    assert_eq!(note.text_contents(), text);
    assert!(document.select_first(".notes-row button").is_err());

    update(&mut dom, |_| runtime::lock(None));
    assert!(!dioxus_ssr::render(&dom).contains("Selected part"));
}

#[test]
fn clock_ticks_update_totp_without_rerendering_the_record_card() {
    use crate::desktop::context::Clock;
    use dioxus::prelude::*;
    let mut dom = mounted();
    let before = super::ENTRY_RENDERS.get();
    update(&mut dom, |_| {
        let mut clock = consume_context::<Clock>().0;
        clock.set(59);
    });
    assert_eq!(super::ENTRY_RENDERS.get(), before);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("max=\"30\" value=\"1\""));
}
