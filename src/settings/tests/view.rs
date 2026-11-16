use crate::{
    desktop::{
        context::edit_unlocked,
        runtime,
        session::Screen,
        view_test_support::{mounted, update},
        workspace::Page,
    },
    settings::{
        appearance::{RgbColor, ThemeMode},
        config::Config,
        language::Language,
        state::Settings,
    },
};
use dioxus::prelude::*;

#[test]
fn settings_is_a_page_and_returning_restores_the_selected_entry() {
    let mut dom = mounted();
    assert!(!dioxus_ssr::render(&dom).contains("settings-language"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.open_settings())
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("settings-page"));
    assert!(!html.contains("entry-detail"));
    assert!(!html.contains("private-test-password"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.page = Page::Vault)
    });
    assert!(dioxus_ssr::render(&dom).contains("entry-detail"));
}

#[test]
fn website_icon_downloads_default_off_and_disabling_discards_session_cache() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.open_settings())
    });
    update(&mut dom, |state| {
        let settings = consume_context::<Signal<Settings>>();
        assert!(!settings.read().config.website_icons);
        let previous = match &*state.read() {
            Screen::Unlocked(u) => u.favicon_cache.clone(),
            _ => panic!(),
        };
        runtime::set_website_icons(true);
        assert!(settings.read().config.website_icons);
        runtime::set_website_icons(false);
        assert!(!settings.read().config.website_icons);
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!()
        };
        assert!(previous != u.favicon_cache);
    });
}

#[test]
fn settings_preserves_unsaved_drafts_when_switching_preferences_and_returning() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.start_new();
            u.draft.as_mut().unwrap().entry.title = "Несохранённая запись {work}".into();
            u.open_settings();
        })
    });
    update(&mut dom, |_| {
        runtime::set_language(Language::English);
        runtime::set_theme(ThemeMode::Dark);
        runtime::set_accent_color(Some("#af52de"));
        runtime::set_background_color(Some("#f3e8d2"));
    });
    let html = dioxus_ssr::render(&dom);
    assert!(!html.contains("id=\"entry-title\""));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.page = Page::Vault)
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Несохранённая запись {work}"));
}

#[test]
fn add_entry_is_disabled_during_editing() {
    let mut dom = mounted();
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_new()));
    let html = dioxus_ssr::render(&dom);
    let add_button = html
        .split("<button")
        .map(|part| part.split('>').next().unwrap())
        .find(|opening| opening.contains("title=\"Новая запись\""))
        .unwrap();
    assert!(add_button.contains("disabled"));
}

#[test]
fn appearance_language_and_security_preferences_persist_and_remain_after_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut dom = mounted();
    update(&mut dom, |_| {
        consume_context::<Signal<Settings>>().write().path = Some(path.clone());
        runtime::set_auto_lock(15);
        runtime::set_language(Language::English);
        runtime::set_theme(ThemeMode::Dark);
        runtime::set_accent_color(Some("#AF52DE"));
        runtime::set_background_color(Some("#F3E8D2"));
    });
    let saved = Config::load(&path);
    assert_eq!(saved.language, Language::English);
    assert_eq!(saved.auto_lock_minutes, 15);
    assert_eq!(saved.appearance.theme, ThemeMode::Dark);
    assert_eq!(saved.appearance.accent, RgbColor::parse("#af52de"));
    assert_eq!(saved.appearance.background, RgbColor::parse("#f3e8d2"));
    update(&mut dom, |_| runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Vault is locked"));
    assert!(!html.contains("locked-language"));
    assert!(!html.contains("Interface language"));
    assert!(html.contains("Unlock"));
    update(&mut dom, |_| runtime::set_language(Language::Russian));
    assert!(dioxus_ssr::render(&dom).contains("Хранилище заблокировано"));
    assert_eq!(Config::load(&path).language, Language::Russian);
    assert_eq!(Config::load(&path).appearance, saved.appearance);
}

