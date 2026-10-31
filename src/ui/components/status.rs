use crate::app::{security::Status, session::Screen};
use crate::ui::context::Session;
use dioxus::prelude::*;

#[component]
pub(super) fn StatusBar() -> Element {
    let state = use_context::<Session>();
    let status = use_context::<Signal<Status>>();
    let status = status.read();
    if status.lock_seconds.is_none() && status.clipboard_seconds.is_none() {
        return rsx! {};
    }
    let editing = matches!(&*state.read(), Screen::Unlocked(workspace) if workspace.is_editing());
    rsx! {
        footer { class: "status", aria_live: "polite",
            if let Some(seconds) = status.lock_seconds {
                span { class: "notice", "Автоблокировка через {seconds} с"
                    if editing { " — несохранённая правка будет сброшена" }
                }
            }
            if let Some(seconds) = status.clipboard_seconds {
                span { "Скопировано · буфер очистится через {seconds} с" }
            }
        }
    }
}
