use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        widgets::symbol::{Icon, Symbol},
        workspace::Unlocked,
    },
    records::tree::DisplayNode,
};
use dioxus::prelude::*;

pub(crate) fn nodes_ui(
    nodes: &[DisplayNode],
    workspace: &Unlocked,
    state: Session,
    language: crate::settings::language::Language,
) -> Element {
    let editing = workspace.directories.draft.is_some();
    let searching = !workspace.search.trim().is_empty();
    rsx! {
        for node in nodes {
            {match node {
                DisplayNode::Record { id, title } => {
                    let id = id.clone();
                    let selected = workspace.page == crate::desktop::workspace::Page::Vault && workspace.selected.as_ref() == Some(&id);
                    rsx! {
                        li { class: "tree-leaf", key: "entry-{id}",
                            button { class: if selected { "entry-row selected" } else { "entry-row" },
                                r#type: "button", title: "{title}", aria_current: "{selected}", disabled: editing,
                                onclick: move |_| edit_unlocked(state, |u| u.navigate(crate::desktop::workspace::Destination::Entry(id.clone()))),
                                "{title}"
                            }
                        }
                    }
                }
                DisplayNode::Folder { id, name, children } => {
                    let id = id.clone();
                    let toggle = id.clone();
                    let select = id.clone();
                    let expanded = !workspace.directories.collapsed.contains(&id) || !workspace.search.trim().is_empty();
                    let selected = workspace.page == crate::desktop::workspace::Page::Vault && workspace.selected.is_none() && workspace.directories.selected.as_ref() == Some(&id);
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
