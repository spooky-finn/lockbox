use crate::ui::i18n::Text;

use super::symbol::{Icon, Symbol};
use crate::{
    app::{editor::replace_secret, session::Screen, settings::Settings, workspace::Page},
    core::language::Language,
    infra::autolock,
    ui::{
        context::{Session, edit_unlocked},
        i18n::use_language,
        runtime,
    },
};
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
pub(super) fn LanguageSelect(id: &'static str) -> Element {
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
pub(super) fn SettingsView() -> Element {
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
            section { class: "settings-section stack",
                h2 { {language.text(Text::Security)} }
                div { class: "preference-row",
                    label { r#for: "auto-lock", {language.text(Text::AutoLock)} }
                    select { id: "auto-lock", aria_label: language.text(Text::AutoLockTimeout),
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
                p { class: "muted", {language.text(Text::LockTheVaultAfterAPeriodOfInactivity)} }
                if unlocked.touch_id.available {
                    label { class: "preference-row", "Touch ID"
                        input { class: "switch", r#type: "checkbox", checked: unlocked.touch_id.enabled,
                            disabled: unlocked.touch_id.pending.is_some(),
                            onchange: move |event| runtime::set_touch_id(event.checked()),
                        }
                    }
                    p { class: "muted", {language.text(Text::UnlockThisVaultWithYourFingerprint)} }
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

/// Copies another vault file into this one; see `app::import`.
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
                    onclick: move |_| runtime::choose_import_file(),
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
#[path = "tests/settings.rs"]
mod tests;
