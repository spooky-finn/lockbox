//! Desktop coordination for moving records within and between vaults.

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        i18n::{Text, use_language},
        session::Screen,
        tabs::Tabs,
        window,
        workspace::{Page, Unlocked},
    },
    records::{
        folder::Hierarchy,
        tree::{self, Node, SortedTopology},
    },
    vault::{
        self,
        move_records::{Move, Selection, same_space},
    },
};
use dioxus::{html::InteractionLocation, prelude::*};
use std::{collections::BTreeMap, path::PathBuf};

const MENU_ACTION_ID: &str = "move-records-menu-action";
const CONTINUE_ID: &str = "move-records-continue";
const DESTINATION_ID: &str = "move-records-destination";
const SUBMIT_ID: &str = "move-records-submit";

#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub(crate) enum MoveItem {
    Entry(String),
    Folder(String),
}

#[derive(serde::Deserialize)]
pub(crate) struct TreeDrop {
    item: MoveItem,
    parent: Option<String>,
}

pub(crate) fn move_in_vault(state: Session, action: TreeDrop) {
    crate::desktop::runtime::record_activity();
    edit_unlocked(state, |workspace| {
        if workspace.page == Page::Vault
            && can_move(workspace)
            && let Err(error) = reparent(workspace, action)
        {
            workspace.error = Some(format!("Could not save: {error}"));
        }
    });
}

fn reparent(workspace: &mut Unlocked, TreeDrop { item, parent }: TreeDrop) -> vault::Result<()> {
    let vault = &mut workspace.vault;
    if let Some(parent) = &parent
        && !vault.folders().iter().any(|folder| &folder.id == parent)
    {
        return Ok(());
    }

    match item {
        MoveItem::Entry(id) => {
            let Some(entry) = vault.entries().iter().find(|entry| entry.id == id) else {
                return Ok(());
            };
            if entry.folder_id == parent {
                return Ok(());
            }
            let mut entry = entry.clone();
            entry.folder_id.clone_from(&parent);
            vault.upsert_entry(entry)?;
            if workspace.selected.as_ref() == Some(&id) {
                workspace.directories.selected.clone_from(&parent);
            }
        }
        MoveItem::Folder(id) => {
            let Some(folder) = vault.folders().iter().find(|folder| folder.id == id) else {
                return Ok(());
            };
            if folder.parent_id == parent {
                return Ok(());
            }
            let mut folder = folder.clone();
            folder.parent_id.clone_from(&parent);
            vault.save_folder(folder)?;
        }
    }

    if let Some(parent) = parent {
        for folder in Hierarchy::new(vault.folders()).lineage(&parent) {
            workspace.directories.collapsed.remove(&folder.id);
        }
    }
    workspace.sync.saved();
    workspace.error = None;
    Ok(())
}

#[derive(Clone)]
struct ContextMenu {
    item: Option<MoveItem>,
    position: Option<(f64, f64)>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct SidebarMove {
    pub(crate) selection: Signal<Option<Selection>>,
    menu: Signal<Option<ContextMenu>>,
}

impl SidebarMove {
    pub(crate) fn use_controls() -> Self {
        Self {
            selection: use_signal(|| None),
            menu: use_signal(|| None),
        }
    }

    pub(crate) fn is_busy(self) -> bool {
        self.selection.read().is_some() || self.menu.read().is_some()
    }

    fn open(mut self, state: Session, item: Option<MoveItem>, position: Option<(f64, f64)>) {
        if self.selection.peek().is_some() {
            return;
        }
        if matches!(&*state.peek(), Screen::Unlocked(u)
            if u.page == Page::Vault && can_move(u)
                && (!u.vault.entries().is_empty() || !u.vault.folders().is_empty()))
        {
            self.menu.set(Some(ContextMenu { item, position }));
        }
    }

    pub(crate) fn context_menu(self, state: Session, item: Option<MoveItem>, event: MouseEvent) {
        event.prevent_default();
        event.stop_propagation();
        let point = event.client_coordinates();
        self.open(state, item, Some((point.x, point.y)));
    }

