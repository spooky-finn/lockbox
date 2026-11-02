use crate::desktop::{
    runtime,
    view_test_support::{mounted, update},
};

#[test]
fn cloud_restore_requires_original_words_and_has_no_create_or_join_storage_choice() {
    use crate::{
        access::auth::{CreationStep, StorageKind},
        desktop::context::edit_locked,
    };
    let mut dom = mounted();
    update(&mut dom, |_| runtime::begin_cloud_restore());
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Восстановить из Storj"));
    assert!(html.contains("прежние 12 слов"));
    assert!(html.contains("id=\"recovery-words\"") || html.contains("recovery-word"));
    assert!(!html.contains("Сгенерировать"));
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.creation_step = CreationStep::Sync;
            form.storage_kind = Some(StorageKind::Cloud);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"cloud-bucket\""));
    assert!(!html.contains("id=\"cloud-space\""));
    assert!(!html.contains("id=\"cloud-username\""));
    assert!(!html.contains("storage-choices"));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.creation_step = CreationStep::Password)
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"cloud-recovery-confirm\""));
    assert!(html.contains("невозможно проверить"));
}

#[test]
fn creation_requires_key_setup_before_storage_choice_and_cloud_fields() {
    use crate::desktop::context::{Session, edit_locked};
    use dioxus::{
        dioxus_core::Mutation,
        html::{EventData, SerializedFormData, SerializedHtmlEventConverter},
        prelude::*,
    };
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    dom.in_scope(ScopeId::APP, runtime::begin_create);
    let changes = dom.render_immediate_to_vec();
    let submit_id = changes
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "submit" => Some(*id),
            _ => None,
        })
        .expect("key setup form submit listener");
    let submit = || {
        Event::new(
            EventData::Form(SerializedFormData::new(String::new(), vec![])).into_any(),
            true,
        )
    };
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Настройте ключ шифрования"));
    assert!(!html.contains("id=\"recovery-saved\""));
    assert!(!html.contains("storage-choices"));
    assert!(!html.contains("id=\"cloud-secret\""));
    assert!(!html.contains("id=\"master-password\""));
    let event = submit();
    dom.runtime()
        .handle_event("submit", event.clone(), submit_id);
    assert!(!event.default_action_enabled());
    dom.render_immediate_to_vec();
    assert!(!dioxus_ssr::render(&dom).contains("storage-choices"));
    dom.in_scope(ScopeId::APP, || {
        edit_locked(consume_context::<Session>(), |form| {
            form.words_saved = true;
        });
    });
    dom.runtime().handle_event("submit", submit(), submit_id);
    dom.render_immediate_to_vec();
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
fn local_password_step_keeps_key_hidden_and_back_returns_to_sync_choice() {
    use crate::desktop::context::edit_locked;
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    update(&mut dom, |_| {
        runtime::set_language(crate::settings::language::Language::English);
        runtime::begin_create();
    });
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.words_saved = true;
            form.continue_key_setup();
            form.choose_local_path(dir.path().join("local"));
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"master-password\""));
    assert!(html.contains("id=\"repeat-password\""));
    assert!(html.contains("Back to sync settings"));
    assert!(!html.contains("id=\"recovery-words\""));
    assert!(!html.contains("id=\"cloud-secret\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.edit_sync_setup());
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("storage-choices"));
    assert!(!html.contains("id=\"master-password\""));
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
            form.words_saved = true;
            form.continue_key_setup();
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
    assert!(!html.contains("lock-emblem"));
    assert!(!html.contains("id=\"recovery-words\""));
    assert!(!html.contains("id=\"recovery-saved\""));
}

