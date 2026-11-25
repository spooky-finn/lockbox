use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{
            Session, edit_unlocked, edit_workspace_ui, navigate, use_records_version,
            use_unlocked_selector,
        },
        move_records::{MoveSidebar, SidebarMove, TreeDrop, can_move, move_in_vault},
        session::Screen,
        widgets::{
            button::IconButton,
            symbol::{Icon, Symbol},
            vault_icon::VaultIcon,
        },
        workspace::{Destination, Page},
    },
    records::{tree, views::tree::RenderCache},
};
use dioxus::prelude::*;

#[derive(serde::Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
enum TreeAction {
    Entry(String),
    Directory(String),
    Toggle(String),
    Activity,
    /// Escape pressed while the WebView left focus on the page body.
    Close,
}

fn select_tree_row(state: Session, selection: TreeAction) {
    crate::desktop::runtime::record_activity();
    if matches!(selection, TreeAction::Activity) {
        return;
    }
    if !matches!(&*state.peek(), Screen::Unlocked(_)) {
        return;
    }
    edit_workspace_ui(state, |u| {
        let destination = match selection {
            TreeAction::Entry(id) if u.vault.entries().iter().any(|e| e.id == id) => {
                crate::desktop::workspace::Destination::Entry(id)
            }
            TreeAction::Directory(id) if u.vault.folders().iter().any(|f| f.id == id) => {
                crate::desktop::workspace::Destination::Directory(Some(id))
            }
            TreeAction::Close => {
                u.escape();
                return;
            }
            TreeAction::Toggle(id)
                if u.directories.draft.is_none()
                    && u.search.trim().is_empty()
                    && u.vault.folders().iter().any(|f| f.id == id) =>
            {
                u.directories.toggle(id);
                return;
            }
            _ => return,
        };
        u.request_navigation(destination);
    });
}