    pub(crate) fn keyboard_menu(self, state: Session, item: MoveItem, event: KeyboardEvent) {
        if event.key() == Key::ContextMenu
            || (event.key() == Key::F10 && event.modifiers() == Modifiers::SHIFT)
        {
            event.prevent_default();
            event.stop_propagation();
            self.open(state, Some(item), None);
        }
    }

    pub(crate) fn dismiss(mut self) -> bool {
        if self.menu.peek().is_some() {
            self.menu.set(None);
        } else if self.selection.peek().is_some() {
            self.selection.set(None);
        } else {
            return false;
        }
        true
    }

    pub(crate) fn set_checked(mut self, item: &MoveItem, checked: bool) {
        if let Some(selection) = self.selection.write().as_mut() {
            selection.entire_vault = false;
            check_item(selection, item, checked);
        }
    }
}

fn check_item(selection: &mut Selection, item: &MoveItem, checked: bool) {
    let (ids, id) = match item {
        MoveItem::Entry(id) => (&mut selection.entries, id),
        MoveItem::Folder(id) => (&mut selection.folders, id),
    };
    if checked {
        ids.insert(id.clone());
    } else {
        ids.remove(id);
    }
}

#[component]
pub(crate) fn MoveSidebar(mut controls: SidebarMove) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let available =
        crate::desktop::context::use_unlocked_selector(|u| u.page == Page::Vault && can_move(u));
    let mut choosing = use_signal(|| false);
    let mut notice = use_signal(|| None::<String>);
    let menu = controls.menu.read();
    let selection = controls.selection.read();

