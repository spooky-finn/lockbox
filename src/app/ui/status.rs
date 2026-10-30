use crate::app::context::Session;
use crate::app::{security::Status, session::Screen};
use dioxus::prelude::*;

#[component]
pub(super) fn StatusBar() -> Element {
    let state = use_context::<Session>();
    let status = use_context::<Signal<Status>>();
    let status = status.read();
    let editing =
        matches!(&*state.read(), Screen::Unlocked(workspace) if workspace.draft.is_some());
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
