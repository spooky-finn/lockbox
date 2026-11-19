use crate::desktop::context::navigate;
use crate::desktop::i18n::Text;
use crate::records::dialogs;

use crate::{
    desktop::{
        context::{Clock, Session, edit_unlocked},
        runtime,
        security::CopyField,
        session::Screen,
        widgets::{button::IconButton, icons::EntryIcon, symbol::Icon},
    },
    records::{editor::Field, totp},
};
use dioxus::prelude::*;

#[component]
pub(crate) fn EntryView() -> Element {
    #[cfg(test)]
    ENTRY_RENDERS.set(ENTRY_RENDERS.get() + 1);
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let pending_copy = use_hook(|| CopyValue::new(std::cell::Cell::new(0_u64)));
    let copied = use_signal(|| None::<(String, CopyField, std::time::Instant)>);
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
    let is_copied = |field| {
        copied.read().as_ref().is_some_and(|(id, last, at)| {
            id == &entry.id && *last == field && at.elapsed().as_secs_f32() < 2.0
        })
    };
    let title_copied = is_copied(CopyField::Title);
    let notes_copied = is_copied(CopyField::Notes);
    let username_copied = is_copied(CopyField::Username);
    let password_copied = is_copied(CopyField::Password);
    let url_copied = is_copied(CopyField::Url);
    // Only web links may leave the app through the desktop's browser handler.
    let browser_url = url::Url::parse(entry.url.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    rsx! {
        article { class: "entry-detail",
            div { class: "detail-heading", title: language.text(Text::RecordReadHint),
                span { class: "sr-only", {language.text(Text::RecordReadHint)} }
                EntryIcon { title: entry.title.clone(), slug: entry.icon.clone(), url: entry.url.clone(), large: true }
                div { class: "detail-title", p { class: "detail-kind", {language.text(Text::Password)} } h1 { button { class: "quiet copy-value title-value", aria_label: language.text(Text::CopyTitle), onclick: move |_| queue_copy(state, copied, pending_copy, CopyField::Title), ondoubleclick: move |_| { cancel_copy(pending_copy); edit_unlocked(state, |u| u.edit_field(Field::Title)); }, "{entry.title}" } } if title_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } } }
                if !unlocked.confirm_delete {
                    div { class: "actions entry-actions",
                        IconButton { class: "quiet", icon: Icon::Clock, label: language.text(Text::RecordHistory), onclick: move |_| edit_unlocked(state, |u| u.open_records_page(crate::desktop::workspace::Page::History)) }
                        IconButton { class: "quiet", icon: Icon::Edit, label: language.text(Text::Edit), disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.start_edit()) }
                        IconButton { class: "danger quiet", icon: Icon::Trash, label: language.text(Text::Delete), disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = true) }
                    }
                }
                IconButton { class: "quiet entry-dismiss", icon: Icon::Close, label: language.text(Text::CloseEditor), onclick: move |_| navigate(state, crate::desktop::workspace::Destination::Close) }
            }
            dl { class: "fields",
                div { class: "field-row",
                    dt { {language.text(Text::Username2)} }
                    dd { span { class: "field-value copy-value-group", button { class: "quiet copy-value", ondoubleclick: move |_| { cancel_copy(pending_copy); edit_unlocked(state, |u| u.edit_field(Field::Username)); }, aria_label: language.text(Text::CopyUsername), onclick: move |_| queue_copy(state, copied, pending_copy, CopyField::Username), if entry.username.is_empty() { span { class: "muted", "—" } } else { "{entry.username}" } } if username_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } } }
                        if !entry.username.is_empty() {
                            IconButton { class: "quiet field-copy", label: language.text(Text::CopyUsername), icon: Icon::Copy,
                                onclick: move |_| copy_value(state, copied, CopyField::Username),
                            }
                        }
                    }
                }
                div { class: "field-row",
                    dt { {language.text(Text::Password)} }
                    dd { span { class: "field-value copy-value-group", button { class: "quiet copy-value", ondoubleclick: move |_| { cancel_copy(pending_copy); edit_unlocked(state, |u| u.edit_field(Field::Password)); }, aria_label: language.text(Text::CopyPassword), onclick: move |_| queue_copy(state, copied, pending_copy, CopyField::Password), code { class: "password-value", "{password}" } } if password_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } } }
                        IconButton { class: "quiet", label: if unlocked.show_password { language.text(Text::HidePassword) } else { language.text(Text::ShowPassword) },
                            icon: if unlocked.show_password { Icon::EyeOff } else { Icon::Eye },
                            pressed: Some(unlocked.show_password),
                            onclick: move |_| edit_unlocked(state, |u| u.show_password = !u.show_password),
                        }

                        if !entry.password.is_empty() {
                            IconButton { class: "quiet field-copy", label: language.text(Text::CopyPassword), icon: Icon::Copy,
                                onclick: move |_| copy_value(state, copied, CopyField::Password),
                            }
                        }
                    }
                }
                div { class: "field-row",
                    dt { {language.text(Text::Url)} }
                    dd {
                        span { class: "field-value copy-value-group",
                            if let Some(url) = browser_url {
                                // Dioxus Desktop opens external anchors in the default browser.
                                a { class: "url-value", href: url.as_str(), rel: "noopener noreferrer", "{entry.url}" }
                            } else if entry.url.is_empty() {
                                span { class: "muted", "—" }
                            } else {
                                span { class: "url-value", "{entry.url}" }
                            }
                            if url_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } }
                        }
                        if !entry.url.is_empty() {
                            IconButton { class: "quiet field-copy", label: language.text(Text::CopyUrl), icon: Icon::Copy,
                                onclick: move |_| copy_value(state, copied, CopyField::Url),
                            }
                        }
                    }
                }
                if entry.totp.is_some() { TotpField {} }

                div { class: "field-row notes-row",
                    dt { {language.text(Text::Note)} }
                    dd { class: "notes",
                        span { class: "field-value notes-value",
                            if entry.notes.is_empty() { span { class: "muted", "—" } }
                            else { "{entry.notes}" }
                        }
                        if !entry.notes.is_empty() {
                            IconButton { class: "quiet field-copy", label: language.text(Text::CopyNotes), icon: Icon::Copy,
                                onclick: move |_| copy_value(state, copied, CopyField::Notes),
                            }
                        }
                        if notes_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } }
                    }
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
                                onclick: { let id = file.id.clone(); move |_| dialogs::export_attachment(&id) },
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

