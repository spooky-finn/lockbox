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
    let editing = unlocked.draft.is_some();
    let filename = unlocked
        .vault
        .path()
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    rsx! {
        header { class: "toolbar",
            div { class: "brand", strong { "lockbox" } span { class: "muted", "{filename}" } }
            div { class: "actions",
                label { class: "inline", "Автоблок:"
                    select { aria_label: "Время автоблокировки", value: "{preferences.config.auto_lock_minutes}",
                        onchange: move |event| { if let Ok(minutes) = event.value().parse() { runtime::set_auto_lock(minutes); } },
                        for minutes in autolock::CHOICES {
                            option { value: "{minutes}", "{minutes_label(minutes)}" }
                        }
                    }
                }
                if unlocked.touch_id.available {
                    label { class: "inline",
                        input { r#type: "checkbox", checked: unlocked.touch_id.enabled,
                            disabled: unlocked.touch_id.pending.is_some(),
                            onchange: move |event| runtime::set_touch_id(event.checked()),
                        }
                        "Touch ID"
                    }
                }
                button { class: "primary", disabled: editing, onclick: move |_| edit_unlocked(state, |u| u.start_new()), "Новая запись" }
                button { onclick: move |_| runtime::lock(None), "Заблокировать" }
            }
        }
    }
}
