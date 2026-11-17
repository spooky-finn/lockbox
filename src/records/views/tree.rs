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
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
};

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
    children: Option<Element>,
}

#[derive(PartialEq)]
struct Navigation {
    entry: Option<String>,
    folder: Option<String>,
    collapsed: BTreeSet<String>,
    editing: bool,
    searching: bool,
    language: Language,
}

#[derive(Default)]
struct DirtyRows {
    entries: HashSet<String>,
    folders: HashSet<String>,
}

/// Retain identical VNodes so Dioxus can skip their attributes and descendants.
/// Only display metadata and transient row state live here; no entry secrets.
pub(crate) struct RenderCache {
    nodes: Rc<Vec<DisplayNode>>,
    entries: HashMap<String, CachedRow>,
    folders: HashMap<String, CachedRow>,
    view: Element,
    navigation: Option<Navigation>,
    parents: HashMap<String, String>,
    entry_parents: HashMap<String, String>,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            nodes: Rc::new(Vec::new()),
            entries: HashMap::new(),
            folders: HashMap::new(),
            view: rsx! {},
            navigation: None,
            parents: HashMap::new(),
            entry_parents: HashMap::new(),
        }
    }
}

impl RenderCache {
    fn index_parents(&mut self, nodes: &[DisplayNode], parent: Option<&str>) {
        for node in nodes {
            match node {
                DisplayNode::Record { id, .. } => {
                    if let Some(parent) = parent {
                        self.entry_parents.insert(id.clone(), parent.into());
                    }
                }
                DisplayNode::Folder { id, children, .. } => {
                    if let Some(parent) = parent {
                        self.parents.insert(id.clone(), parent.into());
                    }
                    self.index_parents(children, Some(id));
                }
            }
        }
    }

    fn mark_folder(&self, id: &str, dirty: &mut DirtyRows) {
        let mut current = Some(id);
        while let Some(id) = current {
            if !dirty.folders.insert(id.to_owned()) {
                break;
            }
            current = self.parents.get(id).map(String::as_str);
        }
    }

    fn mark_selection(&self, navigation: &Navigation, dirty: &mut DirtyRows) {
        if let Some(id) = &navigation.entry {
            dirty.entries.insert(id.clone());
            if let Some(parent) = self.entry_parents.get(id) {
                self.mark_folder(parent, dirty);
            }
        }
        if let Some(id) = &navigation.folder {
            self.mark_folder(id, dirty);
        }
    }

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
        #[cfg(test)]
        let started = std::time::Instant::now();
        let fresh = !Rc::ptr_eq(&self.nodes, &nodes);
        if fresh {
            self.entries.clear();
            self.folders.clear();
            self.nodes = nodes.clone();
            self.parents.clear();
            self.entry_parents.clear();
            self.index_parents(&nodes, None);
        }
        let in_vault = workspace.page == crate::desktop::workspace::Page::Vault;
        let navigation = Navigation {
            entry: in_vault.then(|| workspace.selected.clone()).flatten(),
            folder: (in_vault && workspace.selected.is_none())
                .then(|| workspace.directories.selected.clone())
                .flatten(),
            collapsed: workspace.directories.collapsed.clone(),
            editing: workspace.directories.draft.is_some(),
            searching: !workspace.search.trim().is_empty(),
            language,
        };
        let mut dirty = DirtyRows::default();
        let targeted = !fresh
            && self.navigation.as_ref().is_some_and(|previous| {
                previous.editing == navigation.editing
                    && previous.searching == navigation.searching
                    && previous.language == navigation.language
            });
        if targeted {
            let previous = self.navigation.as_ref().unwrap();
            self.mark_selection(previous, &mut dirty);
            self.mark_selection(&navigation, &mut dirty);
            for id in previous
                .collapsed
                .symmetric_difference(&navigation.collapsed)
            {
                self.mark_folder(id, &mut dirty);
            }
        }
        let (view, changed) = self.branch(
            &nodes,
            workspace,
            state,
            language,
            targeted.then_some(&dirty),
        );
        self.navigation = Some(navigation);
        if fresh || changed {
            self.view = view.unwrap_or_else(|| rsx! {});
        }
        #[cfg(test)]
        CACHE_MICROS.set(CACHE_MICROS.get() + started.elapsed().as_micros() as usize);
        self.view.clone()
    }

    fn branch(
        &mut self,
        nodes: &[DisplayNode],
        workspace: &Unlocked,
        state: Session,
        language: Language,
        dirty: Option<&DirtyRows>,
    ) -> (Option<Element>, bool) {
        let editing = workspace.directories.draft.is_some();
        let searching = !workspace.search.trim().is_empty();
        let mut changed = false;
        let mut views = Vec::with_capacity(nodes.len());
        for node in nodes {
            let cached = match node {
                DisplayNode::Record { id, .. }
                    if dirty.is_some_and(|d| !d.entries.contains(id)) =>
                {
                    self.entries.get(id)
                }
                DisplayNode::Folder { id, .. }
                    if dirty.is_some_and(|d| !d.folders.contains(id)) =>
                {
                    self.folders.get(id)
                }
                _ => None,
            };
            if let Some(cached) = cached {
                views.push(cached.view.clone());
                continue;
            }
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
                    self.branch(children, workspace, state, language, dirty)
                }
                _ => (None, false),
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
            let children_view = children_view.or_else(|| {
                if expanded {
                    cache.get(id).and_then(|cached| cached.children.clone())
                } else {
                    None
                }
            });
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
                                    {children_view.clone()}
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
                    children: children_view,
                },
            );
            views.push(view);
        }
        (
            changed.then(|| rsx! { for view in views { {view} } }),
            changed,
        )
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static CACHE_MICROS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static ROW_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "tests/tree.rs"]
mod tests;
