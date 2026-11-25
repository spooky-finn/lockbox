use crate::desktop::profiles;
use crate::desktop::view_test_support::{mounted, update};
use crate::settings::actions;

#[test]
fn arrows_change_the_centered_vault_name_and_clear_the_password() {
    use crate::{
        access::auth::Locked,
        desktop::{context::Session, session::Screen},
        settings::{language::Language, state::Settings},
    };
    use dioxus::{
        dioxus_core::{AttributeValue, Mutation},
        html::{EventData, SerializedHtmlEventConverter, SerializedMouseData},
        prelude::*,
    };
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("Personal.lbx");
    let missing = dir.path().join("missing.lbx");
    let cloud = dir.path().join("profile-123.lbx");
    std::fs::write(&local, []).unwrap();
    std::fs::write(&cloud, []).unwrap();
    dom.in_scope(ScopeId::APP, || {
        let mut settings = consume_context::<Signal<Settings>>();
        let mut preferences = settings.write();
        preferences.config.language = Language::English;
        preferences.config.recent_vaults = vec![local.clone(), missing, cloud.clone()];
        preferences
            .config
            .space_names
            .insert(cloud.clone(), "Design team".into());
        let mut form = Locked::new(Some(local.clone()));
        form.password.push_str("entered for Personal");
        consume_context::<Session>().set(Screen::Locked(Box::new(form)));
    });
    let changes = dom.render_immediate_to_vec();
    let arrow = |direction: &str| {
        let class = format!("icon-button quiet unlock-{direction}");
        changes
            .edits
            .iter()
            .find_map(|edit| match edit {
                Mutation::SetAttribute {
                    name: "class",
                    value: AttributeValue::Text(value),
                    id,
                    ..
                } if *value == class => Some(*id),
                _ => None,
            })
            .unwrap()
    };
    let heading = |dom: &VirtualDom, name: &str| {
        let html = dioxus_ssr::render(dom);
        assert!(html.contains(&format!(
            "<h1 class=\"unlock-title\" aria-live=\"polite\">{name}</h1>"
        )));
    };
    heading(&dom, "Personal");
    for (direction, path, name) in [
        ("next", &cloud, "Design team"),
        ("next", &local, "Personal"),
        ("previous", &cloud, "Design team"),
        ("previous", &local, "Personal"),
    ] {
        let event = Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        );
        dom.runtime().handle_event("click", event, arrow(direction));
        dom.render_immediate_to_vec();
        heading(&dom, name);
        dom.in_scope(ScopeId::APP, || {
            let state = consume_context::<Session>();
            let screen = state.peek();
            let Screen::Locked(form) = &*screen else {
                panic!("expected password unlock");
            };
            assert_eq!(form.path.as_ref(), Some(path));
            assert!(form.password.is_empty());
        });
    }
}

#[test]
fn arrows_return_when_another_tab_releases_a_vault() {
    use crate::{
        access::auth::Locked,
        desktop::{session::Screen, tabs},
        settings::state::Settings,
    };
    use dioxus::prelude::*;

    let mut dom = crate::desktop::view_test_support::mounted_with_header();
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.lbx");
    let second = dir.path().join("second.lbx");
    std::fs::write(&first, []).unwrap();
    std::fs::write(&second, []).unwrap();
    update(&mut dom, |mut state| {
        consume_context::<Signal<Settings>>()
            .write()
            .config
            .recent_vaults = vec![first.clone(), second.clone()];
        state.set(Screen::Locked(Box::new(Locked::new(Some(first)))));
        tabs::new_tab();
        state.set(Screen::Locked(Box::new(Locked::new(Some(second)))));
    });
    assert!(!dioxus_ssr::render(&dom).contains("unlock-next"));

    update(&mut dom, |_| tabs::close(0));
    assert!(dioxus_ssr::render(&dom).contains("unlock-next"));
}

#[test]
fn cloud_restore_requires_original_words_and_has_no_create_or_join_storage_choice() {
    use crate::{
        access::auth::{CreationStep, StorageKind},
        desktop::context::edit_locked,
    };
    let mut dom = mounted();
    update(&mut dom, |_| profiles::begin_cloud_restore());
    let html = dioxus_ssr::render(&dom);
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
    dom.in_scope(ScopeId::APP, profiles::begin_create);
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
    assert!(html.contains("storage-choices"));
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |state| {
        crate::desktop::context::edit_locked(state, |form| {
            form.storage_kind = Some(crate::access::auth::StorageKind::Cloud);
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"cloud-secret\""));
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |_| profiles::begin_create());
    assert!(!dioxus_ssr::render(&dom).contains("id=\"cloud-secret\""));
}

