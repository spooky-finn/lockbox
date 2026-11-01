use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        widgets::symbol::{Icon, Symbol},
        workspace::Unlocked,
    },
    records::tree::Node,
};
use dioxus::prelude::*;

pub(crate) fn nodes_ui(
    nodes: &[Node<'_>],
    workspace: &Unlocked,
    state: Session,
    language: crate::settings::language::Language,
) -> Element {
    let editing = workspace.is_editing();
    let searching = !workspace.search.trim().is_empty();
    rsx! {
        for node in nodes {
            {match node {
                Node::Record(entry) => {
                    let id = entry.id.clone();
                    let selected = workspace.page == crate::desktop::workspace::Page::Vault && workspace.selected.as_ref() == Some(&id);
                    rsx! {
                        li { class: "tree-leaf", key: "entry-{id}",
                            button { class: if selected { "entry-row selected" } else { "entry-row" },
                                r#type: "button", title: "{entry.title}", aria_current: "{selected}", disabled: editing,
                                onclick: move |_| edit_unlocked(state, |u| u.select(id.clone())),
                                "{entry.title}"
                            }
                        }
                    }
                }
                Node::Folder(folder, children) => {
                    let id = folder.id.clone();
                    let toggle = id.clone();
                    let select = id.clone();
                    let keyboard = id.clone();
                    let expanded = !workspace.directories.collapsed.contains(&id) || !workspace.search.trim().is_empty();
                    let selected = workspace.page == crate::desktop::workspace::Page::Vault && workspace.selected.is_none() && workspace.directories.selected.as_ref() == Some(&id);
                    rsx! {
                        li { class: "tree-folder", key: "folder-{id}",
                            div { class: if selected { "folder-row selected" } else { "folder-row" },
                                onkeydown: move |event| {
                                    if editing { return; }
                                    match event.key() {
                                        Key::ArrowRight => { event.prevent_default(); edit_unlocked(state, |u| { u.directories.collapsed.remove(&keyboard); }); }
                                        Key::ArrowLeft => { event.prevent_default(); edit_unlocked(state, |u| { u.directories.collapsed.insert(keyboard.clone()); }); }
                                        _ => {}
                                    }
                                },
                                button { class: if expanded { "tree-disclosure expanded" } else { "tree-disclosure" },
                                    r#type: "button", disabled: editing || searching,
                                    title: if expanded { language.text(Text::CollapseFolder) } else { language.text(Text::ExpandFolder) },
                                    aria_label: if expanded { language.format(Text::CollapseFolder2, std::slice::from_ref(&folder.name)) } else { language.format(Text::ExpandFolder2, std::slice::from_ref(&folder.name)) },
                                    aria_expanded: "{expanded}", aria_controls: "folder-{id}",
                                    onclick: move |_| edit_unlocked(state, |u| u.directories.toggle(toggle.clone())),
                                    Symbol { icon: Icon::ChevronRight }
                                }
                                button { class: "folder-label", r#type: "button", title: "{folder.name}",
                                    disabled: editing, aria_current: "{selected}",
                                    onclick: move |_| edit_unlocked(state, |u| u.select_directory(Some(select.clone()))),
                                    Symbol { icon: if expanded { Icon::FolderOpen } else { Icon::Folder } }
                                    span { "{folder.name}" }
                                }
                            }

                            if expanded {
                                ul { class: "tree-branch", id: "folder-{id}",
                                    if children.is_empty() { li { class: "tree-empty", {language.text(Text::EmptyFolder)} } }
                                    {nodes_ui(children, workspace, state, language)}
                                }
                            }
                        }
                    }
                }
            }}
        }
    }
}

#[cfg(test)]
#[path = "tests/tree.rs"]
mod tests;
