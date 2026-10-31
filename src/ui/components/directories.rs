use super::{
    button::IconButton,
    symbol::{Icon, Symbol},
};
use crate::ui::context::{Session, edit_unlocked};
use crate::{app::session::Screen, core::folder};
use dioxus::prelude::*;

#[component]
pub(super) fn DirectoryView() -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };
    let Some(selected) = workspace.directories.selected.as_deref() else {
        return rsx! {};
    };
    let Some(directory) = workspace
        .vault
        .folders
        .iter()
        .find(|folder| folder.id == selected)
    else {
        return rsx! {};
    };
    let breadcrumbs = folder::ancestors(&workspace.vault.folders, selected);
    let records = workspace
        .vault
        .entries
        .iter()
        .filter(|entry| {
            entry
                .folder_id
                .as_deref()
                .is_some_and(|id| folder::contains(&workspace.vault.folders, selected, id))
        })
        .count();
    let folders = workspace
        .vault
        .folders
        .iter()
        .filter(|folder| {
            folder.id != selected
                && folder::contains(&workspace.vault.folders, selected, &folder.id)
        })
        .count();
    rsx! {
        article { class: "directory-detail",
            nav { class: "breadcrumbs", aria_label: "Расположение папки",
                IconButton { class: "quiet", icon: Icon::Folder, label: "Корень хранилища", onclick: move |_| edit_unlocked(state, |u| u.select_directory(None)) }
                for folder in breadcrumbs {
                    {
                        let id = folder.id.clone();
                        rsx! {
                            Symbol { icon: Icon::ChevronRight }
                            button { class: "quiet breadcrumb", onclick: move |_| edit_unlocked(state, |u| u.select_directory(Some(id.clone()))), "{folder.name}" }
                        }
                    }
                }
            }
            div { class: "detail-heading",
                div { class: "directory-icon", Symbol { icon: Icon::FolderOpen } }
                div { p { class: "detail-kind", "Папка" } h1 { "{directory.name}" } }
            }
            div { class: "directory-summary",
                div { class: "directory-stats",
                    div { strong { "{records}" } span { "Записей" } }
                    div { strong { "{folders}" } span { "Вложенных папок" } }
                }
                p { "Выберите запись в дереве или добавьте новую в эту папку." }
                div { class: "actions",
                    IconButton { class: "primary", icon: Icon::Plus, label: "Новая запись", onclick: move |_| edit_unlocked(state, |u| u.start_new()) }
                    IconButton { icon: Icon::FolderPlus, label: "Новая вложенная папка", onclick: move |_| edit_unlocked(state, |u| u.start_directory()) }
                }
            }
            if workspace.directories.confirm_delete {
                div { class: "delete-confirm", role: "alert",
                    p { "Удалить папку «{directory.name}» и её подпапки? Все записи будут перемещены на уровень выше." }
                    div { class: "actions",
                        IconButton { class: "danger", icon: Icon::Trash, label: "Удалить папку", onclick: move |_| edit_unlocked(state, |u| { u.error = u.directories.delete(&mut u.vault).err(); if u.error.is_none() { u.sync.saved(); } }) }
                        IconButton { icon: Icon::Close, label: "Отмена", onclick: move |_| edit_unlocked(state, |u| u.directories.confirm_delete = false) }
                    }
                }
            } else {
                div { class: "actions entry-actions",
                    IconButton { icon: Icon::Edit, label: "Изменить папку", onclick: move |_| edit_unlocked(state, |u| u.edit_directory()) }
                    IconButton { class: "danger quiet", icon: Icon::Trash, label: "Удалить папку…", onclick: move |_| edit_unlocked(state, |u| u.directories.confirm_delete = true) }
                }
            }
        }
    }
}