    rsx! {
        if let Some(message) = notice.read().as_ref() {
            p { class: "move-notice muted", role: "status", "{message}" }
        }
        if let Some(menu) = menu.as_ref().filter(|_| available() == Some(true)) {
            { let item = menu.item.clone();
              let style = menu.position.map(|(x, y)| format!("left:{x}px;top:{y}px"));
              rsx! {
                button { class: "space-switch-backdrop", r#type: "button", tabindex: "-1",
                    aria_label: language.text(Text::Cancel),
                    onclick: move |_| controls.menu.set(None),
                    oncontextmenu: move |event| {
                        event.prevent_default();
                        controls.menu.set(None);
                    },
                }
                div { class: "move-context-menu", id: "move-context-menu", role: "menu", style,
                    onmounted: move |_| {
                        let _ = document::eval("
                            const menu = document.getElementById('move-context-menu');
                            if (!menu) return;
                            const active = document.activeElement?.getBoundingClientRect();
                            menu.style.left = Math.max(4, Math.min(parseFloat(menu.style.left) || active?.left || 4, innerWidth - menu.offsetWidth - 4)) + 'px';
                            menu.style.top = Math.max(4, Math.min(parseFloat(menu.style.top) || active?.bottom || 4, innerHeight - menu.offsetHeight - 4)) + 'px';
                            menu.querySelector('button')?.focus({preventScroll: true});
                        ");
                    },
                    button { id: MENU_ACTION_ID, class: "quiet", r#type: "button", role: "menuitem",
                        onclick: move |_| {
                            let mut selection = Selection::default();
                            if let Some(item) = &item {
                                check_item(&mut selection, item, true);
                            }
                            controls.menu.set(None);
                            controls.selection.set(Some(selection));
                            notice.set(None);
                        },
                        {language.text(Text::MoveRecords)}
                    }
                }
              }
            }
        }
        if let Some(selection) = selection.as_ref() {
            div { class: "move-sidebar-actions",
                button { class: "quiet", r#type: "button",
                    onclick: move |_| {
                        if let Screen::Unlocked(source) = &*state.peek() {
                            let selected = if controls.selection.peek().as_ref().is_some_and(|s| s.entire_vault) {
                                Selection::default()
                            } else {
                                select_all(source)
                            };
                            controls.selection.set(Some(selected));
                        }
                    },
                    {language.text(if selection.entire_vault { Text::MoveClearSelection } else { Text::MoveSelectAll })}
                }
                button { class: "quiet", r#type: "button", onclick: move |_| { controls.dismiss(); },
                    {language.text(Text::Cancel)}
                }
                button { id: CONTINUE_ID, class: "primary", r#type: "button",
                    disabled: selection.is_empty() || available() != Some(true),
                    onclick: move |_| choosing.set(true),
                    {language.text(Text::MoveChooseDestination)}
                }
            }
            if choosing() {
                MoveDialog { selection: selection.clone(), onclose: move |outcome: Outcome| {
                    let _ = document::eval("document.getElementById('move-records-dialog')?.close()");
                    choosing.set(false);
                    match outcome {
                        Outcome::Cancelled => {}
                        Outcome::Interrupted => controls.selection.set(None),
                        Outcome::Moved(message) => {
                            controls.selection.set(None);
                            notice.set(Some(message));
                        }
                    }
                } }
            }
        }
    }
}

fn select_all(source: &Unlocked) -> Selection {
    let mut selection = Selection {
        entire_vault: true,
        ..Selection::default()
    };
    let topology = SortedTopology::new(&source.vault);
    for node in tree::project_indexed(&source.vault, "", &topology) {
        match node {
            Node::Folder(folder, _) => selection.folders.insert(folder.id.clone()),
            Node::Record(entry) => selection.entries.insert(entry.id.clone()),
        };
    }
    selection
}

#[derive(Clone)]
enum Outcome {
    Cancelled,
    Interrupted,
    Moved(String),
}

pub(crate) fn can_move(workspace: &Unlocked) -> bool {
    workspace.editor_is_clear()
        && !workspace.import.busy()
        && !workspace.sharing.busy()
        && !workspace.sharing.joining
        && !workspace.vault.pairing_waits_to_join()
        && workspace.vault.signing_error().is_none()
}

#[derive(Clone)]
struct Destination {
    key: String,
    tab: usize,
    title: String,
    vault_id: String,
    path: PathBuf,
    session: Session,
    ready: bool,
    shared: bool,
}

#[component]
fn MoveDialog(selection: Selection, onclose: EventHandler<Outcome>) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let tabs = use_context::<Signal<Tabs>>();
    let settings = use_context::<Signal<crate::settings::state::Settings>>();
    let preferences = settings.read();
    let mut destination = use_signal(String::new);
    let pending = use_signal(|| None::<Move>);
    let mut error = use_signal(|| None::<String>);

