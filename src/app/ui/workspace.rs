use super::{
    button::IconButton,
    symbol::{Icon, Symbol},
};
use super::{
    directories::DirectoryView, directory_editor::DirectoryEditor, editor::DraftView,
    entry::EntryView, sidebar::Sidebar, status::StatusBar, toolbar::Toolbar,
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
    let editing = unlocked.is_editing();
    let has_detail =
        unlocked.selected.is_some() || unlocked.directories.selected.is_some() || editing;
    rsx! {
        Toolbar {}
        div { class: if has_detail { "workspace has-detail" } else { "workspace" },
            Sidebar {}
            section { class: "detail",
                if !editing && has_detail {
                    IconButton { class: "mobile-back quiet", icon: Icon::ArrowLeft, label: "К списку", onclick: move |_| edit_unlocked(state, |u| u.select_directory(None)) }
                }
                if let Some(error) = &unlocked.error { p { class: "error", role: "alert", "{error}" } }
                if unlocked.directories.draft.is_some() { DirectoryEditor {} }
                else if unlocked.draft.is_some() { DraftView {} }
                else if unlocked.selected_entry().is_some() { EntryView {} }
                else if unlocked.directories.selected.is_some() { DirectoryView {} }
                else { div { class: "empty-state",
                    div { class: "empty-icon", Symbol { icon: Icon::Key } }
                    h2 { "Всё важное — под замком" }
                    p { "Выберите запись слева или создайте новую." }
                    div { class: "actions",
                        IconButton { class: "primary", icon: Icon::Plus, label: "Новая запись", onclick: move |_| edit_unlocked(state, |u| u.start_new()) }
                        IconButton { icon: Icon::FolderPlus, label: "Новая папка", onclick: move |_| edit_unlocked(state, |u| u.start_directory()) }
                    }
                } }
            }
        }
        StatusBar {}
    }
}
