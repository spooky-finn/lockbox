use crate::{
    desktop::{
        context::{Session, edit_draft, edit_unlocked, set_field},
        session::Screen,
        view_test_support::{mounted, update},
    },
    records::{
        editor::Field,
        entry::{Totp, TotpAlgorithm},
    },
};
use dioxus::prelude::*;

#[test]
fn editor_renders_generator_icons_and_reactive_totp_link_fields() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_new());
        edit_draft(state, |d| {
            d.generator_open = true;
            d.icon_picker_open = true;
            d.icon_query = "gitlab".into();
            d.entry.totp = Some(Totp {
                secret: String::new(),
                algorithm: TotpAlgorithm::Sha1,
                digits: 6,
                period: 30,
            });
        });
        set_field(
            state,
            Field::Totp,
            "otpauth://totp/GitLab:alice?secret=JBSWY3DPEHPK3PXP".into(),
        );
    });
    let html = dioxus_ssr::render(&dom);
    for text in [
        "Новая запись",
        "Генератор паролей",
        "Выбор иконки",
        "SHA256",
        "Сохранить",
        "value=\"alice\"",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.error = u.save_draft().err())
    });
    assert!(!dioxus_ssr::render(&dom).contains("id=\"entry-title\""));
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        let vault = crate::desktop::test_support::reopen(u);
        assert_eq!(vault.entries.len(), 2);
        assert_eq!(vault.entries[1].username, "alice");
    });
}

#[test]
fn submitting_the_dioxus_form_saves_and_prevents_browser_navigation() {
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
    dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("From submit event"));
    assert!(!html.contains("id=\"entry-title\""));
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
