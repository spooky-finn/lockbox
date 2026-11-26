//! Record audit view and reversible deletion browser.
use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        i18n::Text,
        session::Screen,
        widgets::{button::IconButton, identifier::Identifier, symbol::Icon},
        workspace::Page,
    },
    records::{entry::Entry, history::RecordChange},
};
use dioxus::prelude::*;
use std::borrow::Cow;

mod diff;
use diff::{Kind, lines};

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
        article { class: if trash { "entry-detail" } else { "entry-detail record-history" },
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
                    div { class: "history-intro",
                        if let Some(entry) = u.selected_entry() { h2 { "{entry.title}" } }
                        p { class: "muted",
                            {language.text(Text::HistoryHint)}
                            if u.vault.sync_state().is_none() { " " {language.text(Text::LocalHistoryHint)} }
                        }
                    }
                    if history.is_empty() { p { {language.text(Text::NoRecordHistory)} } }
                    for change in history.iter().rev() {
                        section { class: "history-change", key: "{change.revision}",
                            header { class: "history-revision-heading",
                                h2 { {language.text(match change.action.as_str() { "Delete" => Text::Deleted, "Restore" => Text::HistoryRestored, "Resolve" => Text::HistoryResolved, _ if change.before.is_empty() => Text::HistoryCreated, _ => Text::HistoryUpdated })} }
                                p { class: "muted history-time",
                                    if let Some(at) = change.at {
                                        {language.format(if u.vault.sync_state().is_some() { Text::HistoryAuthorTime } else { Text::HistoryTime }, &[crate::desktop::time::utc_timestamp(at)])}
                                    } else {
                                        {language.text(Text::HistoryTimeUnavailable)}
                                    }
                                }
                                button { class: "quiet history-secrets", r#type: "button",
                                    aria_pressed: revealed.read().contains(&change.revision).to_string(),
                                    onclick: { let revision = change.revision.clone(); move |_| { let mut values = revealed.write(); if !values.remove(&revision) { values.insert(revision.clone()); } } },
                                    {language.text(Text::HistorySecrets)}
                                }
                            }
                            div { class: "muted history-meta",
                                p {
                                    if let Ok(actor) = change.actor.parse() {
                                        {language.format(Text::HistoryAuthor, &[String::new()])}
                                        if let Some(name) = u.vault.participant_name(actor) {
                                            strong { {crate::sync::protocol::pairing::display_name(name)} }
                                        }
                                        Identifier { value: change.actor.clone() }
                                    } else {
                                        {language.format(Text::HistoryAuthor, std::slice::from_ref(&change.actor))}
                                    }
                                }
                                p {
                                    if change.device.parse::<crate::sync::protocol::Hash>().is_ok() {
                                        {language.format(Text::Device, &[String::new()])}
                                        Identifier { value: change.device.clone() }
                                    } else {
                                        {language.format(Text::Device, std::slice::from_ref(&change.device))}
                                    }
                                }
                                Identifier { value: change.revision.clone() }
                            }
                            for before in &change.before {
                                {record_diff(language, Some(before), change.after.as_ref(), revealed.read().contains(&change.revision))}
                            }
                            if change.before.is_empty() {
                                {record_diff(language, None, change.after.as_ref(), revealed.read().contains(&change.revision))}
                            }
                        }
                    }
                },
            }
        }
    }
}

fn record_diff(
    language: crate::settings::language::Language,
    before: Option<&Entry>,
    after: Option<&Entry>,
    reveal: bool,
) -> Element {
    let empty = Entry::default();
    let old = before.unwrap_or(&empty);
    let new = after.unwrap_or(&empty);
    let mut fields = Vec::new();
    for (label, previous, value, secret) in [
        (Text::Name, old.title.as_str(), new.title.as_str(), false),
        (
            Text::Username2,
            old.username.as_str(),
            new.username.as_str(),
            false,
        ),
        (
            Text::Password,
            old.password.as_str(),
            new.password.as_str(),
            true,
        ),
        (Text::Url, old.url.as_str(), new.url.as_str(), false),
        (Text::Note, old.notes.as_str(), new.notes.as_str(), false),
        (
            Text::Folder,
            old.folder_id.as_deref().unwrap_or_default(),
            new.folder_id.as_deref().unwrap_or_default(),
            false,
        ),
        (
            Text::ChooseIcon,
            old.icon.as_deref().unwrap_or_default(),
            new.icon.as_deref().unwrap_or_default(),
            false,
        ),
    ] {
        if previous != value {
            fields.push(field_diff(
                language.text(label),
                masked(previous, secret && !reveal),
                masked(value, secret && !reveal),
                secret && !reveal,
            ));
        }
    }
    if old.totp != new.totp {
        let display = |entry: &Entry| match &entry.totp {
            None => Cow::Borrowed(""),
            Some(_) if !reveal => Cow::Borrowed("••••••••"),
            Some(t) => Cow::Owned(format!(
                "{} · {:?} · {} · {}",
                t.secret, t.algorithm, t.digits, t.period
            )),
        };
        fields.push(field_diff("TOTP", &display(old), &display(new), !reveal));
    }
    if old.attachments != new.attachments {
        // Compare the files themselves: replacing bytes can leave names and sizes unchanged.
        let removed = old
            .attachments
            .iter()
            .filter(|file| !new.attachments.contains(file))
            .collect::<Vec<_>>();
        let added = new
            .attachments
            .iter()
            .filter(|file| !old.attachments.contains(file))
            .collect::<Vec<_>>();
        let display = |files: Vec<&crate::records::entry::Attachment>| {
            files
                .iter()
                .map(|file| format!("{} ({} B)", file.name, file.data.len()))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let reordered = removed.is_empty() && added.is_empty();
        fields.push(field_diff(
            language.text(Text::Attachments),
            &display(if reordered {
                old.attachments.iter().collect()
            } else {
                removed
            }),
            &display(if reordered {
                new.attachments.iter().collect()
            } else {
                added
            }),
            true,
        ));
    }
    rsx! { div { class: "history-diff", {fields.into_iter()} } }
}

fn masked(text: &str, hide: bool) -> &str {
    if hide && !text.is_empty() {
        "••••••••"
    } else {
        text
    }
}

fn field_diff(label: &str, before: &str, after: &str, replace: bool) -> Element {
    let rows = lines(before, after, replace);
    rsx! {
        section { class: "history-diff-field",
            h3 { "{label}" }
            div { class: "history-diff-lines",
                for row in rows {
                    div { class: match row.kind {
                        Kind::Equal => "diff-line diff-context",
                        Kind::Removed => "diff-line diff-removed",
                        Kind::Added => "diff-line diff-added",
                    },
                        span { class: "diff-number", "{row.old.map(|n| n.to_string()).unwrap_or_default()}" }
                        span { class: "diff-number", "{row.new.map(|n| n.to_string()).unwrap_or_default()}" }
                        span { class: "diff-sign", {match row.kind { Kind::Equal => " ", Kind::Removed => "−", Kind::Added => "+" }} }
                        code { class: "diff-text", {row.text.trim_end_matches('\n')} }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/history.rs"]
mod tests;
