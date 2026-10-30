use super::{
    editor::DraftView, entry::EntryView, sidebar::Sidebar, status::StatusBar, toolbar::Toolbar,
};
use crate::app::context::{Session, edit_unlocked};
use crate::app::session::Screen;
use dioxus::prelude::*;

#[component]
pub(super) fn UnlockedView() -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let editing = unlocked.draft.is_some();
    rsx! {
        Toolbar {}
        div { class: if unlocked.selected.is_some() || editing { "workspace has-detail" } else { "workspace" },
            Sidebar {}
            section { class: "detail",
                if !editing && unlocked.selected.is_some() {
                    button { class: "mobile-back quiet", onclick: move |_| edit_unlocked(state, |u| u.selected = None), "← К списку" }
                }
                if let Some(error) = &unlocked.error { p { class: "error", role: "alert", "{error}" } }
                if editing { DraftView {} }
                else if unlocked.selected_entry().is_some() { EntryView {} }
                else { div { class: "empty-state", h2 { "Ваши записи" } p { "Выберите запись слева или создайте новую." } } }
            }
        }
        StatusBar {}
    }
}
