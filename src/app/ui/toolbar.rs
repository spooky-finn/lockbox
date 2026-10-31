use super::button::IconButton;
use super::symbol::{Icon, Symbol};
use crate::app::context::{Session, edit_unlocked};
use crate::{
    app::{
        runtime,
        session::Screen,
        settings::{Settings, minutes_label},
    },
    autolock,
};
use dioxus::prelude::*;

#[component]
pub(super) fn Toolbar() -> Element {
    let state = use_context::<Session>();
    let settings = use_context::<Signal<Settings>>();
    let preferences = settings.read();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let editing = unlocked.is_editing();
    let filename = unlocked
        .vault
        .path()
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    rsx! {
        header { class: "toolbar",
            div { class: "brand",
                div { class: "brand-icon", Symbol { icon: Icon::Key } }
                div { strong { "lockbox" } span { class: "vault-name", "{filename}" } }
            }
            div { class: "actions",
                details { class: "preferences",
                    summary { class: "icon-button", title: "Настройки хранилища", aria_label: "Настройки хранилища",
                        Symbol { icon: Icon::Settings }
                    }
                    div { class: "preferences-panel stack",
                        h2 { "Настройки хранилища" }
                        label { class: "preference-row", "Автоблокировка"
                            select { aria_label: "Время автоблокировки", value: "{preferences.config.auto_lock_minutes}",
                                onchange: move |event| { if let Ok(minutes) = event.value().parse() { runtime::set_auto_lock(minutes); } },
                                for minutes in autolock::CHOICES {
                                    option { value: "{minutes}", "{minutes_label(minutes)}" }
                                }
                            }
                        }
                        if unlocked.touch_id.available {
                            label { class: "preference-row", "Touch ID"
                                input { class: "switch", r#type: "checkbox", checked: unlocked.touch_id.enabled,
                                    disabled: unlocked.touch_id.pending.is_some(),
                                    onchange: move |event| runtime::set_touch_id(event.checked()),
                                }
                            }
                        }
                    }
                }
                IconButton { class: "primary", icon: Icon::Plus, label: "Новая запись",
                    disabled: editing, onclick: move |_| edit_unlocked(state, |u| u.start_new()),
                }
                IconButton { icon: Icon::Lock, label: "Заблокировать", onclick: move |_| runtime::lock(None),
                }
            }
        }
    }
}
