use crate::desktop::i18n::Text;
use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        widgets::symbol::{Icon, Symbol},
        workspace::Unlocked,
    },
    records::tree::DisplayNode,
    settings::language::Language,
};
use dioxus::prelude::*;
use std::{collections::HashMap, rc::Rc};

#[derive(PartialEq)]
struct RowState {
    selected: bool,
    expanded: bool,
    editing: bool,
    searching: bool,
    language: Language,
}

struct CachedRow {
    state: RowState,
    view: Element,
}

/// Retain identical VNodes so Dioxus can skip their attributes and descendants.
/// Only display metadata and transient row state live here; no entry secrets.
pub(crate) struct RenderCache {
    nodes: Rc<Vec<DisplayNode>>,
    entries: HashMap<String, CachedRow>,
    folders: HashMap<String, CachedRow>,
    view: Element,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            nodes: Rc::new(Vec::new()),
            entries: HashMap::new(),
            folders: HashMap::new(),
            view: rsx! {},
        }
    }
}

impl RenderCache {
    fn forget(&mut self, nodes: &[DisplayNode]) {
        for node in nodes {
            match node {
                DisplayNode::Record { id, .. } => {
                    self.entries.remove(id);
                }
                DisplayNode::Folder { id, children, .. } => {
                    self.folders.remove(id);
                    self.forget(children);
                }
            }
        }
    }

    pub(crate) fn render(
        &mut self,
        nodes: Rc<Vec<DisplayNode>>,
        workspace: &Unlocked,
        state: Session,
        language: Language,
    ) -> Element {
        let fresh = !Rc::ptr_eq(&self.nodes, &nodes);
        if fresh {
            self.entries.clear();
            self.folders.clear();
            self.nodes = nodes.clone();
        }
        let (view, changed) = self.branch(&nodes, workspace, state, language);
        if fresh || changed {
            self.view = view;
        }
        self.view.clone()
    }

    fn branch(
        &mut self,
        nodes: &[DisplayNode],
        workspace: &Unlocked,
        state: Session,
        language: Language,
    ) -> (Element, bool) {
        let editing = workspace.directories.draft.is_some();
        let searching = !workspace.search.trim().is_empty();
        let mut changed = false;
        let mut views = Vec::with_capacity(nodes.len());
        for node in nodes {
            let (id, folder, selected, expanded) = match node {
                DisplayNode::Record { id, .. } => (
                    id,
                    false,
                    workspace.page == crate::desktop::workspace::Page::Vault
                        && workspace.selected.as_ref() == Some(id),
                    false,
                ),
                DisplayNode::Folder { id, .. } => (
                    id,
                    true,
                    workspace.page == crate::desktop::workspace::Page::Vault
                        && workspace.selected.is_none()
                        && workspace.directories.selected.as_ref() == Some(id),
                    searching || !workspace.directories.collapsed.contains(id),
                ),
            };
            let row = RowState {
                selected,
                expanded,
                editing,
                searching,
                language,
            };
            if !expanded
                && self
                    .folders
                    .get(id)
                    .is_some_and(|cached| cached.state.expanded)
                && let DisplayNode::Folder { children, .. } = node
            {
                // Unmounted components cannot be resurrected from cached VNodes.
                self.forget(children);
            }
            let (children_view, children_changed) = match node {
                DisplayNode::Folder { children, .. } if expanded => {
                    self.branch(children, workspace, state, language)
                }
                _ => (rsx! {}, false),
            };
            let cache = if folder {
                &mut self.folders
            } else {
                &mut self.entries
            };
            if !children_changed
                && let Some(cached) = cache.get(id)
                && cached.state == row
            {
                views.push(cached.view.clone());
                continue;
            }
            changed = true;
            #[cfg(test)]
            ROW_BUILDS.set(ROW_BUILDS.get() + 1);
            let view = match node {
                DisplayNode::Record { id, title } => {
                    let id = id.clone();
                    rsx! {
                        li { class: "tree-leaf", key: "entry-{id}",
                            button { class: if selected { "entry-row selected" } else { "entry-row" },
                                "data-kind": "entry", "data-id": "{id}",
                                r#type: "button", title: "{title}", aria_current: "{selected}", disabled: editing,
                                onclick: move |_| edit_unlocked(state, |u| u.navigate(crate::desktop::workspace::Destination::Entry(id.clone()))),
                                "{title}"
                            }
                        }
                    }
                }
                DisplayNode::Folder { id, name, children } => {
                    let toggle = id.clone();
                    let select = id.clone();
                    rsx! {
                        li { class: "tree-folder", key: "folder-{id}",
                            div { class: if selected { "folder-row selected" } else { "folder-row" },
                                button { class: if expanded { "tree-disclosure expanded" } else { "tree-disclosure" },
                                    r#type: "button", disabled: editing || searching,
                                    title: if expanded { language.text(Text::CollapseFolder) } else { language.text(Text::ExpandFolder) },
                                    aria_label: if expanded { language.format(Text::CollapseFolder2, std::slice::from_ref(name)) } else { language.format(Text::ExpandFolder2, std::slice::from_ref(name)) },
                                    aria_expanded: "{expanded}", aria_controls: "folder-{id}",
                                    onclick: move |_| edit_unlocked(state, |u| u.directories.toggle(toggle.clone())),
                                    Symbol { icon: Icon::ChevronRight }
                                }
                                button { class: "folder-label", r#type: "button", title: "{name}",
                                    "data-kind": "directory", "data-id": "{id}",
                                    disabled: editing, aria_current: "{selected}",
                                    aria_expanded: "{expanded}", aria_controls: "folder-{id}",
                                    onclick: move |_| edit_unlocked(state, |u| u.navigate(crate::desktop::workspace::Destination::ToggleDirectory(select.clone()))),
                                    Symbol { icon: if expanded { Icon::FolderOpen } else { Icon::Folder } }
                                    span { "{name}" }
                                }
                            }
                            if expanded {
                                ul { class: "tree-branch", id: "folder-{id}",
                                    if children.is_empty() { li { class: "tree-empty", {language.text(Text::EmptyFolder)} } }
                                    {children_view}
                                }
                            }
                        }
                    }
                }
            };
            cache.insert(
                id.clone(),
                CachedRow {
                    state: row,
                    view: view.clone(),
                },
            );
            views.push(view);
        }
        (rsx! { for view in views { {view} } }, changed)
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static ROW_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "tests/tree.rs"]
mod tests;
