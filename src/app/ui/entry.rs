use super::icons::EntryIcon;
use crate::app::context::{Clock, Session, edit_unlocked};
use crate::{
    app::{runtime, security::CopyField, session::Screen},
    icons, totp,
};
use dioxus::prelude::*;

#[component]
pub(super) fn EntryView() -> Element {
    let state = use_context::<Session>();
    let now = (use_context::<Clock>().0)();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let Some(entry) = unlocked.selected_entry() else {
        return rsx! {};
    };
    // Never capture an Entry or secret in an event handler; resolve it at click time.
    let password = if unlocked.show_password {
        entry.password.as_str()
    } else {
        "••••••••"
    };
    rsx! {
        article { class: "entry-detail",
            div { class: "detail-heading",
                EntryIcon { title: entry.title.clone(), slug: icons::catalog().for_entry(entry).map(|i| i.slug.clone()), large: true }
                h1 { "{entry.title}" }
            }
            dl { class: "fields",
                dt { "Логин" }
                dd { span { "{entry.username}" }
                    if !entry.username.is_empty() { button { onclick: move |_| runtime::copy(CopyField::Username), "Скопировать" } }
                }
                dt { "Пароль" }
                dd { code { "{password}" }
                    button { onclick: move |_| edit_unlocked(state, |u| u.show_password = !u.show_password),
                        if unlocked.show_password { "Скрыть" } else { "Показать" }
                    }
                    if !entry.password.is_empty() { button { onclick: move |_| runtime::copy(CopyField::Password), "Скопировать" } }
                }
                dt { "URL" } dd { "{entry.url}" }
                if let Some(settings) = &entry.totp {
                    dt { "Код TOTP" }
                    dd {
                        match totp::code(settings, now) {
                            Ok(code) => rsx! {
                                code { class: "totp-code", "{totp::group(&code)}" }
                                progress { max: "{settings.period}", value: "{totp::seconds_left(settings, now)}" }
                                span { class: "muted", "{totp::seconds_left(settings, now)} с" }
                                button { onclick: move |_| runtime::copy(CopyField::Totp), "Скопировать" }
                            },
                            Err(error) => rsx! { span { class: "error", "{error}" } },
                        }
                    }
                }
                dt { "Заметка" } dd { class: "notes", "{entry.notes}" }
            }
            if unlocked.confirm_delete {
                div { class: "delete-confirm", role: "alert",
                    p { "Удалить запись «{entry.title}»? Это нельзя отменить." }
                    div { class: "actions",
                        button { class: "danger", onclick: move |_| edit_unlocked(state, |u| u.error = u.delete_selected().err()), "Удалить" }
                        button { onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = false), "Отмена" }
                    }
                }
            } else {
                div { class: "actions",
                    button { class: "primary", onclick: move |_| edit_unlocked(state, |u| u.start_edit()), "Изменить" }
                    button { class: "danger quiet", onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = true), "Удалить" }
                }
            }
        }
    }
}
