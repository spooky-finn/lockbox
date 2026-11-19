use crate::{
    desktop::{
        context::{edit_unlocked, set_field},
        view_test_support::{mounted, update},
    },
    records::{
        editor::Field,
        entry::{Totp, TotpAlgorithm},
    },
};

#[test]
fn creation_feedback_is_noninteractive_and_removed_on_lock() {
    let mut dom = mounted();
    assert!(!dioxus_ssr::render(&dom).contains("creation-celebration"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.start_new();
            u.draft.as_mut().unwrap().entry.title = "Fictional new record".into();
            u.autosave_draft().unwrap();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Запись создана"));
    assert!(html.contains("aria-live=\"polite\""));
    assert!(html.contains("prefers-reduced-motion"));
    assert!(!html.contains("private-test-password"));
    update(&mut dom, |_| crate::desktop::runtime::lock(None));
    update(&mut dom, |state| {
        set_field(state, Field::Password, "late-fictional-input".into())
    });
    assert!(!dioxus_ssr::render(&dom).contains("creation-celebration"));
    assert!(!dioxus_ssr::render(&dom).contains("late-fictional-input"));
}

#[test]
fn unlock_spell_is_consumed_when_the_workspace_mounts() {
    use crate::{access::auth::Locked, desktop::session::Screen};
    use dioxus::prelude::{ReadableExt, WritableExt};
    use std::time::Instant;

    let mut dom = mounted();
    let locked = || Screen::Locked(Box::new(Locked::new(None)));
    let mut previous = locked();
    update(&mut dom, |mut state| {
        std::mem::swap(&mut *state.write(), &mut previous);
    });
    let Screen::Unlocked(workspace) = &previous else {
        panic!("fixture starts unlocked");
    };
    workspace.unlock_spell.set(Some(Instant::now()));
    update(&mut dom, |mut state| {
        std::mem::swap(&mut *state.write(), &mut previous);
        assert!(matches!(&*state.peek(), Screen::Unlocked(_)));
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("class=\"unlock-spell\" aria-hidden=\"true\""));

    update(&mut dom, |mut state| {
        std::mem::swap(&mut *state.write(), &mut previous);
    });
    assert!(!dioxus_ssr::render(&dom).contains("class=\"unlock-spell\""));
    update(&mut dom, |mut state| {
        std::mem::swap(&mut *state.write(), &mut previous);
    });
    assert!(!dioxus_ssr::render(&dom).contains("class=\"unlock-spell\""));
}

#[test]
fn an_imported_profile_keeps_records_readable_and_disables_mutating_actions() {
    use crate::desktop::session::Screen;
    use crate::desktop::workspace::Unlocked;
    use crate::vault::KdfParams;
    use crate::vault::ProfileStore;
    use crate::vault::ProviderConfiguration;
    use crate::vault::SharedProfile;
    use crate::vault::witness::MemoryWitness;
    use dioxus::prelude::WritableExt;
    use secrecy::SecretString;
    use std::sync::Arc;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.lbx");
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut vault = store
        .create_shared(
            &path,
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Test user".into(),
                space_name: "Team".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "fixture-bucket".into(),
                    access_key: "access".into(),
                    secret_key: "secret".into(),
                },
            },
        )
        .unwrap();
    let mut entry = crate::records::entry::Entry::new();
    entry.title = "Readable shared entry".into();
    entry.password = "hidden imported fixture".into();
    let id = entry.id.clone();
    vault.test_entries_mut().push(entry);
    vault.save_changes().unwrap();
    let key = vault.export_key();
    drop(vault);
    let imported = ProfileStore::new(Arc::new(MemoryWitness::default()))
        .open_with_key(&path, &key)
        .unwrap();
    let mut dom = mounted();
    update(&mut dom, |mut state| {
        let mut unlocked = Unlocked::new(imported);
        unlocked.selected = Some(id);
        state.set(Screen::Unlocked(Box::new(unlocked)));
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("только для чтения"));
    assert!(html.contains("Readable shared entry"));
    assert!(!html.contains("hidden imported fixture"));
    for label in ["Новая запись", "Изменить", "Удалить"] {
        let button = html
            .split("<button")
            .map(|part| part.split('>').next().unwrap())
            .find(|opening| opening.contains(&format!("aria-label=\"{label}\"")))
            .unwrap();
        assert!(button.contains("disabled"), "read-only action was enabled");
    }
}

#[test]
fn search_reacts_and_failed_save_preserves_the_form() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.search = "missing".into();
            u.start_new();
            u.draft.as_mut().unwrap().entry.totp = Some(Totp {
                secret: "invalid!".into(),
                algorithm: TotpAlgorithm::Sha1,
                digits: 6,
                period: 30,
            });
        })
    });
    update(&mut dom, |state| {
        set_field(state, Field::Title, "Unsaved".into());
        edit_unlocked(state, |u| u.error = u.save_draft().err());
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Записей не найдено"));
    assert!(html.contains("value=\"Unsaved\""));
    assert!(html.contains("TOTP:"));
    assert!(!html.contains("Изменения сохраняются автоматически"));
}

#[test]
fn profile_shows_full_public_fingerprints_and_lock_removes_identity() {
    use crate::desktop::session::Screen;
    use crate::desktop::workspace::Unlocked;
    use crate::sync::identity::hex;
    use crate::vault::KdfParams;
    use crate::vault::Vault;
    use crate::vault::profile::BootstrapProfile;
    use crate::vault::profile::ProviderConfiguration;
    use dioxus::prelude::WritableExt;
    use secrecy::SecretString;
    let dir = tempfile::tempdir().unwrap();
    let fast = KdfParams {
        memory_kib: 8192,
        passes: 1,
        lanes: 1,
    };
    let profile = BootstrapProfile::generate(
        "Alice Profile".into(),
        "Team Profile".into(),
        ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "team-vault".into(),
            access_key: "private-access-key".into(),
            secret_key: "private-secret-key".into(),
        },
        &SecretString::from("recovery password".to_owned()),
        fast,
    )
    .unwrap();
    let fingerprint = hex(&profile.identity.user_id);
    let device = hex(&profile.identity.device_id);
    let vault = Vault::create_profile(
        dir.path().join("profile.lbx"),
        &SecretString::from("master password".to_owned()),
        fast,
        profile,
    )
    .unwrap();
    let mut dom = mounted();
    update(&mut dom, |mut state| {
        state.set(Screen::Unlocked(Box::new(Unlocked::new(vault))))
    });
    let html = dioxus_ssr::render(&dom);
    for value in ["Alice Profile", "Team Profile", &fingerprint, &device] {
        assert!(html.contains(value));
    }
    assert!(html.contains("id=\"space-switch-trigger\""));
    assert!(html.contains("<strong>Team Profile</strong>"));
    assert!(!html.contains("private-access-key"));
    assert!(!html.contains("private-secret-key"));
    update(&mut dom, |_| crate::desktop::runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Team Profile"));
    assert!(!html.contains(&fingerprint));
    assert!(!html.contains(&device));
    assert!(!html.contains("Alice Profile"));
}

#[test]
fn save_shortcuts_work_in_both_layouts_and_ignore_composition_and_repeats() {
    use crate::desktop::{session::Screen, workspace_view::handle_record_shortcut};
    use dioxus::html::SerializedKeyboardData;
    use dioxus::prelude::*;
    let mut dom = mounted();
    for modifier in [Modifiers::CONTROL, Modifiers::META] {
        update(&mut dom, |state| {
            edit_unlocked(state, |u| u.start_edit());
            set_field(state, Field::Title, format!("Saved using {modifier:?}"));
            // Use physical KeyS with a non-Latin key value, as on Russian layouts.
            for (repeat, composing) in [(true, false), (false, true)] {
                let event = Event::new(
                    KeyboardData::new(SerializedKeyboardData::new(
                        Key::Character("ы".into()),
                        Code::KeyS,
                        Location::Standard,
                        repeat,
                        modifier,
                        composing,
                    ))
                    .into(),
                    true,
                );
                handle_record_shortcut(state, event.clone());
                assert!(event.default_action_enabled());
                assert!(matches!(&*state.peek(), Screen::Unlocked(u) if u.draft.is_some()));
            }
            let event = Event::new(
                KeyboardData::new(SerializedKeyboardData::new(
                    Key::Character("ы".into()),
                    Code::KeyS,
                    Location::Standard,
                    false,
                    modifier,
                    false,
                ))
                .into(),
                true,
            );
            handle_record_shortcut(state, event.clone());
            crate::desktop::view_test_support::finish_save(state);
            assert!(!event.default_action_enabled());
            assert!(matches!(&*state.peek(), Screen::Unlocked(u) if u.draft.is_none()));
            let Screen::Unlocked(u) = &*state.peek() else {
                panic!("locked");
            };
            assert_eq!(
                crate::desktop::test_support::reopen(u).entries[0].title,
                format!("Saved using {modifier:?}")
            );
        });
    }
}

#[test]
fn shared_review_reuses_verification_until_journal_changes() {
    use crate::{
        desktop::{session::Screen, sync_review::REVIEW_REPLAYS, workspace::Unlocked},
        vault::{
            KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
        },
    };
    use dioxus::prelude::*;
    use secrecy::SecretString;
    use std::sync::Arc;
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let vault = store
        .create_shared(
            directory.path().join("review.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Fixture".into(),
                space_name: "Review fixture".into(),
                provider: ProviderConfiguration {
                    base_url: "https://example.invalid".into(),
                    bucket: "fixture".into(),
                    access_key: "fictional".into(),
                    secret_key: "fictional".into(),
                },
            },
        )
        .unwrap();
    let mut dom = mounted();
    update(&mut dom, |mut state| {
        state.set(Screen::Unlocked(Box::new(Unlocked::new(vault))))
    });
    let before = REVIEW_REPLAYS.get();
    assert!(before > 0);
    for i in 0..5 {
        update(&mut dom, |state| {
            edit_unlocked(state, |u| u.search = format!("fixture {i}"))
        });
    }
    assert_eq!(
        REVIEW_REPLAYS.get(),
        before,
        "search must not verify/decrypt history again"
    );
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let revision = u.vault.records_revision();
            let receipts = u
                .vault
                .sync_state()
                .unwrap()
                .outbox
                .iter()
                .copied()
                .collect::<Vec<_>>();
            u.vault.acknowledge_sync(&receipts, true).unwrap();
            assert_eq!(u.vault.records_revision(), revision);
        })
    });
    assert!(
        REVIEW_REPLAYS.get() > before,
        "non-record journal changes also invalidate review"
    );
    let before = REVIEW_REPLAYS.get();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let mut entry = crate::records::entry::Entry::new();
            entry.title = "New shared record".into();
            u.vault.upsert_entry(entry).unwrap();
        })
    });
    assert!(
        REVIEW_REPLAYS.get() > before,
        "journal changes must refresh verified review"
    );
    update(&mut dom, |_| crate::desktop::runtime::lock(None));
    assert!(!dioxus_ssr::render(&dom).contains("New shared record"));
}