#[test]
fn invalid_accent_is_ignored_and_reset_preserves_other_preferences() {
    let mut dom = mounted();
    update(&mut dom, |_| {
        runtime::set_theme(ThemeMode::Dark);
        runtime::set_accent_color(Some("#af52de"));
        runtime::set_accent_color(Some("#af52de;--canvas:red"));
    });
    dom.in_scope(ScopeId::APP, || {
        let settings = consume_context::<Signal<Settings>>();
        assert_eq!(
            settings.read().config.appearance.accent,
            RgbColor::parse("#af52de")
        );
    });
    update(&mut dom, |state| {
        runtime::set_accent_color(None);
        edit_unlocked(state, |u| u.open_settings());
    });
    dom.in_scope(ScopeId::APP, || {
        let settings = consume_context::<Signal<Settings>>();
        assert_eq!(settings.read().config.appearance.theme, ThemeMode::Dark);
        assert_eq!(settings.read().config.appearance.accent, None);
        assert_eq!(settings.read().config.language, Language::Russian);
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("settings-preset"));
    assert!(!html.contains("type=\"color\""));
}

#[test]
fn invalid_background_is_ignored_and_reset_keeps_accent_and_mode() {
    let mut dom = mounted();
    update(&mut dom, |_| {
        runtime::set_theme(ThemeMode::System);
        runtime::set_accent_color(Some("#af52de"));
        runtime::set_background_color(Some("#182432"));
        runtime::set_background_color(Some("#182432;--ink:red"));
    });
    dom.in_scope(ScopeId::APP, || {
        assert_eq!(
            consume_context::<Signal<Settings>>()
                .read()
                .config
                .appearance
                .background,
            RgbColor::parse("#182432")
        );
    });
    update(&mut dom, |_| runtime::set_background_color(None));
    dom.in_scope(ScopeId::APP, || {
        let settings = consume_context::<Signal<Settings>>();
        let appearance = settings.read().config.appearance;
        assert_eq!(appearance.background, None);
        assert_eq!(appearance.accent, RgbColor::parse("#af52de"));
        assert_eq!(appearance.theme, ThemeMode::System);
    });
}

#[test]
fn appearance_controls_handle_theme_preset_and_gradient_events() {
    use dioxus::{
        dioxus_core::Mutation,
        html::{EventData, SerializedFormData, SerializedHtmlEventConverter},
    };

    fn fixture() -> Element {
        let settings = use_signal(|| Settings {
            config: Config::default(),
            path: None,
        });
        let state = use_signal(|| Screen::Locked(Box::new(crate::access::auth::Locked::new(None))));
        use_context_provider(|| settings);
        use_context_provider(|| state);
        rsx! { super::AppearanceSection {} }
    }

    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = VirtualDom::new(fixture);
    let changes = dom.rebuild_to_vec();
    let controls = changes
        .edits
        .iter()
        .filter_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "change" => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let resets = changes
        .edits
        .iter()
        .filter_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(controls.len(), 3);
    assert!(resets.is_empty());
    for mode in ["dark", "light", "system"] {
        dom.runtime().handle_event(
            "change",
            Event::new(
                EventData::Form(SerializedFormData::new(mode.into(), vec![])).into_any(),
                true,
            ),
            controls[0],
        );
        dom.render_immediate_to_vec();
        dom.in_scope(ScopeId::APP, || {
            assert_eq!(
                consume_context::<Signal<Settings>>()
                    .read()
                    .config
                    .appearance
                    .theme
                    .code(),
                mode
            );
        });
    }
    dom.runtime().handle_event(
        "change",
        Event::new(
            EventData::Form(SerializedFormData::new("ocean".into(), vec![])).into_any(),
            true,
        ),
        controls[1],
    );
    dom.render_immediate_to_vec();
    dom.in_scope(ScopeId::APP, || {
        assert_eq!(
            consume_context::<Signal<Settings>>()
                .read()
                .config
                .appearance
                .preset
                .code(),
            "ocean"
        );
    });
    dom.runtime().handle_event(
        "change",
        Event::new(
            EventData::Form(SerializedFormData::new("true".into(), vec![])).into_any(),
            true,
        ),
        controls[2],
    );
    dom.render_immediate_to_vec();
    dom.in_scope(ScopeId::APP, || {
        let settings = consume_context::<Signal<Settings>>();
        assert!(settings.read().config.appearance.gradient);
        assert_eq!(settings.read().config.appearance.background, None);
        assert_eq!(settings.read().config.appearance.accent, None);
    });
}

#[test]
fn changing_language_preserves_user_data_and_existing_errors() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.test_entries_mut()[0].title = "Работа".into();
            u.error = Some("Could not save: incorrect master password".into());
        });
        runtime::set_language(Language::English);
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Работа"));
    assert!(html.contains("Could not save: incorrect master password"));
    assert!(html.contains("Copy password"));
}
