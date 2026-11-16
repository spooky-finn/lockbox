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
fn submitting_the_dioxus_form_flushes_without_closing_or_browser_navigation() {
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
    assert!(html.contains("id=\"entry-title\""));
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
fn footer_offers_delete_only_for_saved_entries() {
    use kuchikiki::traits::TendrilSink;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_edit());
    });
    let html = dioxus_ssr::render(&dom);
    let document = kuchikiki::parse_html().one(html.clone()).document_node;
    assert_eq!(
        document
            .select(".draft-fields > .draft-field")
            .unwrap()
            .count(),
        6
    );
    assert!(document.select_first(".draft-field .draft-field").is_err());

    assert!(
        document
            .select_first(".draft > .draft-fields > .totp-editor")
            .is_ok()
    );
    assert!(
        document
            .select_first(".draft > footer.draft-footer")
            .is_ok()
    );
    assert!(
        document
            .select_first(".draft-fields .draft-footer")
            .is_err()
    );
    let footer = html.split("<footer").nth(1).expect("action footer");
    assert!(footer.contains("draft-footer"));
    assert!(footer.contains("draft-delete"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.draft = None;
            u.start_new();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("draft-footer"));
    assert!(!html.contains("draft-delete"));
}
