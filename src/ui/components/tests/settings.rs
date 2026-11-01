use crate::{
    app::{session::Screen, settings::Settings, workspace::Page},
    core::language::Language,
    infra::config::Config,
    ui::{
        components::test_support::{mounted, update},
        context::edit_unlocked,
        runtime,
    },
};
use dioxus::prelude::*;

#[test]
fn settings_is_a_page_and_returning_restores_the_selected_entry() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.open_settings())
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("settings-page"));
    assert!(html.contains("Язык интерфейса"));
    assert!(html.contains("Время автоблокировки"));
    assert!(html.contains("workspace has-detail"));
    assert!(!html.contains("entry-detail"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("preferences-panel"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.page = Page::Vault)
    });
    assert!(dioxus_ssr::render(&dom).contains("entry-detail"));
}

#[test]
fn settings_preserves_unsaved_drafts_when_switching_language_and_returning() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.start_new();
            u.draft.as_mut().unwrap().entry.title = "Несохранённая запись {work}".into();
            u.open_settings();
        })
    });
    update(&mut dom, |_| runtime::set_language(Language::English));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Interface language"));
    assert!(html.contains("Auto-lock timeout"));
    assert!(html.contains("Back to vault"));
    assert!(!html.contains("id=\"entry-title\""));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.page = Page::Vault)
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Несохранённая запись {work}"));
    assert!(html.contains("Show password"));
    assert!(html.contains("Save"));
}

#[test]
fn add_entry_is_in_the_sidebar_and_disabled_during_editing() {
    let mut dom = mounted();
    let html = dioxus_ssr::render(&dom);
    let header = html.split("</header>").next().unwrap();
    assert!(!header.contains("Новая запись"));
    let sidebar = html
        .split("<aside")
        .nth(1)
        .unwrap()
        .split("</aside>")
        .next()
        .unwrap();
    assert!(sidebar.contains("Новая запись"));
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_new()));
    let html = dioxus_ssr::render(&dom);
    let sidebar = html
        .split("<aside")
        .nth(1)
        .unwrap()
        .split("</aside>")
        .next()
        .unwrap();
    let add_button = sidebar
        .split("title=\"Новая запись\"")
        .nth(1)
        .unwrap()
        .split("</button>")
        .next()
        .unwrap();
    assert!(add_button.contains("disabled"));
}

#[test]
fn language_and_security_preferences_persist_and_remain_after_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut dom = mounted();
    update(&mut dom, |_| {
        consume_context::<Signal<Settings>>().write().path = Some(path.clone());
        runtime::set_auto_lock(15);
        runtime::set_language(Language::English);
    });
    let saved = Config::load(&path);
    assert_eq!(saved.language, Language::English);
    assert_eq!(saved.auto_lock_minutes, 15);
    update(&mut dom, |_| runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Vault is locked"));
    assert!(html.contains("locked-language"));
    assert!(html.contains("Unlock"));
    update(&mut dom, |_| runtime::set_language(Language::Russian));
    assert!(dioxus_ssr::render(&dom).contains("Хранилище заблокировано"));
    assert_eq!(Config::load(&path).language, Language::Russian);
}

#[test]
fn english_translates_existing_errors_and_creation_without_translating_user_data() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.entries[0].title = "Работа".into();
            u.error = Some("Не удалось сохранить: неверный мастер-пароль".into());
        });
        runtime::set_language(Language::English);
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Работа"));
    assert!(html.contains("Could not save: incorrect master password"));
    assert!(html.contains("Copy password"));
    update(&mut dom, |_| runtime::begin_create());
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Single user"));
    assert!(html.contains("Multiple users"));
    update(&mut dom, |mut state| {
        let mut screen = state.write();
        let Screen::Locked(form) = &mut *screen else {
            panic!("not locked")
        };
        form.storage_kind = Some(crate::app::auth::StorageKind::Cloud);
        form.continue_cloud();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Enter a username"));
    assert!(html.contains("How to set up Storj"));
}
