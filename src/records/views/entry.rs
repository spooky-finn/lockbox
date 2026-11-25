use super::field_label::FieldLabel;
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
    let password = if entry.password.is_empty() {
        "—"
    } else if unlocked.show_password {
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
    let username_copied = is_copied(CopyField::Username);
    let password_copied = is_copied(CopyField::Password);
    let url_copied = is_copied(CopyField::Url);
    // Only web links may leave the app through the desktop's browser handler.
    let browser_url = url::Url::parse(entry.url.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    rsx! {
        article { class: "entry-detail record-read",
            div { class: "detail-heading", title: language.text(Text::RecordReadHint),
                span { class: "sr-only", {language.text(Text::RecordReadHint)} }
                EntryIcon { title: entry.title.clone(), slug: entry.icon.clone(), url: entry.url.clone(), large: true }
                div { class: "detail-title", h1 { button { class: "quiet copy-value title-value", aria_label: language.text(Text::CopyTitle), title: language.text(Text::CopyTitle), onclick: move |_| queue_copy(state, copied, pending_copy, CopyField::Title), ondoubleclick: move |_| { cancel_copy(pending_copy); edit_unlocked(state, |u| u.edit_field(Field::Title)); }, "{entry.title}" } } if title_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } } }
                div { class: "actions entry-actions",
                    IconButton { class: "quiet", icon: Icon::Clock, label: language.text(Text::RecordHistory), onclick: move |_| edit_unlocked(state, |u| u.open_records_page(crate::desktop::workspace::Page::History)) }
                    IconButton { class: "quiet", icon: Icon::Edit, label: language.text(Text::Edit), disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.start_edit()) }
                    IconButton { class: "danger quiet entry-trash-trigger", icon: Icon::Trash, label: language.text(Text::Delete), disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.confirm_delete = true) }
                }
            }
            dl { class: "fields",
                div { class: "field-row",
                    FieldLabel { icon: Icon::User, text: language.text(Text::Username2) }
                    dd {
                        button { class: "quiet copy-value field-value", ondoubleclick: move |_| { cancel_copy(pending_copy); edit_unlocked(state, |u| u.edit_field(Field::Username)); }, aria_label: language.text(Text::CopyUsername), title: language.text(Text::CopyUsername), onclick: move |_| queue_copy(state, copied, pending_copy, CopyField::Username),
                            if entry.username.is_empty() { span { class: "muted", "—" } } else { "{entry.username}" }
                        }
                        if username_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } }
                    }
                }
                div { class: "field-row",
                    FieldLabel { icon: Icon::Key, text: language.text(Text::Password) }
                    dd {
                        button { class: "quiet copy-value field-value", ondoubleclick: move |_| { cancel_copy(pending_copy); edit_unlocked(state, |u| u.edit_field(Field::Password)); }, aria_label: language.text(Text::CopyPassword), title: language.text(Text::CopyPassword), onclick: move |_| queue_copy(state, copied, pending_copy, CopyField::Password),
                            code { class: "password-value", "{password}" }
                        }
                        IconButton { class: "quiet", label: if unlocked.show_password { language.text(Text::HidePassword) } else { language.text(Text::ShowPassword) },
                            icon: if unlocked.show_password { Icon::EyeOff } else { Icon::Eye },
                            pressed: Some(unlocked.show_password),
                            onclick: move |_| edit_unlocked(state, |u| u.show_password = !u.show_password),
                        }
                        if password_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } }
                    }
                }
                div { class: "field-row",
                    FieldLabel { icon: Icon::Link, text: language.text(Text::Url), focusable: entry.url.is_empty() }
                    dd {
                        if entry.url.is_empty() {
                            span { class: "muted", "—" }
                        } else {
                            button { class: "quiet copy-value field-value url-value", aria_label: language.text(Text::CopyUrl), title: language.text(Text::CopyUrl),
                                onclick: move |_| copy_value(state, copied, CopyField::Url),
                                "{entry.url}"
                            }
                            if let Some(url) = browser_url {
                                // Dioxus Desktop opens external anchors in the default browser.
                                a { class: "open-record-url", href: url.as_str(), rel: "noopener noreferrer", {language.text(Text::OpenRecordUrl)} }
                            }
                        }
                        if url_copied { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } }
                    }
                }
                if entry.totp.is_some() { TotpField {} }

                div { class: "field-row notes-row",
                    FieldLabel { icon: Icon::Note, text: language.text(Text::Note), focusable: entry.notes.is_empty() }
                    dd { class: "notes",
                        span { class: "field-value notes-value",
                            if entry.notes.is_empty() { span { class: "muted", "—" } }
                            else { "{entry.notes}" }
                        }
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
                dialog { id: "entry-trash-dialog", class: "entry-trash-dialog",
                    aria_labelledby: "entry-trash-prompt",
                    onmounted: move |_| {
                        let _ = document::eval(
                            "document.querySelector('.entry-trash-trigger')?.focus();
                             document.getElementById('entry-trash-dialog')?.showModal();",
                        );
                    },
                    onkeydown: move |event| {
                        if event.key() == Key::Escape {
                            event.stop_propagation();
                        }
                    },
                    oncancel: move |event| {
                        event.prevent_default();
                        finish_trash_confirmation(state, false);
                    },
                    p { id: "entry-trash-prompt", {language.format(Text::MoveEntryToTrash, &[entry.title.to_string()])} }
                    div { class: "actions",
                        button { r#type: "button", autofocus: true, onclick: move |_| finish_trash_confirmation(state, false), {language.text(Text::Cancel)} }
                        button { r#type: "button", class: "danger", disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| finish_trash_confirmation(state, true), {language.text(Text::MoveToTrash)} }
                    }
                }
            }
        }
    }
}

fn finish_trash_confirmation(state: Session, confirm: bool) {
    edit_unlocked(state, |u| {
        if !u.confirm_delete {
            return;
        }
        let _ = document::eval("document.getElementById('entry-trash-dialog')?.close()");
        u.confirm_delete = false;
        if confirm {
            u.error = u.delete_selected().err();
        }
    });
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
    let code = totp::code(settings, now);
    rsx! {
        div { class: "field-row totp-field",
            FieldLabel { icon: Icon::Clock, text: language.text(Text::TOTPCode), focusable: code.is_err() }
            dd {
                match code {
                    Ok(code) => rsx! {
                        div { class: "field-value totp-value",
                            button { class: "quiet copy-value", aria_label: language.text(Text::CopyTOTPCode), title: language.text(Text::CopyTOTPCode),
                                onclick: move |_| { runtime::copy(CopyField::Totp); },
                                code { class: "totp-code", "{totp::group(&code)}" }
                            }
                            span { class: "totp-countdown muted", role: "timer", aria_label: language.text(Text::CodeValidity),
                                {language.format(Text::S, &[totp::seconds_left(settings, now).to_string()])}
                            }
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