#[component]
pub(crate) fn Sidebar() -> Element {
    // Keep native pointer capture and layout observation inside the webview.
    let resize = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<document::Eval>)));
    let cleanup = resize.clone();
    let navigation = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<document::Eval>)));
    let navigation_cleanup = navigation.clone();
    let drag = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<document::Eval>)));
    let drag_cleanup = drag.clone();
    use_drop(move || {
        if let Some(eval) = cleanup.take() {
            let _ = eval.send(());
        }
        if let Some(eval) = navigation_cleanup.take() {
            let _ = eval.send(());
        }
        if let Some(eval) = drag_cleanup.take() {
            let _ = eval.send(());
        }
    });
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let settings = use_context::<Signal<crate::settings::state::Settings>>();
    let vault_name = use_memo(move || state.read().vault_name(&settings.read().config));
    let vault_icon = use_unlocked_selector(|u| u.vault.icon().cloned());
    let controls = SidebarMove::use_controls();
    let choosing_records = controls.selection.read().is_some();
    let can_drag = use_unlocked_selector(|u| u.page == Page::Vault && can_move(u));
    let can_drag = can_drag() == Some(true) && !controls.is_busy();
    let summary = use_unlocked_selector(|u| {
        (
            u.vault.entries().len(),
            u.directories.draft.is_some(),
            u.vault.signing_error().is_some(),
            u.vault.has_device_certificate(),
            u.page,
            u.directories.fold_actions(&u.vault),
            !u.search.trim().is_empty(),
        )
    });
    let Some((
        count,
        editing_folder,
        read_only,
        shared,
        page,
        (can_collapse, can_expand),
        searching,
    )) = summary()
    else {
        return rsx! {};
    };
    // Search shows every folder open, so folding has nothing to act on.
    let fold_locked = editing_folder || searching;
    let vault_name = vault_name()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| language.text(Text::Vault).to_owned());

    rsx! {
        aside { id: "vault-sidebar", class: "sidebar",
            "data-tree-can-drag": "{can_drag}",
            onkeydown: move |event| {
                if event.key() == Key::Escape && !event.is_auto_repeating() && controls.dismiss() {
                    event.prevent_default();
                    event.stop_propagation();
                }
            },
            onmounted: move |_| resize.set(Some(document::eval(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sidebar-resize.js"))))),
            div { class: "sidebar-heading",
                if let Some(icon) = vault_icon().flatten() {
                    span { class: "sidebar-vault-icon", aria_hidden: "true",
                        VaultIcon { icon: Some(icon), fallback: Icon::Folder }
                    }
                }
                h2 { title: "{vault_name}", "{vault_name}" }
                span { class: "count", "{count}" }
                IconButton { class: "primary", icon: Icon::Plus, label: language.text(Text::NewEntry),
                    disabled: choosing_records || editing_folder || read_only, onclick: move |_| navigate(state, Destination::NewEntry),
                }
                IconButton { class: "quiet", icon: Icon::FolderPlus, label: language.text(Text::NewFolder),
                    disabled: choosing_records || editing_folder || read_only, onclick: move |_| navigate(state, Destination::NewDirectory),
                }
            }
            div { class: "search-row",
                SearchBox {}
                IconButton { class: "quiet", icon: Icon::CollapseAll, label: language.text(Text::CollapseAllFolders),
                    disabled: fold_locked || !can_collapse,
                    onclick: move |_| edit_unlocked(state, |u| u.directories.collapse_all(&u.vault)),
                }
                IconButton { class: "quiet", icon: Icon::ExpandAll, label: language.text(Text::ExpandAllFolders),
                    disabled: fold_locked || !can_expand,
                    onclick: move |_| edit_unlocked(state, |u| u.directories.expand_all()),
                }
            }
            nav { class: "tree-scroll", aria_label: language.text(Text::EntriesAndFolders),
                oncontextmenu: move |event| controls.context_menu(state, None, event),
                ul { id: "vault-tree", class: "vault-tree",
                    "data-navigate-records": "{page == Page::Vault}",
                    onmounted: move |_| {
                        let mut eval = document::eval(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tree-navigation.js")));
                        navigation.set(Some(eval));
                        spawn(async move {
                            while let Ok(selection) = eval.recv::<TreeAction>().await {
                                if !matches!(selection, TreeAction::Close) || !controls.dismiss() {
                                    select_tree_row(state, selection);
                                }
                            }
                        });
                        let mut eval = document::eval(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tree-drag.js")));
                        drag.set(Some(eval));
                        spawn(async move {
                            while let Ok(action) = eval.recv::<TreeDrop>().await {
                                if !controls.is_busy() {
                                    move_in_vault(state, action);
                                }
                            }
                        });
                    },
                    TreeContent { controls }
                }
            }
            div { class: "vault-root-drop", "data-drop-root": "true", {language.text(Text::VaultRoot)} }
            MoveSidebar { controls }
            div { class: "sidebar-footer",
                IconButton { class: "quiet", icon: Icon::Trash, label: language.text(Text::Trash),
                    disabled: editing_folder, current: (page == Page::Trash).then_some("page"),
                    onclick: move |_| navigate(state, Destination::Page(Page::Trash)),
                }
                if shared {
                    IconButton { class: "quiet", icon: Icon::Users, label: language.text(Text::SharingParticipants),
                        current: (page == Page::Sharing).then_some("page"),
                        onclick: move |_| navigate(state, Destination::Page(Page::Sharing)),
                    }
                }
                IconButton { class: "quiet", icon: Icon::Settings, label: language.text(Text::Settings),
                    current: (page == Page::Settings).then_some("page"),
                    onclick: move |_| navigate(state, Destination::Page(Page::Settings)),
                }
            }
            div { id: "sidebar-resizer", class: "sidebar-resizer", role: "separator", tabindex: "0",
                aria_label: language.text(Text::ResizeSidebar),
                title: language.text(Text::ResizeSidebar),
                aria_orientation: "vertical", aria_controls: "vault-sidebar",
                aria_valuemin: "200", aria_valuemax: "480", aria_valuenow: "224",
            }
        }
    }
}

#[component]
fn SearchBox() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let query = use_unlocked_selector(|u| u.search.clone());
    let query = query.read();
    let Some(query) = &*query else {
        return rsx! {};
    };
    rsx! {
        label { class: "sr-only", r#for: "search", {language.text(Text::SearchEntriesAndFolders)} }
        div { class: "search-field",
            Symbol { icon: Icon::Search }
            input { id: "search", r#type: "search", placeholder: language.text(Text::Search), value: "{query}",
                oninput: move |event| edit_workspace_ui(state, |u| u.search = event.value()),
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/sidebar.rs"]
mod tests;

#[component]
fn TreeContent(controls: SidebarMove) -> Element {
    let state = use_context::<Session>();
    let cache = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(RenderCache::default())));
    let settings = use_context::<Signal<crate::settings::state::Settings>>();
    let version = use_records_version();
    let query = use_memo(move || match &*state.read() {
        Screen::Unlocked(u) => u.search.trim().to_lowercase(),
        Screen::Locked(_) => String::new(),
    });
    let topology = use_memo(move || {
        let version = version.read();
        version.as_ref()?;
        let screen = state.peek();
        let Screen::Unlocked(u) = &*screen else {
            return None;
        };
        Some(tree::SortedTopology::new(&u.vault))
    });
    let nodes = use_memo(move || {
        let _version = version.read();
        let query = query.read();
        let topology = topology.read();
        let Some(topology) = &*topology else {
            return std::rc::Rc::new(Vec::new());
        };
        let screen = state.peek();
        let Screen::Unlocked(u) = &*screen else {
            return std::rc::Rc::new(Vec::new());
        };
        std::rc::Rc::new(
            tree::project_indexed(&u.vault, &query, topology)
                .into_iter()
                .map(tree::DisplayNode::from)
                .collect::<Vec<_>>(),
        )
    });
    let tree_key = use_memo(move || {
        let language = settings.read().config.language;
        let screen = state.read();
        match &*screen {
            Screen::Unlocked(u) => Some((
                u.directories.collapsed.clone(),
                u.selected.clone(),
                u.directories.selected.clone(),
                u.page,
                u.directories.draft.is_some(),
                u.vault.signing_error().is_some(),
                language,
            )),
            _ => None,
        }
    });
    let searching = use_memo(move || !query.read().is_empty());
    let tree_view = use_memo(move || {
        let nodes = nodes.read();
        let _selection = controls.selection.read();
        // Search also changes expansion and empty-state behavior when the
        // resulting hierarchy happens to contain exactly the same rows.
        let _searching = searching.read();
        let key = tree_key.read();
        let Some((_, _, _, _, _, _, language)) = &*key else {
            *cache.borrow_mut() = RenderCache::default();
            return rsx! {};
        };
        let language = *language;
        let screen = state.peek();
        let Screen::Unlocked(workspace) = &*screen else {
            return rsx! {};
        };
        rsx! {
            {cache.borrow_mut().render(nodes.clone(), workspace, state, language, controls)}
            if nodes.is_empty() {
                li { class: "muted empty-list",
                    if workspace.search.trim().is_empty() { {language.text(Text::CreateYourFirstEntryOrFolder)} }
                    else { {language.text(Text::NoEntriesFound)} }
                }
            }
        }
    });
    tree_view()
}
