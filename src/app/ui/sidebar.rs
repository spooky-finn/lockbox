use super::icons::EntryIcon;
use crate::app::context::{Session, edit_unlocked};
use crate::{app::session::Screen, icons};
use dioxus::prelude::*;

#[component]
pub(super) fn Sidebar() -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let editing = unlocked.draft.is_some();
    let visible = unlocked.visible();
    rsx! {
            aside { class: "sidebar",
                label { class: "sr-only", r#for: "search", "Поиск записей" }
                input { id: "search", r#type: "search", placeholder: "Поиск", value: "{unlocked.search}",
                    oninput: move |event| edit_unlocked(state, |u| u.search = event.value()),
                }
                nav { aria_label: "Записи",
                    for entry in &visible {
                        {
                            let id = entry.id.clone();
                            rsx! {
                                button { key: "{id}", class: if unlocked.selected.as_ref() == Some(&id) { "entry-row selected" } else { "entry-row" },
                                    disabled: editing, onclick: move |_| edit_unlocked(state, |u| u.select(id.clone())),
                                    EntryIcon { title: entry.title.clone(), slug: icons::catalog().for_entry(entry).map(|icon| icon.slug.clone()) }
                                    span { span { class: "entry-title", "{entry.title}" } small { "{entry.username}" } }
                                }
                            }
                        }
                    }
                    if visible.is_empty() { p { class: "muted empty-list", "Записей не найдено" } }
                }
            }
    }
}
