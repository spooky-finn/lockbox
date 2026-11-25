//! Desktop coordination for moving records between unlocked tabs.

use crate::{
    desktop::{
        context::Session,
        i18n::{Text, use_language},
        session::Screen,
        tabs::Tabs,
        window,
        workspace::Unlocked,
    },
    records::tree::{self, Node, SortedTopology},
    vault::move_records::{Move, Selection, same_space},
};
use dioxus::prelude::*;
use std::{collections::BTreeMap, path::PathBuf};

const TRIGGER_ID: &str = "move-records-trigger";
const DESTINATION_ID: &str = "move-records-destination";
const SUBMIT_ID: &str = "move-records-submit";

#[component]
pub(crate) fn MoveRecordsButton() -> Element {
    let language = use_language();
    let available = crate::desktop::context::use_unlocked_selector(|u| {
        can_move(u) && !(u.vault.entries().is_empty() && u.vault.folders().is_empty())
    });
    let mut open = use_signal(|| false);
    let mut notice = use_signal(|| None::<String>);

    rsx! {
        if let Some(message) = notice.read().as_ref() {
            p { class: "move-notice muted", role: "status", "{message}" }
        }
        button { id: TRIGGER_ID, class: "quiet", r#type: "button",
            disabled: available() != Some(true),
            onclick: move |_| {
                notice.set(None);
                open.set(true);
            },
            {language.text(Text::MoveRecords)}
        }
        if open() {
            MoveDialog { onclose: move |message: Option<String>| {
                let _ = document::eval("document.getElementById('move-records-dialog')?.close()");
                notice.set(message);
                open.set(false);
            } }
        }
    }
}

fn can_move(workspace: &Unlocked) -> bool {
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
}

#[component]
fn MoveDialog(onclose: EventHandler<Option<String>>) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let tabs = use_context::<Signal<Tabs>>();
    let settings = use_context::<Signal<crate::settings::state::Settings>>();
    let preferences = settings.read();
    let mut selected = use_signal(|| {
        let mut selection = Selection::default();
        if let Screen::Unlocked(source) = &*state.peek() {
            if let Some(entry) = source.selected_entry() {
                selection.entries.insert(entry.id.clone());
            } else if let Some(id) = &source.directories.selected {
                selection.folders.insert(id.clone());
            }
        }
        selection
    });
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
    let selection = selected.read();
    let retry = pending.read().is_some();
    let tree = if retry {
        Vec::new()
    } else {
        tree::project_indexed(&source.vault, "", &SortedTopology::new(&source.vault))
    };
    let mut select_all = Selection {
        entire_vault: true,
        ..Selection::default()
    };
    for node in &tree {
        match node {
            Node::Folder(folder, _) => select_all.folders.insert(folder.id.clone()),
            Node::Record(entry) => select_all.entries.insert(entry.id.clone()),
        };
    }

    rsx! {
        dialog { id: "move-records-dialog", class: "entry-trash-dialog move-records-dialog",
            aria_labelledby: "move-records-title",
            onmounted: move |_| {
                let _ = document::eval("document.getElementById('move-records-dialog')?.showModal()");
            },
            onkeydown: move |event| event.stop_propagation(),
            oncancel: move |event| {
                event.prevent_default();
                onclose.call(None);
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
            p { class: "muted", {language.text(Text::MoveHint)} }
            if !retry {
                div { class: "move-selection-actions",
                    button { class: "quiet", r#type: "button",
                        onclick: move |_| selected.set(select_all.clone()),
                        {language.text(Text::MoveSelectAll)}
                    }
                    button { class: "quiet", r#type: "button",
                        onclick: move |_| selected.set(Selection::default()),
                        {language.text(Text::MoveClearSelection)}
                    }
                }
                ul { class: "move-selection",
                    for node in tree {
                        {selection_row(node, false, &selection, selected)}
                    }
                }
            }
            if let Some(message) = error.read().as_ref() {
                p { class: "error", role: "alert", {language.message(message)} }
            }
            if retry {
                p { class: "muted", {language.text(Text::MoveRetryHint)} }
            }
            div { class: "actions",
                button { r#type: "button", onclick: move |_| onclose.call(None),
                    {language.text(Text::Cancel)}
                }
                button { id: SUBMIT_ID, class: "primary", r#type: "button",
                    disabled: target.is_none() || !can_move(source) || (selection.entries.is_empty() && selection.folders.is_empty()),
                    onclick: move |_| {
                        let Some(target) = &target else { return };
                        match move_between(state, target, &source_version, &selected.peek(), pending) {
                            Ok(()) => onclose.call(Some(language.format(Text::MoveComplete, &[&target.title]))),
                            Err(message) => error.set(Some(message)),
                        }
                    },
                    {language.text(if retry { Text::MoveRetry } else { Text::MoveConfirm })}
                }
            }
        }
    }
}

fn selection_row(
    node: Node<'_>,
    included: bool,
    selection: &Selection,
    mut selected: Signal<Selection>,
) -> Element {
    let (id, name, username, children) = match node {
        Node::Folder(folder, children) => (&folder.id, &folder.name, None, Some(children)),
        Node::Record(entry) => (
            &entry.id,
            &entry.title,
            Some(entry.username.as_str()).filter(|name| !name.is_empty()),
            None,
        ),
    };
    let is_folder = children.is_some();
    let checked = included
        || if is_folder {
            selection.folders.contains(id)
        } else {
            selection.entries.contains(id)
        };
    let record_id = id.clone();
    let key = format!("{is_folder}-{id}");

    rsx! {
        li { key: "{key}",
            label {
                input { r#type: "checkbox", checked, disabled: included,
                    onchange: move |event| {
                        let mut selection = selected.write();
                        selection.entire_vault = false;
                        let ids = if is_folder { &mut selection.folders } else { &mut selection.entries };
                        if event.checked() {
                            ids.insert(record_id.clone());
                        } else {
                            ids.remove(&record_id);
                        }
                    },
                }
                if is_folder {
                    crate::desktop::widgets::symbol::Symbol { icon: crate::desktop::widgets::symbol::Icon::Folder }
                }
                span {
                    "{name}"
                    if let Some(username) = username {
                        span { class: "muted", " · {username}" }
                    }
                }
            }
            if let Some(children) = children && !children.is_empty() {
                ul {
                    for child in children {
                        {selection_row(child, checked, selection, selected)}
                    }
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
