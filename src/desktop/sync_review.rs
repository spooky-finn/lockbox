//! Shared-history review and the visibility of its decrypted contents.

use crate::desktop::{
    context::{Session, edit_unlocked},
    i18n::{Text, use_language},
    session::Screen,
};
use dioxus::prelude::*;

#[component]
pub(super) fn SyncReview() -> Element {
    let language = use_language();
    let mut revealed =
        use_signal(std::collections::BTreeSet::<crate::sync::protocol::Revision>::new);
    let mut revealed_drafts =
        use_signal(std::collections::BTreeSet::<crate::sync::protocol::Hash>::new);
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    rsx! {
        if let Ok(Some(replay)) = unlocked.vault.sync_replay() {
            if !replay.history_verified {
                p { class: "notice", {language.text(Text::TheOwnerConfirmedTheInitialStateThisDeviceHasNotVerifiedEarlierChanges)} }
            }

            for record in replay.projection.conflicts() {
                details { class: "notice",
                    summary { {language.format(Text::ConflictVersions, &[record.entity.id.as_str(), &record.versions.len().to_string()])} }
                    for version in &record.versions {
                        div {
                            p { {language.format(Text::Device, &[version.author.to_string()])} }
                            match &version.value {
                                Some(crate::sync::protocol::Value::Entry(entry)) => rsx! { p { "{entry.title} · {entry.username} · {entry.url}" } },
                                Some(crate::sync::protocol::Value::Folder(folder)) => rsx! { p { "{folder.name}" } },
                                None => rsx! { p { {language.text(Text::Deleted)} } },
                            }
                            button {
                                onclick: { let revision = version.revision; move |_| { let mut selected = revealed.write(); if !selected.remove(&revision) { selected.insert(revision); } } },
                                {language.text(Text::ShowHideContents)}
                            }

                            if revealed.read().contains(&version.revision) {
                                match &version.value {
                                    Some(crate::sync::protocol::Value::Entry(entry)) => rsx! {
                                        p { {language.format(Text::Password2, &[entry.password.as_str()])} }
                                        p { {language.format(Text::Notes, &[entry.notes.as_str()])} }
                                        p { {language.format(Text::Folder2, &[entry.folder_id.as_deref().unwrap_or_default()])} }
                                        if let Some(totp) = &entry.totp { p { {language.format(Text::TotpDetails, &[totp.secret.as_str(), &format!("{:?}", totp.algorithm), &totp.digits.to_string(), &totp.period.to_string()])} } }
                                    },
                                    Some(crate::sync::protocol::Value::Folder(folder)) => rsx! { p { {language.format(Text::Parent, &[folder.parent_id.as_deref().unwrap_or_default()])} } },
                                    None => rsx! {},
                                }
                            }
                            button {
                                disabled: unlocked.vault.signing_error().is_some(),
                                onclick: {
                                    let entity = record.entity.clone();
                                    let revision = version.revision;
                                    move |_| edit_unlocked(state, |u| { u.resolve_conflict(entity.clone(), revision); })
                                },
                                {language.text(Text::KeepThisVersion)}
                            }
                        }
                    }
                }
            }

            if !replay.projection.structural_conflicts().is_empty() {
                p { class: "notice", {language.text(Text::FolderLocationConflictAllEntriesAreSavedCorrectTheFolderLocations)} }
            }

            if !replay.excluded.is_empty() {
                p { class: "notice", {language.text(Text::ChangesFromAClosedEpochAreSavedLocallyAndNeedToBeRebasedAfterReview)} }
                if let Ok(edits) = unlocked.vault.excluded_edits() {
                    for edit in edits {
                        details {
                            summary { {language.format(Text::SavedChange, &[edit.hash.to_string()])} }
                            p { {language.format(Text::OriginalAuthor, &[edit.author.to_string()])} }
                            button { onclick: { let hash = edit.hash; move |_| { let mut selected = revealed_drafts.write(); if !selected.remove(&hash) { selected.insert(hash); } } }, {language.text(Text::ShowHideContents)} }
                            if revealed_drafts.read().contains(&edit.hash) {
                                for mutation in &edit.transaction.mutations {
                                    match &mutation.value {
                                        Some(crate::sync::protocol::Value::Entry(entry)) => rsx! { p { "{entry.title} · {entry.username} · {entry.url}" } p { {language.format(Text::Password2, &[entry.password.as_str()])} } p { "{entry.notes}" } },
                                        Some(crate::sync::protocol::Value::Folder(folder)) => rsx! { p { {language.format(Text::Folder3, &[folder.name.as_str()])} } },
                                        None => rsx! { p { {language.format(Text::Deletion, &[mutation.entity.id.as_str()])} } },
                                    }
                                }
                                p { {language.text(Text::RebasingReplacesTheCurrentVersionsWithTheSelectedContentsYouWillBeTheAuthorOfTheNewChange)} }
                                button { disabled: unlocked.vault.signing_error().is_some(), onclick: { let hash = edit.hash; move |_| edit_unlocked(state, |u| { u.rebase_change(hash); }) }, {language.text(Text::RebaseAfterReview)} }
                            }
                        }
                    }
                }
            }
        }
    }
}
