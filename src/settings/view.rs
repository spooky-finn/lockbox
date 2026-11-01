use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        i18n::use_language,
        runtime,
        session::Screen,
        widgets::symbol::{Icon, Symbol},
        workspace::Page,
    },
    platform::autolock,
    records::editor::replace_secret,
    settings::{appearance::ThemeMode, language::Language, state::Settings},
};
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
fn LanguageSelect(id: &'static str) -> Element {
    let language = use_language();
    rsx! {
        select { id, aria_label: language.text(Text::InterfaceLanguage), value: language.code(),
            onchange: move |event| match event.value().as_str() {
                "en" => runtime::set_language(Language::English),
                "ru" => runtime::set_language(Language::Russian),
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
                        onchange: move |event| runtime::set_website_icons(event.checked()),
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
                            if let Ok(minutes) = event.value().parse() { runtime::set_auto_lock(minutes); }
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
                            onchange: move |event| runtime::set_touch_id(event.checked()),
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
fn AppearanceSection() -> Element {
    let language = use_language();
    let settings = use_context::<Signal<Settings>>();
    let appearance = settings.read().config.appearance;
    let dark = try_use_context::<crate::desktop::widgets::theme::ResolvedTheme>()
        .map(|theme| theme.is_dark())
        .unwrap_or_else(|| crate::desktop::widgets::theme::effective_dark(appearance, false));
    let color = appearance
        .accent
        .map(|color| color.hex())
        .unwrap_or_else(|| if dark { "#409cff" } else { "#007aff" }.into());
    let background = appearance
        .background
        .map(|color| color.hex())
        .unwrap_or_else(|| if dark { "#202124" } else { "#f5f5f7" }.into());
    rsx! {
        section { class: "settings-section stack",
            h2 { {language.text(Text::Appearance)} }
            div { class: "preference-row",
                div { class: "preference-copy",
                    label { r#for: "settings-theme", {language.text(Text::ColorTheme)} }
                    p { class: "muted", {language.text(Text::ColorThemeDescription)} }
                }
                select { id: "settings-theme", value: appearance.theme.code(),
                    onchange: move |event| match event.value().as_str() {
                        "system" => runtime::set_theme(ThemeMode::System),
                        "light" => runtime::set_theme(ThemeMode::Light),
                        "dark" => runtime::set_theme(ThemeMode::Dark),
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
            div { class: "preference-row",
                div { class: "preference-copy",
                    label { r#for: "settings-accent", {language.text(Text::AccentColor)} }
                    p { id: "accent-description", class: "muted", {language.text(Text::AccentColorDescription)} }
                }
                div { class: "color-controls",
                    input { id: "settings-accent", r#type: "color", value: "{color}",
                        aria_describedby: "accent-description",
                        onchange: move |event| runtime::set_accent_color(Some(&event.value())),
                    }
                    button { class: "quiet", r#type: "button", disabled: appearance.accent.is_none(),
                        onclick: move |_| runtime::set_accent_color(None),
                        {language.text(Text::ResetAccentColor)}
                    }
                }
            }
            div { class: "preference-row",
                div { class: "preference-copy",
                    label { r#for: "settings-background", {language.text(Text::BackgroundColor)} }
                    p { id: "background-description", class: "muted", {language.text(Text::BackgroundColorDescription)} }
                }
                div { class: "color-controls",
                    input { id: "settings-background", r#type: "color", value: "{background}",
                        aria_describedby: "background-description",
                        onchange: move |event| runtime::set_background_color(Some(&event.value())),
                    }
                    button { class: "quiet", r#type: "button", disabled: appearance.background.is_none(),
                        onclick: move |_| runtime::set_background_color(None),
                        {language.text(Text::ResetBackgroundColor)}
                    }
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
                    onclick: move |_| crate::records::dialogs::choose_import_file(),
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
