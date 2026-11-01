use crate::desktop::{
    runtime,
    view_test_support::{mounted, update},
};

#[test]
fn creation_starts_with_storage_choice_and_cloud_has_separate_fields() {
    let mut dom = mounted();
    update(&mut dom, |_| runtime::begin_create());
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Однопользовательское"));
    assert!(html.contains("Многопользовательское"));
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |state| {
        crate::desktop::context::edit_locked(state, |form| {
            form.storage_kind = Some(crate::access::auth::StorageKind::Cloud);
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
fn cloud_submit_validates_then_offers_identity_creation() {
    use crate::access::auth::StorageKind;
    use crate::desktop::context::Session;
    use crate::desktop::context::edit_locked;
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
    assert!(html.contains("id=\"repeat-password\""));
    assert!(html.contains("Назад к подключению"));
    assert!(!html.contains("test-secret"));
    assert!(!html.contains("test-access"));
    assert!(!html.contains("id=\"recovery-password\""));
    assert!(!html.contains("id=\"recovery-repeat\""));
    assert!(html.contains("Создать профиль и ключи…"));
}

#[test]
fn joining_starts_at_connection_setup_without_creating_a_named_space() {
    use crate::desktop::context::edit_locked;

    let mut dom = mounted();
    update(&mut dom, |_| {
        runtime::set_language(crate::settings::language::Language::English);
        runtime::begin_join();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Join a space"));
    assert!(html.contains("id=\"cloud-username\""));
    assert!(html.contains("id=\"cloud-secret\""));
    assert!(!html.contains("id=\"cloud-space\""));
    assert!(!html.contains("storage-choices"));
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.cloud.username = "Guest".into();
            form.cloud.base_url = "https://gateway.storjshare.io".into();
            form.cloud.bucket = "team-vault".into();
            form.cloud.access_key.push_str("fictional-access");
            form.cloud.secret_key.push_str("fictional-secret");
            form.continue_cloud();
            assert!(form.joining);
            assert!(form.cloud_password_step);
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"master-password\""));
    assert!(html.contains("id=\"repeat-password\""));
    assert!(!html.contains("id=\"recovery-password\""));
    update(&mut dom, |_| runtime::begin_create());
    assert!(dioxus_ssr::render(&dom).contains("storage-choices"));
}

#[test]
fn recovery_words_are_hidden_until_revealed_and_removed_when_setup_is_left() {
    use crate::desktop::context::edit_locked;
    let mut dom = mounted();
    let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    update(&mut dom, |_| {
        runtime::set_language(crate::settings::language::Language::English);
        runtime::begin_create();
    });
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.storage_kind = Some(crate::access::auth::StorageKind::Cloud);
            form.cloud_password_step = true;
            form.recovery_words.push_str(words);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Show recovery words"));
    assert!(!html.contains(words));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.show_recovery_words = true)
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains(words));
    assert!(html.contains("id=\"recovery-saved\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.edit_cloud_connection())
    });
    assert!(!dioxus_ssr::render(&dom).contains(words));
}

#[test]
fn forgot_password_shows_words_and_new_password_confirmation() {
    use crate::desktop::context::edit_locked;
    use dioxus::prelude::WritableExt;
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.lbx");
    std::fs::write(&path, []).unwrap();
    update(&mut dom, |_| {
        runtime::set_language(crate::settings::language::Language::English)
    });
    update(&mut dom, |mut state| {
        *state.write() = crate::desktop::session::Screen::Locked(Box::new(
            crate::access::auth::Locked::new(Some(path.clone())),
        ));
        edit_locked(state, |form| form.toggle_recovery());
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Recover access"));
    assert!(html.contains("New master password"));
    assert!(html.contains("id=\"repeat-password\""));
    assert!(html.contains("id=\"recovery-words\""));
    assert!(html.contains("Reset password and unlock"));
    assert!(!html.contains("id=\"recovery-saved\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.toggle_recovery())
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Forgot master password?"));
    assert!(!html.contains("id=\"recovery-words\""));
}