    let screen = state.read();
    let Screen::Unlocked(source) = &*screen else {
        return rsx! {};
    };
    let source_version = (source.vault.id(), source.vault.records_revision());
    let tabs_read = tabs.read();
    let mut destinations: Vec<_> = tabs_read
        .sessions
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, session)| *session != state)
        .filter_map(|(index, session)| {
            let screen = session.read();
            let Screen::Unlocked(target) = &*screen else {
                return None;
            };
            (!same_space(&source.vault, &target.vault)).then(|| {
                let vault_id = target.vault.id();
                let path = target.vault.path().to_path_buf();
                Destination {
                    key: format!("{vault_id}:{path:?}"),
                    tab: index + 1,
                    title: window::title(&screen, &preferences.config),
                    vault_id,
                    path,
                    session,
                    ready: can_move(target),
                    shared: target.vault.profile_summary().is_some(),
                }
            })
        })
        .collect();
    let mut title_counts = BTreeMap::new();
    for target in &destinations {
        *title_counts.entry(target.title.clone()).or_insert(0) += 1;
    }
    for target in &mut destinations {
        if title_counts[&target.title] > 1 {
            target.title = language.format(
                Text::MoveTab,
                &[target.title.clone(), target.tab.to_string()],
            );
        }
    }
    let destination_key = destination();
    let target = destinations
        .iter()
        .find(|target| target.key == destination_key && target.ready)
        .cloned();
    let retry = pending.read().is_some();
    let on_cancel = move || {
        onclose.call(if retry {
            Outcome::Interrupted
        } else {
            Outcome::Cancelled
        })
    };

    rsx! {
        dialog { id: "move-records-dialog", class: "entry-trash-dialog move-records-dialog",
            aria_labelledby: "move-records-title",
            onmounted: move |_| {
                let _ = document::eval("document.getElementById('move-records-dialog')?.showModal()");
            },
            onkeydown: move |event| event.stop_propagation(),
            oncancel: move |event| {
                event.prevent_default();
                on_cancel();
            },
            h2 { id: "move-records-title", {language.text(Text::MoveRecords)} }
            label { r#for: DESTINATION_ID, {language.text(Text::MoveDestination)} }
            select { id: DESTINATION_ID, value: destination_key, disabled: retry,
                onchange: move |event| destination.set(event.value()),
                option { value: "", {language.text(Text::MoveChooseSpace)} }
                for target in &destinations {
                    option { key: "{target.key}", value: "{target.key}", disabled: !target.ready, "{target.title}" }
                }
            }
            if !destinations.iter().any(|target| target.ready) {
                p { class: "muted", {language.text(Text::MoveOpenSpace)} }
            }
            if target.as_ref().is_some_and(|target| target.shared) {
                p { class: "muted", {language.text(Text::MoveSharedAccess)} }
            }
            if let Some(message) = error.read().as_ref() {
                p { class: "error", role: "alert", {language.message(message)} }
            }
            if retry {
                p { class: "muted", {language.text(Text::MoveRetryHint)} }
            }
            div { class: "actions",
                button { r#type: "button", onclick: move |_| on_cancel(),
                    {language.text(Text::Cancel)}
                }
                button { id: SUBMIT_ID, class: "primary", r#type: "button",
                    disabled: target.is_none() || !can_move(source) || selection.is_empty(),
                    onclick: move |_| {
                        let Some(target) = &target else { return };
                        match move_between(state, target, &source_version, &selection, pending) {
                            Ok(()) => onclose.call(Outcome::Moved(language.format(Text::MoveComplete, &[&target.title]))),
                            Err(message) => error.set(Some(message)),
                        }
                    },
                    {language.text(if retry { Text::MoveRetry } else { Text::MoveConfirm })}
                }
            }
        }
    }
}

fn move_between(
    mut state: Session,
    target: &Destination,
    source_version: &(String, u64),
    selection: &Selection,
    mut pending: Signal<Option<Move>>,
) -> Result<(), String> {
    let unavailable = || "Both spaces must be unlocked and ready to edit".to_owned();
    let mut target_session = target.session;
    if state == target_session {
        return Err(unavailable());
    }
    let mut current = state.write();
    let mut other = target_session.write();
    let (Screen::Unlocked(source), Screen::Unlocked(destination)) = (&mut *current, &mut *other)
    else {
        return Err(unavailable());
    };
    if source.vault.id() != source_version.0
        || destination.vault.id() != target.vault_id
        || destination.vault.path() != target.path
        || !can_move(source)
        || !can_move(destination)
    {
        return Err(unavailable());
    }

    let mut plan = pending.write();
    if plan.is_none() {
        if source.vault.records_revision() != source_version.1 {
            return Err(crate::vault::move_records::MoveError::SourceChanged.to_string());
        }
        *plan = Some(
            Move::new(&source.vault, &destination.vault, selection)
                .map_err(|error| error.to_string())?,
        );
    }
    let transfer = plan.as_mut().unwrap();
    let result = transfer.execute(&mut source.vault, &mut destination.vault);
    source.sync.saved();
    destination.sync.saved();
    if source.selected.is_some() && source.selected_entry().is_none()
        || source
            .directories
            .selected
            .as_ref()
            .is_some_and(|id| !source.vault.folders().iter().any(|folder| &folder.id == id))
    {
        source.select_directory(None);
    }
    source
        .directories
        .collapsed
        .retain(|id| source.vault.folders().iter().any(|folder| &folder.id == id));
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            if !transfer.copying_started() {
                *plan = None;
            }
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
#[path = "tests/move_records.rs"]
mod tests;
