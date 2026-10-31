use super::button::IconButton;
use super::icons::EntryIcon;
use super::symbol::Icon;
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
                div { p { class: "detail-kind", "Пароль" } h1 { "{entry.title}" } }
            }
            dl { class: "fields",
                div { class: "field-row",
                    dt { "Логин" }
                    dd { span { class: "field-value", "{entry.username}" }
                        if !entry.username.is_empty() {
                            IconButton { class: "quiet", label: "Скопировать логин", icon: Icon::Copy,
                                onclick: move |_| runtime::copy(CopyField::Username),
                            }
                        }
                    }
                }
                div { class: "field-row",
                    dt { "Пароль" }
                    dd { code { class: "field-value password-value", "{password}" }
                        IconButton { class: "quiet", label: if unlocked.show_password { "Скрыть пароль" } else { "Показать пароль" },
                            icon: if unlocked.show_password { Icon::EyeOff } else { Icon::Eye },
                            pressed: Some(unlocked.show_password),
                            onclick: move |_| edit_unlocked(state, |u| u.show_password = !u.show_password),
                        }
                        if !entry.password.is_empty() {
                            IconButton { class: "quiet", label: "Скопировать пароль", icon: Icon::Copy,
                                onclick: move |_| runtime::copy(CopyField::Password),
                            }
                        }
                    }
                }
                div { class: "field-row", dt { "Веб-сайт" } dd { span { class: "field-value website-value", "{entry.url}" } } }
                if let Some(settings) = &entry.totp {
                    div { class: "field-row totp-field",
                        dt { "Код TOTP" }
                        dd {
                            match totp::code(settings, now) {
                                Ok(code) => rsx! {
                                    div { class: "field-value totp-value",
                                        code { class: "totp-code", "{totp::group(&code)}" }
                                        div { class: "totp-countdown",
                                            progress { aria_label: "Время действия кода", max: "{settings.period}", value: "{totp::seconds_left(settings, now)}" }
                                            span { class: "muted", "{totp::seconds_left(settings, now)} с" }
                                        }
                                    }
                                    IconButton { class: "quiet", label: "Скопировать код TOTP", icon: Icon::Copy,
                                        onclick: move |_| runtime::copy(CopyField::Totp),
                                    }
                                },
                                Err(error) => rsx! { span { class: "error", "{error}" } },
                            }
                        }
                    }
                }
                if !entry.notes.is_empty() {
                    div { class: "field-row notes-row", dt { "Заметка" } dd { class: "notes", "{entry.notes}" } }
                }
            }
            if unlocked.confirm_delete {
                div { class: "delete-confirm", role: "alert",
                    p { "Удалить запись «{entry.title}»? Это нельзя отменить." }
                    div { class: "actions",
                        IconButton { class: "danger", icon: Icon::Trash, label: "Удалить запись", onclick: move |_| edit_unlocked(state, |u| u.error = u.delete_selected().err()) }
                        IconButton { icon: Icon::Close, label: "Отмена", onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = false) }
                    }
                }
            } else {
                div { class: "actions entry-actions",
                    IconButton { icon: Icon::Edit, label: "Изменить", onclick: move |_| edit_unlocked(state, |u| u.start_edit()) }
                    IconButton { class: "danger quiet", icon: Icon::Trash, label: "Удалить", onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = true) }
                }
            }
        }
    }
}
