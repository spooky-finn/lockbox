use crate::desktop::i18n::Text;
use crate::records::dialogs;
use crate::settings::actions;

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        i18n::use_language,
        session::Screen,
        widgets::{
            symbol::{Icon, Symbol},
            theme::{ResolvedTheme, mode_name, preset_style},
        },
        workspace::Page,
    },
    platform::autolock,
    records::editor::replace_secret,
    settings::{
        appearance::{ThemeMode, ThemePreset},
        language::Language,
        state::Settings,
    },
};
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
fn LanguageSelect(id: &'static str) -> Element {
    let language = use_language();
    rsx! {
        select { id, aria_label: language.text(Text::InterfaceLanguage), value: language.code(),
            onchange: move |event| match event.value().as_str() {
                "en" => actions::set_language(Language::English),
                "ru" => actions::set_language(Language::Russian),
                _ => {},
            },
            option { value: "en", selected: language == Language::English, lang: "en", "English" }
            option { value: "ru", selected: language == Language::Russian, lang: "ru", "Русский" }
        }
    }
}

#[component]
pub(crate) fn SettingsView() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let settings = use_context::<Signal<Settings>>();
    let preferences = settings.read();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    rsx! {
        article { class: "settings-page stack", aria_label: language.text(Text::Settings),
            div { class: "detail-heading",
                div { class: "directory-icon", Symbol { icon: Icon::Settings } }
                div {
                    p { class: "detail-kind", "lockbox" }
                    h1 { {language.text(Text::Settings)} }
                }
            }
            p { class: "muted", {language.text(Text::SettingsAreSavedAutomaticallyAndAppliedImmediately)} }
            section { class: "settings-section stack",
                h2 { {language.text(Text::General)} }
                div { class: "preference-row",
                    label { r#for: "settings-language", {language.text(Text::InterfaceLanguage)} }
                    LanguageSelect { id: "settings-language" }
                }
                if let Some(profile) = unlocked.vault.profile_summary() {
                    UsernameSetting {
                        username: profile.username.to_owned(),
                        disabled: unlocked.saving_draft() || unlocked.vault.signing_error().is_some(),
                    }
                }
            }
            AppearanceSection {}
            section { class: "settings-section stack",
                h2 { {language.text(Text::Security)} }
                div { class: "preference-row",
                    div { class: "preference-copy",
                        label { r#for: "website-icons", {language.text(Text::WebsiteIcons)} }
                        p { id: "website-icons-description", class: "muted", {language.text(Text::WebsiteIconsDescription)} }
                    }
                    input { id: "website-icons", class: "switch", r#type: "checkbox",
                        checked: preferences.config.website_icons, aria_describedby: "website-icons-description",
                        onchange: move |event| actions::set_website_icons(event.checked()),
                    }
                }
                div { class: "preference-row",
                    div { class: "preference-copy",
                        label { r#for: "auto-lock", {language.text(Text::AutoLock)} }
                        p { id: "auto-lock-description", class: "muted", {language.text(Text::LockTheVaultAfterAPeriodOfInactivity)} }
                    }
                    select { id: "auto-lock", aria_label: language.text(Text::AutoLockTimeout),
                        aria_describedby: "auto-lock-description",
                        value: "{preferences.config.auto_lock_minutes}",
                        onchange: move |event| {
                            if let Ok(minutes) = event.value().parse() { actions::set_auto_lock(minutes); }
                        },
                        for minutes in autolock::CHOICES {
                            option { value: "{minutes}", selected: preferences.config.auto_lock_minutes == minutes,
                                {language.minutes_label(minutes)}
                            }
                        }
                    }
                }
                if unlocked.touch_id.available {
                    div { class: "preference-row",
                        div { class: "preference-copy",
                            label { r#for: "settings-touch-id", "Touch ID" }
                            p { id: "touch-id-description", class: "muted", {language.text(Text::UnlockThisVaultWithYourFingerprint)} }
                        }
                        input { id: "settings-touch-id", class: "switch", r#type: "checkbox", checked: unlocked.touch_id.enabled,
                            aria_describedby: "touch-id-description",
                            disabled: unlocked.touch_id.pending.is_some(),
                            onchange: move |event| actions::set_touch_id(event.checked()),
                        }
                    }
                } else {
                    p { class: "muted", {language.text(Text::TouchIDIsUnavailableOnThisDevice)} }
                }
            }
            ImportSection {}
            button { class: "quiet settings-back", r#type: "button",
                onclick: move |_| edit_unlocked(state, |u| u.page = Page::Vault),
                Symbol { icon: Icon::ArrowLeft }
                {language.text(Text::BackToVault)}
            }
        }
    }
}

#[component]
fn UsernameSetting(username: String, disabled: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let mut draft = use_signal(|| None::<String>);
    let name = draft().unwrap_or_else(|| username.clone());
    rsx! {
        form { class: "preference-row",
            onsubmit: move |event: FormEvent| {
                event.prevent_default();
                if !disabled && let Some(name) = draft() {
                    edit_unlocked(state, |workspace| {
                        workspace.error = workspace.vault.set_username(&name).err().map(|error| error.to_string());
                        if workspace.error.is_none() {
                            workspace.sync.saved();
                            draft.set(None);
                        }
                    });
                }
            },
            label { r#for: "settings-username", {language.text(Text::RenameMyName)} }
            input { id: "settings-username", value: "{name}", disabled, autocomplete: "off",
                oninput: move |event| draft.set(Some(event.value())),
            }
            button { r#type: "submit", disabled: disabled || name.trim().is_empty() || name.trim() == username,
                {language.text(Text::Save)}
            }
        }
    }
}

#[component]
fn AppearanceSection() -> Element {
    let language = use_language();
    let settings = use_context::<Signal<Settings>>();
    let appearance = settings.read().config.appearance;
    let dark = try_use_context::<ResolvedTheme>()
        .map_or_else(|| appearance.theme.is_dark(false), ResolvedTheme::is_dark);
    rsx! {
        section { class: "settings-section stack",
            h2 { id: "settings-appearance", {language.text(Text::Appearance)} }
            div { class: "preference-row",
                label { r#for: "settings-theme", {language.text(Text::ColorTheme)} }
                select { id: "settings-theme", value: appearance.theme.code(),
                    onchange: move |event| match event.value().as_str() {
                        "system" => actions::set_theme(ThemeMode::System),
                        "light" => actions::set_theme(ThemeMode::Light),
                        "dark" => actions::set_theme(ThemeMode::Dark),
                        _ => {},
                    },
                    for (theme, label) in [
                        (ThemeMode::System, Text::FollowSystem),
                        (ThemeMode::Light, Text::LightTheme),
                        (ThemeMode::Dark, Text::DarkTheme),
                    ] {
                        option { value: theme.code(), selected: appearance.theme == theme, {language.text(label)} }
                    }
                }
            }
            div { id: "settings-presets", class: "theme-grid", role: "group", aria_labelledby: "settings-appearance",
                for preset in ThemePreset::ALL {
                    ThemeCard { key: "{preset.code()}", preset, dark, selected: appearance.preset == preset }
                }
            }
        }
    }
}

#[component]
fn ThemeCard(preset: ThemePreset, dark: bool, selected: bool) -> Element {
    let language = use_language();
    let name = match preset {
        ThemePreset::Classic => Text::ThemeTypewriter,
        ThemePreset::Paper => Text::ThemeNotebook,
        ThemePreset::Forest => Text::ThemeFieldNotes,
        ThemePreset::Ocean => Text::ThemeBlueprint,
        ThemePreset::Slate => Text::ThemeGraphPaper,
        ThemePreset::Amber => Text::ThemeKraft,
        ThemePreset::Rose => Text::ThemeRedMargin,
        ThemePreset::Teal => Text::ThemeIndexCards,
        ThemePreset::Plum => Text::ThemeInk,
        ThemePreset::Indigo => Text::ThemeTerminal,
    };
    let sample = if preset == ThemePreset::Indigo {
        ">_"
    } else {
        "Aa"
    };
    rsx! {
        button {
            class: "theme-card",
            r#type: "button",
            aria_pressed: selected,
            "data-theme-choice": preset.code(),
            onclick: move |_| actions::set_theme_preset(preset),
            span {
                class: "theme-preview",
                "data-preset": preset.code(),
                "data-theme": mode_name(dark),
                style: preset_style(preset, dark),
                aria_hidden: true,
                span { class: "theme-sample", "{sample}" }
            }
            span { class: "theme-card-caption",
                span { {language.text(name)} }
                if selected {
                    span { class: "theme-card-check", aria_hidden: true, Symbol { icon: Icon::Check } }
                }
            }
        }
    }
}

/// Copies another vault file into this one; see `records::import`.
#[component]
fn ImportSection() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let import = &unlocked.import;
    let busy = import.busy();
    let ready = !busy && import.path.is_some() && !import.password.is_empty();
    let file_name = import.file_name();
    rsx! {
        section { class: "settings-section stack", aria_label: language.text(Text::ImportVault),
            h2 { {language.text(Text::ImportVault)} }
            p { class: "muted", {language.text(Text::ImportDescription)} }
            div { class: "preference-row",
                span { class: "import-file",
                    match &file_name {
                        Some(name) => rsx! { "{name}" },
                        None => rsx! { {language.text(Text::NoFileChosen)} },
                    }
                }
                button { r#type: "button", disabled: busy,
                    onclick: move |_| dialogs::choose_import_file(),
                    {language.text(Text::ChooseVaultFile)}
                }
            }
            if file_name.is_some() {
                form { class: "stack",
                    onsubmit: move |event: FormEvent| {
                        event.prevent_default();
                        edit_unlocked(state, |u| u.start_import());
                    },
                    label { class: "sr-only", r#for: "import-password", {language.text(Text::ImportPassword)} }
                    input { id: "import-password", r#type: "password", autocomplete: "off",
                        placeholder: language.text(Text::ImportPassword),
                        disabled: busy, value: import.password.as_str(),
                        oninput: move |event| {
                            let value = Zeroizing::new(event.value());
                            edit_unlocked(state, |u| replace_secret(&mut u.import.password, &value));
                        },
                    }
                    button { class: "primary", r#type: "submit", disabled: !ready,
                        if busy { {language.text(Text::ImportBusy)} } else { {language.text(Text::ImportStart)} }
                    }
                }
            }
            if busy && unlocked.is_editing() {
                p { class: "notice", {language.text(Text::ImportAfterEditing)} }
            }
            if let Some(notice) = &import.notice {
                p { class: "notice", role: "status", {language.message(notice)} }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
