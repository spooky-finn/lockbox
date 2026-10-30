use crate::app::context::{BiometricProvider, Session, edit_locked};
use crate::app::{
    auth::{MIN_PASSWORD_CHARS, Mode},
    editor::replace_secret,
    runtime,
    session::Screen,
    settings::Settings,
};
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
    let ready = locked
        .path
        .as_ref()
        .is_some_and(|path| create || path.exists());
    let filename = locked
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "Хранилище не выбрано".into());
    let submit = move |event: FormEvent| {
        event.prevent_default();
        edit_locked(state, |locked| {
            if locked.pending.is_none() {
                locked.submit();
            }
        })
    };
    rsx! {
        section { class: "lock-screen",
            div { class: "lock-card",
                div { class: "wordmark", "lockbox" }
                p { class: "muted", "Локальное хранилище паролей" }
                if let Some(notice) = &locked.notice { p { class: "notice", role: "status", "{notice}" } }
                p { class: "vault-path", "{filename}" }
                div { class: "actions",
                    button { disabled: busy, onclick: move |_| runtime::pick_vault(false), "Открыть хранилище…" }
                    button { disabled: busy, onclick: move |_| runtime::pick_vault(true), "Создать хранилище…" }
                    if settings.read().config.last_vault.is_some() {
                        button { class: "quiet", disabled: busy,
                            onclick: move |_| runtime::forget_path(), "Забыть путь"
                        }
                    }
                }
                if ready {
                    form { class: "stack", onsubmit: submit,
                        label { r#for: "master-password", "Мастер-пароль" }
                        input {
                            id: "master-password", r#type: "password", autocomplete: "off", autofocus: true,
                            disabled: busy, value: locked.password.as_str(),
                            oninput: move |event| { let value = Zeroizing::new(event.value()); edit_locked(state, |locked| replace_secret(&mut locked.password, &value)); },
                        }
                        if create {
                            label { r#for: "repeat-password", "Повторите мастер-пароль" }
                            input {
                                id: "repeat-password", r#type: "password", autocomplete: "off", disabled: busy,
                                value: locked.repeat.as_str(),
                                oninput: move |event| { let value = Zeroizing::new(event.value()); edit_locked(state, |locked| replace_secret(&mut locked.repeat, &value)); },
                            }
                            if !locked.password.is_empty() && locked.password.chars().count() < MIN_PASSWORD_CHARS {
                                p { class: "notice", "Пароль короче {MIN_PASSWORD_CHARS} символов — его проще подобрать" }
                            }
                        }
                        button { class: "primary", r#type: "submit", disabled: busy,
                            if create { "Создать" } else { "Открыть" }
                        }
                    }
                    if !create && locked.touch_id.is_some() {
                        button { class: "touch-id", disabled: busy,
                            onclick: move |_| {
                                let biometrics = hardware.0.clone();
                                edit_locked(state, |locked| locked.start_touch_id(biometrics));
                            }, "Открыть по Touch ID"
                        }
                    }
                }
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
}
