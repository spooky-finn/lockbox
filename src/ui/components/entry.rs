use crate::ui::i18n::Text;

use super::button::IconButton;
use super::icons::EntryIcon;
use super::symbol::Icon;
use crate::ui::context::{Clock, Session, edit_unlocked};
use crate::ui::runtime;
use crate::{
    app::{security::CopyField, session::Screen},
    core::totp,
    ui::icons,
};
use dioxus::prelude::*;

#[component]
pub(super) fn EntryView() -> Element {
    let language = crate::ui::i18n::use_language();
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
                div { class: "detail-title", p { class: "detail-kind", {language.text(Text::Password)} } h1 { "{entry.title}" } }
                if !unlocked.confirm_delete {
                    div { class: "actions entry-actions",
                        button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| u.open_records_page(crate::app::workspace::Page::History)), {language.text(Text::RecordHistory)} }
                        IconButton { class: "quiet", icon: Icon::Edit, label: language.text(Text::Edit), disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.start_edit()) }
                        IconButton { class: "danger quiet", icon: Icon::Trash, label: language.text(Text::Delete), disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = true) }
                    }
                }
            }
            dl { class: "fields",
                div { class: "field-row",
                    dt { {language.text(Text::Username2)} }
                    dd { span { class: "field-value", "{entry.username}" }
                        if !entry.username.is_empty() {
                            IconButton { class: "quiet", label: language.text(Text::CopyUsername), icon: Icon::Copy,
                                onclick: move |_| runtime::copy(CopyField::Username),
                            }
                        }
                    }
                }
                div { class: "field-row",
                    dt { {language.text(Text::Password)} }
                    dd { code { class: "field-value password-value", "{password}" }
                        IconButton { class: "quiet", label: if unlocked.show_password { language.text(Text::HidePassword) } else { language.text(Text::ShowPassword) },
                            icon: if unlocked.show_password { Icon::EyeOff } else { Icon::Eye },
                            pressed: Some(unlocked.show_password),
                            onclick: move |_| edit_unlocked(state, |u| u.show_password = !u.show_password),
                        }

                        if !entry.password.is_empty() {
                            IconButton { class: "quiet", label: language.text(Text::CopyPassword), icon: Icon::Copy,
                                onclick: move |_| runtime::copy(CopyField::Password),
                            }
                        }
                    }
                }
                div { class: "field-row", dt { {language.text(Text::Website)} } dd { span { class: "field-value website-value", "{entry.url}" } } }
                if let Some(settings) = &entry.totp {
                    div { class: "field-row totp-field",
                        dt { {language.text(Text::TOTPCode)} }
                        dd {
                            match totp::code(settings, now) {
                                Ok(code) => rsx! {
                                    div { class: "field-value totp-value",
                                        code { class: "totp-code", "{totp::group(&code)}" }
                                        div { class: "totp-countdown",
                                            progress { aria_label: language.text(Text::CodeValidity), max: "{settings.period}", value: "{totp::seconds_left(settings, now)}" }
                                            span { class: "muted", {language.format(Text::S, &[totp::seconds_left(settings, now).to_string()])} }
                                        }
                                    }
                                    IconButton { class: "quiet", label: language.text(Text::CopyTOTPCode), icon: Icon::Copy,
                                        onclick: move |_| runtime::copy(CopyField::Totp),
                                    }
                                },
                                Err(error) => rsx! { span { class: "error", {language.message(&error.to_string())} } },
                            }
                        }
                    }
                }

                if !entry.notes.is_empty() {
                    div { class: "field-row notes-row", dt { {language.text(Text::Note)} } dd { class: "notes", "{entry.notes}" } }
                }
            }

            if !entry.attachments.is_empty() {
                section { class: "entry-attachments",
                    h2 { {language.text(Text::Attachments)} }
                    for file in &entry.attachments {
                        div { class: "attachment-row", key: "{file.id}",
                            span { class: "attachment-name", "{file.name}" }
                            span { class: "attachment-size", "{file.data.len()} B" }
                            IconButton { class: "quiet", icon: Icon::Save, label: language.text(Text::SaveAttachment),
                                onclick: { let id = file.id.clone(); move |_| runtime::export_attachment(&id) },
                            }
                        }
                    }
                }
            }

            if unlocked.confirm_delete {
                div { class: "delete-confirm", role: "alert",
                    p { {language.format(Text::MoveEntryToTrash, &[entry.title.to_string()])} }
                    div { class: "actions",
                        IconButton { class: "danger", icon: Icon::Trash, label: language.text(Text::DeleteEntry), disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.error = u.delete_selected().err()) }
                        IconButton { icon: Icon::Close, label: language.text(Text::Cancel), onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = false) }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/entry.rs"]
mod tests;