#[test]
fn local_password_step_keeps_key_hidden_and_back_returns_to_sync_choice() {
    use crate::desktop::context::edit_locked;
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    update(&mut dom, |_| {
        actions::set_language(crate::settings::language::Language::English);
        profiles::begin_create();
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
        profiles::begin_create();
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
    assert!(!html.contains("test-secret"));
    assert!(!html.contains("test-access"));
    assert!(!html.contains("id=\"recovery-password\""));
    assert!(!html.contains("id=\"recovery-repeat\""));
    assert!(!html.contains("id=\"recovery-words\""));
    assert!(!html.contains("id=\"recovery-saved\""));
}

#[test]
fn joining_sets_up_key_before_connection_without_creating_a_named_space() {
    use crate::desktop::context::edit_locked;

    let mut dom = mounted();
    update(&mut dom, |_| {
        actions::set_language(crate::settings::language::Language::English);
        profiles::begin_join();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Set up your mnemonic"));
    assert!(!html.contains("id=\"cloud-secret\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.words_saved = true;
            form.continue_key_setup();
        });
    });
    let html = dioxus_ssr::render(&dom);
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
    update(&mut dom, |_| profiles::begin_create());
    assert!(!dioxus_ssr::render(&dom).contains("storage-choices"));
    assert!(dioxus_ssr::render(&dom).contains("Set up your mnemonic"));
}

#[test]
fn recovery_words_are_hidden_after_key_setup_and_preserved_on_back() {
    use crate::desktop::context::edit_locked;
    let mut dom = mounted();
    let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    update(&mut dom, |_| {
        actions::set_language(crate::settings::language::Language::English);
        profiles::begin_create();
    });
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            zeroize::Zeroize::zeroize(&mut form.recovery_words);
            form.recovery_words.push_str(words);
        })
    });
    let html = dioxus_ssr::render(&dom);
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
    assert!(!html.contains(">abandon</span>"));
    update(&mut dom, |_| profiles::begin_create());
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
        actions::set_language(crate::settings::language::Language::English)
    });
    update(&mut dom, |mut state| {
        *state.write() = crate::desktop::session::Screen::Locked(Box::new(
            crate::access::auth::Locked::new(Some(path.clone())),
        ));
        edit_locked(state, |form| form.toggle_recovery());
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"master-password\""));
    assert!(html.contains("id=\"repeat-password\""));
    assert!(html.contains("id=\"recovery-words\""));
    assert!(!html.contains("id=\"recovery-saved\""));
    update(&mut dom, |state| {
        edit_locked(state, |form| form.toggle_recovery())
    });
    let html = dioxus_ssr::render(&dom);
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
        actions::set_language(crate::settings::language::Language::English);
        consume_context::<Signal<Settings>>().write().path =
            Some(directory.path().join("config.json"));
        profiles::begin_join();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Unlock your mnemonic"));
    assert!(!html.contains("Set up your mnemonic"));
    assert!(!html.contains("Show"));
    assert!(!html.contains("Use existing mnemonic"));
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
    assert!(html.contains("Using your existing mnemonic"));
    assert!(!html.contains("Back to mnemonic"));
    update(&mut dom, |_| profiles::begin_create());
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

#[test]
fn managed_invitation_uses_username_and_still_requires_a_local_password() {
    use crate::{
        access::auth::CreationStep,
        desktop::context::edit_locked,
        sync::protocol::pairing::ManagedLink,
        sync::transport::storj::invite::Owner,
        vault::{
            KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
        },
    };
    use secrecy::SecretString;
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(std::sync::Arc::new(MemoryWitness::default()));
    let endpoint = "https://gateway.storjshare.io";
    let mut owner = store
        .create_shared(
            dir.path().join("owner.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Owner".into(),
                space_name: "Team".into(),
                provider: ProviderConfiguration {
                    base_url: endpoint.into(),
                    bucket: "fixture".into(),
                    access_key: "fixture-access".into(),
                    secret_key: "fixture-secret".into(),
                },
            },
        )
        .unwrap();
    let id = owner.create_link(0, 1).unwrap();
    let ticket = owner.issued_links().next().unwrap().ticket.clone();
    let grant = Owner::new(
        endpoint,
        "fixture",
        "Team",
        "fixture-access",
        "fixture-secret",
        id,
    )
    .unwrap()
    .grant(std::time::Duration::from_secs(3600))
    .unwrap();
    let link = ManagedLink::create(ticket, grant, &owner.test_profile().unwrap().sync_secret())
        .unwrap()
        .link()
        .unwrap();
    let mut dom = mounted();
    update(&mut dom, |_| profiles::begin_join());
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.invitation_link = Some(link.clone());
            form.continue_cloud();
            assert!(form.creation_step == CreationStep::EncryptionKey);
            form.words_saved = true;
            form.continue_key_setup();
            form.continue_cloud();
            assert!(form.creation_step == CreationStep::Sync);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"setup-invitation\""));
    assert!(html.contains("id=\"cloud-username\""));
    for input in ["cloud-url", "cloud-bucket", "cloud-access", "cloud-secret"] {
        assert!(!html.contains(&format!("id=\"{input}\"")));
    }
    update(&mut dom, |state| {
        edit_locked(state, |form| {
            form.cloud.username = "Guest".into();
            form.invitation_link = Some("lockbox://pair/invalid".into());
            form.continue_cloud();
            assert!(form.creation_step == CreationStep::Sync);
            form.invitation_link = Some(link);
            form.continue_cloud();
            assert!(form.creation_step == CreationStep::Password);
            assert!(!form.validate_creation());
            form.password.push_str("test master password");
            form.repeat.push_str("test master password");
            form.path = Some(dir.path().join("guest.lbx"));
            // Ignore credentials typed before the managed link was pasted.
            form.cloud.base_url = endpoint.into();
            form.cloud.bucket = "previous-bucket".into();
            form.cloud.access_key.push_str("previous-access");
            form.cloud.secret_key.push_str("previous-secret");
            form.submit();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let hardware =
                std::sync::Arc::new(crate::platform::touchid::tests::FakeBiometrics::default());
            let opened = loop {
                if let Some(opened) = form.poll(hardware.clone()) {
                    break opened;
                }
                assert!(form.error.is_none(), "{:?}", form.error);
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert!(opened.vault.awaiting_member_storage());
            use crate::desktop::{session::Screen, test_support::security};
            let mut restart = Screen::Locked(Box::new(crate::access::auth::Locked::new(None)));
            let mut settings = crate::settings::state::Settings {
                config: Default::default(),
                path: None,
            };
            restart.open(opened, &mut settings, hardware.as_ref(), &mut security());
            assert!(
                matches!(restart, Screen::Unlocked(workspace) if workspace.sharing.joining && workspace.page == crate::desktop::workspace::Page::Sharing)
            );
        })
    });
}
