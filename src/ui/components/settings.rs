use crate::ui::i18n::Text;

use super::symbol::{Icon, Symbol};
use crate::{
    app::{session::Screen, settings::Settings, workspace::Page},
    core::language::Language,
    infra::autolock,
    ui::{
        context::{Session, edit_unlocked},
        i18n::use_language,
        runtime,
    },
};
use dioxus::prelude::*;

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
            button { class: "quiet settings-back", r#type: "button",
                onclick: move |_| edit_unlocked(state, |u| u.page = Page::Vault),
                Symbol { icon: Icon::ArrowLeft }
                {language.text(Text::BackToVault)}
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/settings.rs"]
mod tests;
