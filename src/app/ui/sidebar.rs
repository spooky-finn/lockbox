use super::{
    button::IconButton,
    symbol::{Icon, Symbol},
    tree::nodes_ui,
};
use crate::app::{
    context::{Session, edit_unlocked},
    session::Screen,
    tree,
};
use dioxus::prelude::*;

#[component]
pub(super) fn Sidebar() -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };
    let nodes = tree::project(&workspace.vault, &workspace.search);
    rsx! {
        aside { class: "sidebar",
            div { class: "sidebar-heading",
                h2 { "Хранилище" }
                span { class: "count", "{workspace.vault.entries.len()}" }
                IconButton { class: "quiet", icon: Icon::FolderPlus, label: "Новая папка",
                    disabled: workspace.is_editing(), onclick: move |_| edit_unlocked(state, |u| u.start_directory()),
                }
            }
            label { class: "sr-only", r#for: "search", "Поиск записей и папок" }
            div { class: "search-field",
                Symbol { icon: Icon::Search }
                input { id: "search", r#type: "search", placeholder: "Поиск", value: "{workspace.search}",
                    oninput: move |event| edit_unlocked(state, |u| u.search = event.value()),
                }
            }
            nav { class: "tree-scroll", aria_label: "Записи и папки",
                ul { class: "vault-tree", {nodes_ui(&nodes, workspace, state)} }
                if nodes.is_empty() {
                    p { class: "muted empty-list",
                        if workspace.search.trim().is_empty() { "Создайте первую запись или папку" }
                        else { "Записей не найдено" }
                    }
                }
            }
        }
    }
}