#[test]
fn joining_sets_up_key_before_connection_without_creating_a_named_space() {
    use crate::desktop::context::edit_locked;

    let mut dom = mounted();
    update(&mut dom, |_| {
        runtime::set_language(crate::settings::language::Language::English);
        runtime::begin_join();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Set up your encryption key"));
    assert!(!html.contains("id=\"cloud-secret\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.words_saved = true;
            form.continue_key_setup();
        });
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
            assert!(form.creation_step == crate::access::auth::CreationStep::Password);
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"master-password\""));
    assert!(html.contains("id=\"repeat-password\""));
    assert!(!html.contains("id=\"recovery-password\""));
    update(&mut dom, |_| runtime::begin_create());
    assert!(!dioxus_ssr::render(&dom).contains("storage-choices"));
    assert!(dioxus_ssr::render(&dom).contains("Set up your encryption key"));
}

#[test]
fn recovery_words_are_hidden_after_key_setup_and_preserved_on_back() {
    use crate::desktop::context::edit_locked;
    let mut dom = mounted();
    let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    update(&mut dom, |_| {
        runtime::set_language(crate::settings::language::Language::English);
        runtime::begin_create();
    });
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            zeroize::Zeroize::zeroize(&mut form.recovery_words);
            form.recovery_words.push_str(words);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Show"));
    assert!(html.contains("Copy"));
    assert!(html.contains("Download"));
    assert!(!html.contains("recovery-word-display"));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.show_recovery_words = true)
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("recovery-word-display"));
    assert_eq!(html.matches(">abandon</span>").count(), 11);
    assert!(html.contains(">about</span>"));
    assert!(!html.contains("id=\"recovery-saved\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.continue_key_setup());
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Enter three words from your saved mnemonic"));
    assert!(!html.contains("recovery-word-display"));
    assert!(!html.contains(">abandon</span>"));
    assert_eq!(html.matches("type=\"password\"").count(), 3);
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            for (slot, index) in form.word_challenge.unwrap().iter().enumerate() {
                form.word_answers[slot].push_str(words.split_whitespace().nth(*index).unwrap());
            }
            form.continue_key_setup();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(!html.contains("recovery-word-display"));
    assert!(!html.contains(">abandon</span>"));
    assert!(html.contains("storage-choices"));
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            assert_eq!(form.recovery_words.as_str(), words);
            form.edit_recovery_key();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Show"));
    assert!(!html.contains(">abandon</span>"));
    update(&mut dom, |_| runtime::begin_create());
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            assert_eq!(form.recovery_words.as_str(), words);
            assert!(form.words_saved);
        })
    });
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

#[test]
fn saved_identity_requires_unlock_and_never_offers_key_generation() {
    use crate::{
        access::mnemonic::RecoveryPhrase,
        desktop::{context::Session, session::Screen},
        settings::state::Settings,
        vault::{KdfParams, identity::Identity},
    };
    use dioxus::prelude::*;
    use secrecy::SecretString;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.key");
    let identity = Identity::from_phrase(&RecoveryPhrase::generate().unwrap());
    identity
        .save_new(
            &path,
            &SecretString::from("identity password".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
        )
        .unwrap();
    let mut dom = mounted();
    update(&mut dom, |_| {
        runtime::set_language(crate::settings::language::Language::English);
        consume_context::<Signal<Settings>>().write().path =
            Some(directory.path().join("config.json"));
        runtime::begin_join();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Unlock your identity"));
    assert!(!html.contains("Set up your encryption key"));
    assert!(!html.contains("Show"));
    assert!(!html.contains("I already have 12 words"));
    assert!(!html.contains("id=\"repeat-password\""));
    assert!(!html.contains("id=\"cloud-secret\""));
    update(&mut dom, |mut state| {
        let mut screen = state.write();
        let Screen::Locked(form) = &mut *screen else {
            panic!("unlocked")
        };
        assert!(form.recovery_words.is_empty());
        form.identity = Some(identity.clone());
        form.creation_step = crate::access::auth::CreationStep::Sync;
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Using your existing private identity key"));
    assert!(!html.contains("Back to encryption key"));
    update(&mut dom, |_| runtime::begin_create());
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("unlocked")
        };
        assert_eq!(
            form.identity.as_ref().unwrap().signing_key().to_bytes(),
            identity.signing_key().to_bytes()
        );
    });
    assert!(dioxus_ssr::render(&dom).contains("storage-choices"));
}
