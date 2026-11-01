use crate::ui::i18n::Text;

use super::{
    button::IconButton,
    symbol::{Icon, Symbol},
    tree::nodes_ui,
};
use crate::app::{session::Screen, tree};
use crate::ui::context::{Session, edit_unlocked};
use dioxus::prelude::*;

#[component]
pub(super) fn Sidebar() -> Element {
    // Keep native pointer capture and layout observation inside the webview.
    let resize = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<document::Eval>)));
    let cleanup = resize.clone();
    use_drop(move || {
        if let Some(eval) = cleanup.take() {
            let _ = eval.send(());
        }
    });
    let language = crate::ui::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };

    let nodes = tree::project(&workspace.vault, &workspace.search);
    rsx! {
        aside { id: "vault-sidebar", class: "sidebar",
            onmounted: move |_| resize.set(Some(document::eval(include_str!("../../../assets/sidebar-resize.js")))),
            div { class: "sidebar-heading",
                h2 { {language.text(Text::Vault)} }
                span { class: "count", "{workspace.vault.entries.len()}" }
                IconButton { class: "primary", icon: Icon::Plus, label: language.text(Text::NewEntry),
                    disabled: workspace.is_editing() || workspace.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.start_new()),
                }
                IconButton { class: "quiet", icon: Icon::FolderPlus, label: language.text(Text::NewFolder),
                    disabled: workspace.is_editing() || workspace.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.start_directory()),
                }
            }
            label { class: "sr-only", r#for: "search", {language.text(Text::SearchEntriesAndFolders)} }
            div { class: "search-field",
                Symbol { icon: Icon::Search }
                input { id: "search", r#type: "search", placeholder: language.text(Text::Search), value: "{workspace.search}",
                    oninput: move |event| edit_unlocked(state, |u| u.search = event.value()),
                }
            }
            nav { class: "tree-scroll", aria_label: language.text(Text::EntriesAndFolders),
                ul { class: "vault-tree", {nodes_ui(&nodes, workspace, state, language)} }
                if nodes.is_empty() {
                    p { class: "muted empty-list",
                        if workspace.search.trim().is_empty() { {language.text(Text::CreateYourFirstEntryOrFolder)} }
                        else { {language.text(Text::NoEntriesFound)} }
                    }
                }
            }
            div { class: "sidebar-footer",
                button { class: "quiet sidebar-settings", disabled: workspace.is_editing(),
                    aria_current: (workspace.page == crate::app::workspace::Page::Trash).then_some("page"),
                    onclick: move |_| edit_unlocked(state, |u| u.open_records_page(crate::app::workspace::Page::Trash)),
                    Symbol { icon: Icon::Trash } {language.text(Text::Trash)}
                }
                if workspace.vault.device_certificate().is_some() {
                    button { class: "quiet sidebar-settings", r#type: "button",
                        aria_current: (workspace.page == crate::app::workspace::Page::Sharing).then_some("page"),
                        onclick: move |_| edit_unlocked(state, |u| u.open_sharing()),
                        Symbol { icon: Icon::Fingerprint }
                        {language.text(Text::SharingParticipants)}
                    }
                }
                button { class: "quiet sidebar-settings", r#type: "button",
                    aria_current: (workspace.page == crate::app::workspace::Page::Settings).then_some("page"),
                    onclick: move |_| edit_unlocked(state, |u| u.open_settings()),
                    Symbol { icon: Icon::Settings }
                    {language.text(Text::Settings)}
                }
            }
            div { id: "sidebar-resizer", class: "sidebar-resizer", role: "separator", tabindex: "0",
                aria_label: language.text(Text::ResizeSidebar),
                title: language.text(Text::ResizeSidebar),
                aria_orientation: "vertical", aria_controls: "vault-sidebar",
                aria_valuemin: "200", aria_valuemax: "480", aria_valuenow: "240",
            }
        }
    }
}
