use crate::desktop::i18n::Text;
use crate::{
    desktop::{
        context::{Session, edit_unlocked, navigate},
        move_records::{MoveItem, SidebarMove},
        widgets::{
            button::IconButton,
            icons::EntryIcon,
            symbol::{Icon, Symbol},
        },
        workspace::{Destination, Page, Unlocked},
    },
    records::tree::DisplayNode,
    settings::language::Language,
    vault::move_records::Selection,
};
use dioxus::prelude::*;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Copy, PartialEq)]
struct MoveCheck {
    checked: bool,
    inherited: bool,
}

#[derive(PartialEq)]
struct RowState {
    selected: bool,
    expanded: bool,
    editing: bool,
    searching: bool,
    can_create: bool,
    move_check: Option<MoveCheck>,
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
    read_only: bool,
    language: Language,
    moving: Option<Selection>,
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
        controls: SidebarMove,
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
        let in_vault = workspace.page == Page::Vault;
        let navigation = Navigation {
            entry: in_vault.then(|| workspace.selected.clone()).flatten(),
            folder: (in_vault && workspace.selected.is_none())
                .then(|| workspace.directories.selected.clone())
                .flatten(),
            collapsed: workspace.directories.collapsed.clone(),
            editing: workspace.directories.draft.is_some(),
            searching: !workspace.search.trim().is_empty(),
            read_only: workspace.vault.signing_error().is_some(),
            language,
            moving: (*controls.selection.peek()).clone(),
        };
        let mut dirty = DirtyRows::default();
        let targeted = !fresh
            && self.navigation.as_ref().is_some_and(|previous| {
                previous.editing == navigation.editing
                    && previous.searching == navigation.searching
                    && previous.read_only == navigation.read_only
                    && previous.language == navigation.language
                    && previous.moving == navigation.moving
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
            &navigation,
            state,
            controls,
            false,
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
        navigation: &Navigation,
        state: Session,
        controls: SidebarMove,
        inherited: bool,
        dirty: Option<&DirtyRows>,
    ) -> (Option<Element>, bool) {
        let editing = navigation.editing;
        let searching = navigation.searching;
        let language = navigation.language;
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
                DisplayNode::Record { id, .. } => {
                    (id, false, navigation.entry.as_ref() == Some(id), false)
                }
                DisplayNode::Folder { id, .. } => (
                    id,
                    true,
                    navigation.folder.as_ref() == Some(id),
                    searching || !navigation.collapsed.contains(id),
                ),
            };
            let can_create =
                folder && !editing && !navigation.read_only && navigation.moving.is_none();
            let move_check = navigation.moving.as_ref().map(|selection| {
                let ids = if folder {
                    &selection.folders
                } else {
                    &selection.entries
                };
                MoveCheck {
                    checked: inherited || ids.contains(id),
                    inherited,
                }
            });
            let row = RowState {
                selected,
                expanded,
                editing,
                searching,
                can_create,
                move_check,
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
                DisplayNode::Folder { children, .. } if expanded => self.branch(
                    children,
                    navigation,
                    state,
                    controls,
                    move_check.is_some_and(|check| check.checked),
                    dirty,
                ),
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
                DisplayNode::Record {
                    id,
                    title,
                    icon,
                    url,
                } => {
                    let id = id.clone();
                    let item = MoveItem::Entry(id.clone());
                    let shortcut = item.clone();
                    let context_item = item.clone();
                    rsx! {
                        li { class: if move_check.is_some() { "tree-leaf move-tree-leaf" } else { "tree-leaf" }, key: "entry-{id}",
                            oncontextmenu: move |event| controls.context_menu(state, Some(context_item.clone()), event),
                            onkeydown: move |event| controls.keyboard_menu(state, shortcut.clone(), event),
                            if let Some(check) = move_check {
                                {move_checkbox(controls, item, check, title, language)}
                            }
                            button { class: if selected { "entry-row selected" } else { "entry-row" },
                                "data-kind": "entry", "data-id": "{id}",
                                r#type: "button", aria_current: "{selected}", disabled: editing,
                                onclick: move |_| navigate(state, Destination::Entry(id.clone())),
                                EntryIcon { title: title.clone(), slug: icon.clone(), url: url.clone() }
                                span { class: "entry-row-title", "{title}" }
                            }
                        }
                    }
                }
                DisplayNode::Folder { id, name, children } => {
                    let toggle = id.clone();
                    let select = id.clone();
                    let create_in = id.clone();
                    let item = MoveItem::Folder(id.clone());
                    let shortcut = item.clone();
                    let context_item = item.clone();
                    rsx! {
                        li { class: "tree-folder", key: "folder-{id}",
                            div { class: if selected { "folder-row selected" } else { "folder-row" },
                                oncontextmenu: move |event| controls.context_menu(state, Some(context_item.clone()), event),
                                onkeydown: move |event| controls.keyboard_menu(state, shortcut.clone(), event),
                                button { class: if expanded { "tree-disclosure expanded" } else { "tree-disclosure" },
                                    r#type: "button", disabled: editing || searching,
                                    title: if expanded { language.text(Text::CollapseFolder) } else { language.text(Text::ExpandFolder) },
                                    aria_label: if expanded { language.format(Text::CollapseFolder2, std::slice::from_ref(name)) } else { language.format(Text::ExpandFolder2, std::slice::from_ref(name)) },
                                    aria_expanded: "{expanded}", aria_controls: "folder-{id}",
                                    onclick: move |_| edit_unlocked(state, |u| u.directories.toggle(toggle.clone())),
                                    Symbol { icon: Icon::ChevronRight }
                                }
                                if let Some(check) = move_check {
                                    {move_checkbox(controls, item, check, name, language)}
                                }
                                button { class: "folder-label", r#type: "button",
                                    "data-kind": "directory", "data-id": "{id}",
                                    disabled: editing, aria_current: "{selected}",
                                    aria_expanded: "{expanded}", aria_controls: "folder-{id}",
                                    onclick: move |_| navigate(state, Destination::ToggleDirectory(select.clone())),
                                    Symbol { icon: if expanded { Icon::FolderOpen } else { Icon::Folder } }
                                    span { "{name}" }
                                }
                                if move_check.is_none() {
                                    IconButton { class: "quiet folder-new-entry", icon: Icon::Plus,
                                        label: language.format(Text::NewEntryInFolder, std::slice::from_ref(name)),
                                        disabled: !can_create,
                                        onclick: move |_| navigate(state, Destination::NewEntryInFolder(create_in.clone())),
                                    }
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

fn move_checkbox(
    controls: SidebarMove,
    item: MoveItem,
    check: MoveCheck,
    name: &str,
    language: Language,
) -> Element {
    rsx! {
        input { class: "move-tree-checkbox", r#type: "checkbox",
            aria_label: language.format(Text::MoveSelectItem, &[name]),
            checked: check.checked, disabled: check.inherited,
            onchange: move |event| controls.set_checked(&item, event.checked()),
        }
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
