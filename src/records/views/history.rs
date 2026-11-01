//! Record audit view and reversible deletion browser.
use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        i18n::Text,
        session::Screen,
        widgets::{button::IconButton, symbol::Icon},
        workspace::Page,
    },
    records::{entry::Entry, history::RecordChange},
};
use dioxus::prelude::*;
use std::borrow::Cow;

enum Records {
    History(Vec<RecordChange>),
    Trash(Vec<Entry>),
}

#[component]
pub(crate) fn RecordsView() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let mut revealed = use_signal(std::collections::BTreeSet::<String>::new);
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let trash = u.page == Page::Trash;
    let records = if trash {
        u.vault.trash().map(Records::Trash)
    } else {
        u.vault
            .record_history_for(u.selected.as_deref())
            .map(Records::History)
    };
    let records = match records {
        Ok(records) => records,
        Err(error) => return rsx! { p { class: "error", role: "alert", "{error}" } },
    };
    rsx! {
        article { class: "entry-detail record-history",
            div { class: "detail-heading",
                IconButton { class: "quiet", icon: Icon::ArrowLeft, label: language.text(Text::BackToVault),
                    onclick: move |_| edit_unlocked(state, |u| u.page = Page::Vault),
                }
                h1 { {language.text(if trash { Text::Trash } else { Text::RecordHistory })} }
            }
            match records {
                Records::Trash(entries) => rsx! {
                    if entries.is_empty() { p { class: "muted", {language.text(Text::EmptyTrash)} } }
                    for entry in entries {
                        section { class: "history-change", key: "{entry.id}",
                            h2 { "{entry.title}" }
                            p { "{entry.username}" }
                            button { disabled: u.is_editing() || u.vault.signing_error().is_some(),
                                onclick: { let id = entry.id.clone(); move |_| edit_unlocked(state, |u| u.restore_record(&id)) },
                                {language.text(Text::RestoreRecord)}
                            }
                            button { class: "quiet", onclick: { let id = entry.id.clone(); move |_| edit_unlocked(state, |u| { u.selected = Some(id.clone()); u.open_records_page(Page::History); }) }, {language.text(Text::RecordHistory)} }
                        }
                    }
                },
                Records::History(history) => rsx! {
                    if let Some(entry) = u.selected_entry() { h2 { "{entry.title}" } }
                    p { class: "muted", {language.text(Text::HistoryHint)} }
                    if u.vault.sync_state().is_none() { p { class: "muted", {language.text(Text::LocalHistoryHint)} } }
                    if history.is_empty() { p { {language.text(Text::NoRecordHistory)} } }
                    for change in history.iter().rev() {
                        section { class: "history-change", key: "{change.revision}",
                            h2 { {language.text(match change.action.as_str() { "Delete" => Text::Deleted, "Restore" => Text::HistoryRestored, "Resolve" => Text::HistoryResolved, _ if change.before.is_empty() => Text::HistoryCreated, _ => Text::HistoryUpdated })} }
                            p { {language.format(Text::HistoryAuthor, std::slice::from_ref(&change.actor))} }
                            p { class: "muted fingerprint", {language.format(Text::Device, std::slice::from_ref(&change.device))} }
                            code { class: "fingerprint", "{change.revision}" }
                            if let Some(at) = change.at { p { class: "muted", {language.format(Text::HistoryTime, &[crate::desktop::time::utc_timestamp(at)])} } }
                            button { class: "quiet", onclick: { let revision = change.revision.clone(); move |_| { let mut values = revealed.write(); if !values.remove(&revision) { values.insert(revision.clone()); } } }, {language.text(Text::HistorySecrets)} }
                            for before in &change.before {
                                div { class: "history-diff",
                                    div { h3 { {language.text(Text::HistoryBefore)} } {snapshot(language, before, change.after.as_ref(), revealed.read().contains(&change.revision))} }
                                    div { h3 { {language.text(Text::HistoryAfter)} } if let Some(after) = &change.after { {snapshot(language, after, Some(before), revealed.read().contains(&change.revision))} } else { p { {language.text(Text::Deleted)} } } }
                                }
                            }
                            if change.before.is_empty() {
                                if let Some(after) = &change.after { {snapshot(language, after, None, revealed.read().contains(&change.revision))} }
                                else { p { {language.text(Text::Deleted)} } }
                            }
                        }
                    }
                },
            }
        }
    }
}

fn snapshot(
    language: crate::settings::language::Language,
    entry: &Entry,
    other: Option<&Entry>,
    reveal: bool,
) -> Element {
    let mut fields = Vec::new();
    for (label, value, previous, secret) in [
        (
            Text::Name,
            entry.title.as_str(),
            other.map(|e| e.title.as_str()),
            false,
        ),
        (
            Text::Username2,
            entry.username.as_str(),
            other.map(|e| e.username.as_str()),
            false,
        ),
        (
            Text::Password,
            entry.password.as_str(),
            other.map(|e| e.password.as_str()),
            true,
        ),
        (
            Text::Website,
            entry.url.as_str(),
            other.map(|e| e.url.as_str()),
            false,
        ),
        (
            Text::Note,
            entry.notes.as_str(),
            other.map(|e| e.notes.as_str()),
            false,
        ),
    ] {
        if previous != Some(value) {
            fields.push((
                language.text(label),
                if secret && !reveal {
                    Cow::Borrowed("••••••••")
                } else {
                    Cow::Borrowed(value)
                },
            ));
        }
    }
    if other.is_none_or(|e| e.totp != entry.totp) {
        fields.push((
            "TOTP",
            if reveal {
                Cow::Owned(
                    entry
                        .totp
                        .as_ref()
                        .map(|t| {
                            format!(
                                "{} · {:?} · {} · {}",
                                t.secret, t.algorithm, t.digits, t.period
                            )
                        })
                        .unwrap_or_default(),
                )
            } else {
                Cow::Borrowed("••••••••")
            },
        ));
    }
    if other.is_none_or(|e| e.folder_id != entry.folder_id) {
        fields.push((
            language.text(Text::Folder),
            Cow::Borrowed(entry.folder_id.as_deref().unwrap_or_default()),
        ));
    }
    if other.is_none_or(|e| e.icon != entry.icon) {
        fields.push((
            language.text(Text::ChooseIcon),
            Cow::Borrowed(entry.icon.as_deref().unwrap_or_default()),
        ));
    }
    let files_changed = other.is_none_or(|e| e.attachments != entry.attachments);
    rsx! {
        dl { class: "history-fields",
            for (label, value) in fields { dt { "{label}" } dd { "{value}" } }
            if files_changed { dt { {language.text(Text::Attachments)} } dd { for file in &entry.attachments { p { "{file.name} ({file.data.len()} B)" } } } }
        }
    }
}