/// The one-second clock belongs to this row, not the complete record card.
#[component]
fn TotpField() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let now = (use_context::<Clock>().0)();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let Some(settings) = u.selected_entry().and_then(|e| e.totp.as_ref()) else {
        return rsx! {};
    };
    rsx! {
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
                        IconButton { class: "quiet field-copy", label: language.text(Text::CopyTOTPCode), icon: Icon::Copy,
                            onclick: move |_| { runtime::copy(CopyField::Totp); },
                        }
                    },
                    Err(error) => rsx! { span { class: "error", {language.message(&error.to_string())} } },
                }
            }
        }
    }
}

fn cancel_copy(pending: CopyValue<std::cell::Cell<u64>>) {
    let ticket = pending.read();
    ticket.set(ticket.get().wrapping_add(1));
}

// A double click cancels the pending single-click clipboard action.
fn queue_copy(
    state: Session,
    copied: Signal<Option<(String, CopyField, std::time::Instant)>>,
    pending: CopyValue<std::cell::Cell<u64>>,
    field: CopyField,
) {
    cancel_copy(pending);
    let ticket = pending.read().get();
    let id = match &*state.peek() {
        Screen::Unlocked(u) => u.selected.clone(),
        _ => None,
    };
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if pending.read().get() == ticket
            && matches!(&*state.peek(), Screen::Unlocked(u) if u.selected == id && u.draft.is_none())
        {
            copy_value(state, copied, field);
        }
    });
}

// Store only record identity and timing; resolve clipboard contents at click time.
fn copy_value(
    state: Session,
    mut copied: Signal<Option<(String, CopyField, std::time::Instant)>>,
    field: CopyField,
) {
    if runtime::copy(field) {
        if let Screen::Unlocked(workspace) = &*state.read()
            && let Some(entry) = workspace.selected_entry()
        {
            let marker = (entry.id.clone(), field, std::time::Instant::now());
            copied.set(Some(marker.clone()));
            spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if copied.peek().as_ref() == Some(&marker) {
                    copied.set(None);
                }
            });
        }
    } else {
        copied.set(None);
    }
}

#[cfg(test)]
#[path = "tests/entry.rs"]
mod tests;

#[cfg(test)]
thread_local! {
    pub(crate) static ENTRY_RENDERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
