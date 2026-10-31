use super::{
    button::IconButton,
    lock_mark::LockMark,
    symbol::{Icon, Symbol},
};
use crate::app::{
    auth::{MIN_PASSWORD_CHARS, Mode, StorageKind},
    editor::replace_secret,
    session::Screen,
    settings::Settings,
};
use crate::ui::context::{BiometricProvider, Session, edit_locked};
use crate::ui::runtime;
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
pub(super) fn LockedView() -> Element {
    let state = use_context::<Session>();
    let settings = use_context::<Signal<Settings>>();
    let hardware = use_context::<BiometricProvider>();
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };
    let busy = locked.pending.is_some();
    let create = locked.mode == Mode::Create;
    let cloud_password = create && locked.cloud_password_step;
    let ready = cloud_password
        || locked
            .path
            .as_ref()
            .is_some_and(|path| create || path.exists());
    let touch_id = ready && !create && locked.touch_id.is_some();
    let filename = locked
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = locked
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let weak_password = create
        && !locked.password.is_empty()
        && locked.password.chars().count() < MIN_PASSWORD_CHARS;
    let submit = move |event: FormEvent| {
        event.prevent_default();
        if cloud_password {
            runtime::create_cloud_profile();
            return;
        }
        edit_locked(state, |locked| {
            if locked.pending.is_none() {
                locked.submit();
            }
        });
    };
    rsx! {
        section { class: "lock-screen", aria_label: "Разблокировка хранилища",
            div { class: "unlock-content",
                LockMark {
                    if touch_id {
                        IconButton { class: "lock-badge touch-id", icon: Icon::Fingerprint, label: "Открыть по Touch ID", disabled: busy,
                            onclick: move |_| {
                                let biometrics = hardware.0.clone();
                                edit_locked(state, |locked| locked.start_touch_id(biometrics));
                            },
                        }
                    } else {
                        span { class: "lock-badge", aria_hidden: "true", Symbol { icon: Icon::Lock } }
                    }
                }
                h1 { class: "unlock-title",
                    if create { "Создайте хранилище" }
                    else if ready { "Хранилище заблокировано" }
                    else { "Ваши пароли под замком" }
                }
                p { class: "unlock-description", title: "{path}",
                    if cloud_password { "Придумайте мастер-пароль для защиты локального профиля." }
                    else if create && locked.path.is_none() { "Выберите тип хранилища." }
                    else if create { "Придумайте мастер-пароль для нового хранилища «{filename}»." }
                    else if touch_id { "Используйте Touch ID или введите мастер-пароль, чтобы открыть «{filename}»." }
                    else if ready { "Введите мастер-пароль, чтобы открыть хранилище «{filename}»." }
                    else { "Откройте хранилище или создайте новое, чтобы сохранить свои пароли." }
                }
                if create && locked.path.is_none() && !cloud_password {
                    div { class: "storage-choices",
                        button { r#type: "button", class: "storage-choice", onclick: move |_| runtime::pick_vault(true),
                            strong { "Однопользовательское" }
                            span { "Локальное хранилище на этом устройстве" }
                        }
                        button { r#type: "button", class: "storage-choice",
                            aria_pressed: locked.storage_kind == Some(StorageKind::Cloud),
                            onclick: move |_| edit_locked(state, |locked| {
                                locked.storage_kind = Some(StorageKind::Cloud);
                                locked.error = None;
                            }),
                            strong { "Многопользовательское" }
                            span { "Общее облачное хранилище в Storj" }
                        }
                    }
                    if locked.storage_kind == Some(StorageKind::Cloud) {
                        super::cloud_setup::CloudSetupView {}
                    }
                }
                if cloud_password {
                    p { class: "notice", "Профиль и ключи будут сохранены на этом устройстве. Подключение к Storj ещё не выполняется." }
                    button { r#type: "button", class: "setup-button", disabled: busy, onclick: move |_| edit_locked(state, |locked| locked.edit_cloud_connection()), "Назад к подключению" }
                }
                if ready {
                    form { class: "unlock-form", onsubmit: submit,
                        label { class: "sr-only", r#for: "master-password", "Мастер-пароль" }
                        div { class: "unlock-input",
                            input { id: "master-password", r#type: "password", autocomplete: "off", autofocus: true,
                                placeholder: if create { "Мастер-пароль" } else { "Введите мастер-пароль" },
                                disabled: busy, value: locked.password.as_str(),
                                oninput: move |event| {
                                    let value = Zeroizing::new(event.value());
                                    edit_locked(state, |locked| replace_secret(&mut locked.password, &value));
                                },
                            }
                            if !create {
                                IconButton { class: "unlock-submit quiet", kind: "submit", icon: Icon::ArrowRight, label: "Разблокировать", disabled: busy }
                            }
                        }
                        if create {
                            label { class: "sr-only", r#for: "repeat-password", "Повторите мастер-пароль" }
                            div { class: "unlock-input",
                                input { id: "repeat-password", r#type: "password", autocomplete: "off", disabled: busy,
                                    placeholder: "Повторите пароль", value: locked.repeat.as_str(),
                                    oninput: move |event| {
                                        let value = Zeroizing::new(event.value());
                                        edit_locked(state, |locked| replace_secret(&mut locked.repeat, &value));
                                    },
                                }
                                if !cloud_password {
                                    IconButton { class: "unlock-submit quiet", kind: "submit", icon: Icon::ArrowRight, label: "Создать хранилище", disabled: busy }
                                }
                            }
                        }
                        if cloud_password {
                            p { class: "notice", "Отдельный пароль восстановления защищает корневой ключ пользователя. Сохраните его в надёжном месте." }
                            label { r#for: "recovery-password", "Пароль восстановления" }
                            div { class: "unlock-input",
                                input { id: "recovery-password", r#type: "password", autocomplete: "off", disabled: busy,
                                    value: locked.recovery_password.as_str(),
                                    oninput: move |event| {
                                        let value = Zeroizing::new(event.value());
                                        edit_locked(state, |locked| replace_secret(&mut locked.recovery_password, &value));
                                    },
                                }
                            }
                            label { r#for: "recovery-repeat", "Повторите пароль восстановления" }
                            div { class: "unlock-input",
                                input { id: "recovery-repeat", r#type: "password", autocomplete: "off", disabled: busy,
                                    value: locked.recovery_repeat.as_str(),
                                    oninput: move |event| {
                                        let value = Zeroizing::new(event.value());
                                        edit_locked(state, |locked| replace_secret(&mut locked.recovery_repeat, &value));
                                    },
                                }
                            }
                            button { r#type: "submit", class: "setup-button", disabled: busy, "Создать профиль и ключи…" }
                        }
                    }
                }
                if weak_password || locked.notice.is_some() || busy || locked.error.is_some() {
                    div { class: "unlock-feedback", aria_live: "polite",
                        if weak_password { p { class: "notice", "Пароль короче {MIN_PASSWORD_CHARS} символов — его проще подобрать" } }
                        if let Some(notice) = &locked.notice { p { class: "notice", role: "status", "{notice}" } }
                        if busy {
                            p { role: "status", class: "muted",
                                if locked.waiting_touch_id { "Жду Touch ID…" }
                                else if create { "Создаю хранилище…" }
                                else { "Проверяю пароль…" }
                            }
                        }
                        if let Some(error) = &locked.error { p { class: "error", role: "alert", "{error}" } }
                    }
                }
            }
            nav { class: "lock-actions", aria_label: "Действия с хранилищем",
                IconButton { class: "quiet", icon: Icon::Folder, label: "Открыть хранилище…", disabled: busy,
                    onclick: move |_| runtime::pick_vault(false),
                }
                IconButton { class: "quiet", icon: Icon::FolderPlus, label: "Создать хранилище…", disabled: busy,
                    onclick: move |_| runtime::begin_create(),
                }
                if settings.read().config.last_vault.is_some() {
                    IconButton { class: "quiet", icon: Icon::FolderX, label: "Забыть путь", disabled: busy,
                        onclick: move |_| runtime::forget_path(),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/locked.rs"]
mod tests;
