use super::{
    button::IconButton,
    symbol::{Icon, Symbol},
};
use super::{
    directories::DirectoryView, directory_editor::DirectoryEditor, editor::DraftView,
    entry::EntryView, sidebar::Sidebar, status::StatusBar, toolbar::Toolbar,
};
use crate::app::session::Screen;
use crate::ui::context::{Session, edit_unlocked};
use dioxus::prelude::*;

#[component]
pub(super) fn UnlockedView() -> Element {
    let mut revealed = use_signal(std::collections::BTreeSet::<crate::core::sync::Revision>::new);
    let mut revealed_drafts =
        use_signal(std::collections::BTreeSet::<crate::core::sync::Hash>::new);
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let editing = unlocked.is_editing();
    let has_detail =
        unlocked.selected.is_some() || unlocked.directories.selected.is_some() || editing;
    rsx! {
        Toolbar {}
        div { class: if has_detail { "workspace has-detail" } else { "workspace" },
            Sidebar {}
            section { class: "detail",
                if !editing && has_detail {
                    IconButton { class: "mobile-back quiet", icon: Icon::ArrowLeft, label: "К списку", onclick: move |_| edit_unlocked(state, |u| u.select_directory(None)) }
                }
                if let Some(error) = &unlocked.error { p { class: "error", role: "alert", "{error}" } }
                if let Some(profile) = &unlocked.vault.profile {
                    if let Some(sync) = &profile.sync {
                        p { class: "notice", aria_live: "polite", "{unlocked.sync.status.label()} · ожидают отправки: {sync.outbox.len()}" }
                        button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| u.sync.retry()), "Повторить обмен" }
                    } else {
                        p { class: "notice", "Профиль создан до появления синхронизации. Подтвердите пароль восстановления, чтобы подключить существующие записи." }
                        form {
                            onsubmit: move |event: FormEvent| { event.prevent_default(); edit_unlocked(state, |u| u.activate_sync()); },
                            input { r#type: "password", autocomplete: "off", aria_label: "Пароль восстановления для включения синхронизации", placeholder: "Пароль восстановления", value: unlocked.sync_recovery.as_str(),
                                oninput: move |event| edit_unlocked(state, |u| u.sync_recovery = zeroize::Zeroizing::new(event.value())),
                            }
                            button { r#type: "submit", "Включить синхронизацию" }
                        }
                    }
                }
                if let Ok(Some(replay)) = unlocked.vault.sync_replay() {
                    if !replay.history_verified {
                        p { class: "notice", "Начальное состояние подтверждено владельцем. Старые изменения не проверены этим устройством." }
                    }
                    for record in replay.projection.conflicts() {
                        details { class: "notice",
                            summary { "Конфликт: {record.entity.id} · версий: {record.versions.len()}" }
                            for version in &record.versions {
                                div {
                                    p { "Устройство: {version.author}" }
                                    match &version.value {
                                        Some(crate::core::sync::Value::Entry(entry)) => rsx! { p { "{entry.title} · {entry.username} · {entry.url}" } },
                                        Some(crate::core::sync::Value::Folder(folder)) => rsx! { p { "{folder.name}" } },
                                        None => rsx! { p { "Удалено" } },
                                    }
                                    button {
                                        onclick: { let revision = version.revision; move |_| { let mut selected = revealed.write(); if !selected.remove(&revision) { selected.insert(revision); } } },
                                        "Показать / скрыть содержимое"
                                    }
                                    if revealed.read().contains(&version.revision) {
                                        match &version.value {
                                            Some(crate::core::sync::Value::Entry(entry)) => rsx! {
                                                p { "Пароль: {entry.password}" }
                                                p { "Заметки: {entry.notes}" }
                                                p { "Папка: {entry.folder_id.clone().unwrap_or_default()}" }
                                                if let Some(totp) = &entry.totp { p { "TOTP: {totp.secret} · {totp.algorithm:?} · {totp.digits} · {totp.period} с" } }
                                            },
                                            Some(crate::core::sync::Value::Folder(folder)) => rsx! { p { "Родитель: {folder.parent_id.clone().unwrap_or_default()}" } },
                                            None => rsx! {},
                                        }
                                    }
                                    button {
                                        onclick: {
                                            let entity = record.entity.clone();
                                            let revision = version.revision;
                                            move |_| edit_unlocked(state, |u| { u.error = u.vault.resolve_sync(entity.clone(), revision).err().map(|e| e.to_string()); u.sync.saved(); })
                                        },
                                        "Сохранить эту версию"
                                    }
                                }
                            }
                        }
                    }
                    if !replay.projection.structural_conflicts().is_empty() {
                        p { class: "notice", "Конфликт расположения папок. Все записи сохранены; исправьте расположение папок." }
                    }
                    if !replay.excluded.is_empty() {
                        p { class: "notice", "Есть изменения из закрытой эпохи. Они сохранены локально и требуют переноса после проверки." }
                        if let Ok(edits) = unlocked.vault.excluded_edits() {
                            for edit in edits {
                                details {
                                    summary { "Сохранённое изменение: {edit.hash}" }
                                    p { "Автор исходного изменения: {edit.author}" }
                                    button { onclick: { let hash = edit.hash; move |_| { let mut selected = revealed_drafts.write(); if !selected.remove(&hash) { selected.insert(hash); } } }, "Показать / скрыть содержимое" }
                                    if revealed_drafts.read().contains(&edit.hash) {
                                        for mutation in &edit.transaction.mutations {
                                            match &mutation.value {
                                                Some(crate::core::sync::Value::Entry(entry)) => rsx! { p { "{entry.title} · {entry.username} · {entry.url}" } p { "Пароль: {entry.password}" } p { "{entry.notes}" } },
                                                Some(crate::core::sync::Value::Folder(folder)) => rsx! { p { "Папка: {folder.name}" } },
                                                None => rsx! { p { "Удаление: {mutation.entity.id}" } },
                                            }
                                        }
                                        p { "Перенос заменит текущие версии выбранным содержимым. Автором нового изменения будете вы." }
                                        button { onclick: { let hash = edit.hash; move |_| edit_unlocked(state, |u| { u.error = u.vault.rebase_edit(hash).err().map(|e| e.to_string()); u.sync.saved(); }) }, "Перенести после проверки" }
                                    }
                                }
                            }
                        }
                    }
                }
                if unlocked.directories.draft.is_some() { DirectoryEditor {} }
                else if unlocked.draft.is_some() { DraftView {} }
                else if unlocked.selected_entry().is_some() { EntryView {} }
                else if unlocked.directories.selected.is_some() { DirectoryView {} }
                else { div { class: "empty-state",
                    div { class: "empty-icon", Symbol { icon: Icon::Key } }
                    if let Some(profile) = &unlocked.vault.profile {
                        div { class: "profile-summary",
                            h2 { "{profile.space_name}" }
                            p { "Пользователь: {profile.username}" }
                            if profile.sync.is_some() {
                                p { class: "notice", "Изменения сохраняются на устройстве и передаются через Storj в зашифрованном виде." }
                            }
                            details {
                                summary { "Идентичность и ключи" }
                                p { "Отпечаток пользователя" }
                                code { "{crate::core::identity::hex(&profile.identity.user_id)}" }
                                p { "Отпечаток устройства" }
                                code { "{crate::core::identity::hex(&profile.identity.device_id)}" }
                                p { "ID хранилища" }
                                code { "{crate::core::identity::hex(&profile.space.space_id)}" }
                            }
                        }
                    } else { h2 { "Всё важное — под замком" } }
                    p { "Выберите запись слева или создайте новую." }
                    div { class: "actions",
                        IconButton { class: "primary", icon: Icon::Plus, label: "Новая запись", onclick: move |_| edit_unlocked(state, |u| u.start_new()) }
                        IconButton { icon: Icon::FolderPlus, label: "Новая папка", onclick: move |_| edit_unlocked(state, |u| u.start_directory()) }
                    }
                } }
            }
        }
        StatusBar {}
    }
}

#[cfg(test)]
#[path = "tests/workspace.rs"]
mod tests;
