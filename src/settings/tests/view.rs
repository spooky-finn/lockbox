use crate::settings::actions;
use crate::{
    desktop::{
        context::edit_unlocked,
        runtime,
        session::Screen,
        view_test_support::{mounted, update},
        workspace::Page,
    },
    settings::{
        appearance::{ThemeMode, ThemePreset},
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
    assert!(html.contains("id=\"settings-vault-appearance\""));
    assert!(html.contains("id=\"settings-vault-icon\""));
    assert!(html.contains("id=\"settings-upload-vault-image\""));
    assert!(html.contains("id=\"settings-vault-filename\""));
    assert!(html.contains("id=\"settings-personal-vault-name\""));
    assert!(!html.contains("id=\"settings-shared-vault-name\""));
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
        actions::set_website_icons(true);
        assert!(settings.read().config.website_icons);
        actions::set_website_icons(false);
        assert!(!settings.read().config.website_icons);
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!()
        };
        assert!(previous != u.favicon_cache);
    });
}

#[test]
fn settings_saves_pending_edits_before_switching_preferences_and_returning() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.start_new();
            u.draft.as_mut().unwrap().entry.title = "Несохранённая запись {work}".into();
            u.navigate(crate::desktop::workspace::Destination::Page(Page::Settings));
        })
    });
    update(&mut dom, |_| {
        actions::set_language(Language::English);
        actions::set_theme(ThemeMode::Dark);
        actions::set_theme_preset(ThemePreset::Forest);
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
fn add_entry_stays_available_during_record_editing() {
    let mut dom = mounted();
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_new()));
    let html = dioxus_ssr::render(&dom);
    let add_button = html
        .split("<button")
        .map(|part| part.split('>').next().unwrap())
        .find(|opening| opening.contains("title=\"Новая запись\""))
        .unwrap();
    assert!(!add_button.contains("disabled"));
}

#[test]
fn appearance_language_and_security_preferences_persist_and_remain_after_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut dom = mounted();
    let id = dom.in_scope(ScopeId::APP, || {
        consume_context::<crate::desktop::context::Session>()
            .peek()
            .vault_id()
            .unwrap()
    });
    update(&mut dom, |_| {
        consume_context::<Signal<Settings>>().write().path = Some(path.clone());
        actions::set_auto_lock(15);
        actions::set_language(Language::English);
        actions::set_custom_appearance(true);
        actions::set_theme(ThemeMode::Dark);
        actions::set_theme_preset(ThemePreset::Paper);
    });
    let saved = Config::load(&path);
    assert_eq!(saved.language, Language::English);
    assert_eq!(saved.auto_lock_minutes, 15);
    assert_eq!(saved.appearance, Default::default());
    let appearance = saved.appearance_for(Some(&id));
    assert_eq!(appearance.theme, ThemeMode::Dark);
    assert_eq!(appearance.preset, ThemePreset::Paper);
    update(&mut dom, |_| runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Master password"));
    assert!(!html.contains("id=\"space-switch-trigger\""));
    assert!(!html.contains("locked-language"));
    assert!(!html.contains("Interface language"));
    assert!(html.contains("Unlock"));
    update(&mut dom, |_| actions::set_language(Language::Russian));
    assert!(dioxus_ssr::render(&dom).contains("Мастер-пароль"));
    assert_eq!(Config::load(&path).language, Language::Russian);
    assert_eq!(Config::load(&path).appearance_for(Some(&id)), appearance);
}

#[test]
fn appearance_controls_apply_each_preset_and_update_its_preview() {
    use dioxus::{
        dioxus_core::Mutation,
        html::{EventData, SerializedFormData, SerializedHtmlEventConverter, SerializedMouseData},
    };
    use kuchikiki::traits::TendrilSink;

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
    let theme_control = changes
        .edits
        .iter()
        .filter_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "change" => Some(*id),
            _ => None,
        })
        .next()
        .unwrap();
    let preset_controls = changes
        .edits
        .iter()
        .filter_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let change = |dom: &mut VirtualDom, control, value: &str| {
        dom.runtime().handle_event(
            "change",
            Event::new(
                EventData::Form(SerializedFormData::new(value.to_owned(), vec![])).into_any(),
                true,
            ),
            control,
        );
        dom.render_immediate_to_vec();
    };
    let click = |dom: &mut VirtualDom, control| {
        dom.runtime().handle_event(
            "click",
            Event::new(
                EventData::Mouse(SerializedMouseData::default()).into_any(),
                true,
            ),
            control,
        );
        dom.render_immediate_to_vec();
    };
    let appearance = |dom: &VirtualDom| {
        dom.in_scope(ScopeId::APP, || {
            consume_context::<Signal<Settings>>()
                .read()
                .config
                .appearance
        })
    };
    let render = |dom: &VirtualDom| {
        kuchikiki::parse_html()
            .one(dioxus_ssr::render(dom))
            .document_node
    };
    let document = render(&dom);
    let preset_codes = document
        .select("#settings-presets button")
        .unwrap()
        .map(|button| {
            button
                .attributes
                .borrow()
                .get("data-theme-choice")
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(preset_codes, ThemePreset::ALL.map(ThemePreset::code));
    assert!(document.select_first("input[type=color]").is_err());
    assert_eq!(preset_controls.len(), ThemePreset::ALL.len());

    for mode in ["light", "system", "dark"] {
        change(&mut dom, theme_control, mode);
        assert_eq!(appearance(&dom).theme.code(), mode);
    }
    for (preset, control) in ThemePreset::ALL.into_iter().zip(preset_controls) {
        click(&mut dom, control);
        assert_eq!(appearance(&dom).preset, preset);
        assert_eq!(appearance(&dom).theme, ThemeMode::Dark);
        let document = render(&dom);
        let pressed = document
            .select("#settings-presets button[aria-pressed=true]")
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(pressed.len(), 1);
        assert_eq!(
            pressed[0].attributes.borrow().get("data-theme-choice"),
            Some(preset.code())
        );
        assert_eq!(
            document
                .select(".theme-preview[data-theme=dark]")
                .unwrap()
                .count(),
            10
        );
    }
}

#[test]
fn changing_language_preserves_user_data_and_existing_errors() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.test_entries_mut()[0].title = "Работа".into();
            u.error = Some("Could not save: incorrect master password".into());
        });
        actions::set_language(Language::English);
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Работа"));
    assert!(html.contains("Could not save: incorrect master password"));
    assert!(html.contains("Copy password"));
}
