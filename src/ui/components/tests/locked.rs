use crate::ui::components::test_support::{mounted, update};
use crate::ui::runtime;

#[test]
fn creation_starts_with_storage_choice_and_cloud_has_separate_fields() {
    let mut dom = mounted();
    update(&mut dom, |_| runtime::begin_create());
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Однопользовательское"));
    assert!(html.contains("Многопользовательское"));
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |state| {
        crate::ui::context::edit_locked(state, |form| {
            form.storage_kind = Some(crate::app::auth::StorageKind::Cloud);
        });
    });
    let html = dioxus_ssr::render(&dom);
    for label in [
        "Имя пользователя",
        "Имя хранилища",
        "Base URL",
        "Access Key",
        "Secret Key",
    ] {
        assert!(html.contains(label));
    }
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |_| runtime::begin_create());
    assert!(!dioxus_ssr::render(&dom).contains("id=\"cloud-secret\""));
}

#[test]
fn cloud_submit_validates_then_shows_password_without_enabling_creation() {
    use crate::app::auth::StorageKind;
    use crate::ui::context::{Session, edit_locked};
    use dioxus::dioxus_core::Mutation;
    use dioxus::html::{EventData, SerializedFormData, SerializedHtmlEventConverter};
    use dioxus::prelude::*;
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        runtime::begin_create();
        edit_locked(consume_context::<Session>(), |form| {
            form.storage_kind = Some(StorageKind::Cloud);
        });
    });
    let changes = dom.render_immediate_to_vec();
    let submit_id = changes
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "submit" => Some(*id),
            _ => None,
        })
        .expect("cloud form has a submit listener");
    let submit = || {
        Event::new(
            EventData::Form(SerializedFormData::new(String::new(), vec![])).into_any(),
            true,
        )
    };
    let event = submit();
    dom.runtime()
        .handle_event("submit", event.clone(), submit_id);
    assert!(!event.default_action_enabled());
    dom.render_immediate_to_vec();
    assert!(!dioxus_ssr::render(&dom).contains("id=\"master-password\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.cloud.username = "Alice".into();
            form.cloud.space_name = "Team".into();
            form.cloud.base_url = "https://gateway.storjshare.io".into();
            form.cloud.bucket = "team-vault".into();
            form.cloud.access_key.push_str("test-access");
            form.cloud.secret_key.push_str("test-secret");
        })
    });
    dom.runtime().handle_event("submit", submit(), submit_id);
    dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"master-password\""));
    assert!(html.contains("Назад к подключению"));
    assert!(!html.contains("test-secret"));
    assert!(!html.contains("test-access"));
    let create_button = html
        .split("<button")
        .find(|button| button.contains("aria-label=\"Создать хранилище\""))
        .expect("create action exists");
    assert!(
        create_button
            .split('>')
            .next()
            .unwrap()
            .contains("disabled")
    );
}
