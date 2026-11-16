use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        session::Screen,
        widgets::{
            button::IconButton,
            symbol::{Icon, Symbol},
        },
    },
    records::{tree, views::tree::nodes_ui},
};
use dioxus::prelude::*;

#[component]
pub(crate) fn Sidebar() -> Element {
    // Keep native pointer capture and layout observation inside the webview.
    let resize = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<document::Eval>)));
    let cleanup = resize.clone();
    let navigation = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<document::Eval>)));
    let navigation_cleanup = navigation.clone();
    use_drop(move || {
        if let Some(eval) = cleanup.take() {
            let _ = eval.send(());
        }
        if let Some(eval) = navigation_cleanup.take() {
            let _ = eval.send(());
        }
    });
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let settings = use_context::<Signal<crate::settings::state::Settings>>();
    // Dioxus compares this metadata before rebuilding/sorting the tree. A draft
    // keystroke or sync progress notification leaves the cached VNode untouched.
    let tree_key = use_memo(move || {
        let language = settings.read().config.language;
        let screen = state.read();
        match &*screen {
            Screen::Unlocked(u) => Some((
                tree::PresentationKey::new(&u.vault, &u.search),
                u.directories.collapsed.clone(),
                u.selected.clone(),
                u.directories.selected.clone(),
                u.page,
                u.is_editing(),
                language,
            )),
            _ => None,
        }
    });
    let tree_view = use_memo(move || {
        let key = tree_key.read();
        let Some((_, _, _, _, _, _, language)) = &*key else {
            return rsx! {};
        };
        let language = *language;
        let screen = state.peek();
        let Screen::Unlocked(workspace) = &*screen else {
            return rsx! {};
        };
        let nodes = tree::project(&workspace.vault, &workspace.search);
        rsx! {
            {nodes_ui(&nodes, workspace, state, language)}
            if nodes.is_empty() {
                li { class: "muted empty-list",
                    if workspace.search.trim().is_empty() { {language.text(Text::CreateYourFirstEntryOrFolder)} }
                    else { {language.text(Text::NoEntriesFound)} }
                }
            }
        }
    });
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };

    rsx! {
        aside { id: "vault-sidebar", class: "sidebar",
            onmounted: move |_| resize.set(Some(document::eval(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sidebar-resize.js"))))),
            div { class: "sidebar-heading",
                h2 { {language.text(Text::Vault)} }
                span { class: "count", "{workspace.vault.entries().len()}" }
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
                ul { id: "vault-tree", class: "vault-tree",
                    onmounted: move |_| navigation.set(Some(document::eval(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tree-navigation.js"))))),
                    {tree_view()}
                }
            }
            div { class: "sidebar-footer",
                button { class: "quiet sidebar-settings", disabled: workspace.is_editing(),
                    aria_current: (workspace.page == crate::desktop::workspace::Page::Trash).then_some("page"),
                    onclick: move |_| edit_unlocked(state, |u| u.open_records_page(crate::desktop::workspace::Page::Trash)),
                    Symbol { icon: Icon::Trash } {language.text(Text::Trash)}
                }
                if workspace.vault.device_certificate().is_some() {
                    button { class: "quiet sidebar-settings", r#type: "button",
                        aria_current: (workspace.page == crate::desktop::workspace::Page::Sharing).then_some("page"),
                        onclick: move |_| edit_unlocked(state, |u| u.open_sharing()),
                        Symbol { icon: Icon::Fingerprint }
                        {language.text(Text::SharingParticipants)}
                    }
                }
                button { class: "quiet sidebar-settings", r#type: "button",
                    aria_current: (workspace.page == crate::desktop::workspace::Page::Settings).then_some("page"),
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
