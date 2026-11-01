use crate::ui::i18n::Text;

use super::{
    button::IconButton,
    symbol::{Icon, Symbol},
};
use super::{
    directories::DirectoryView, directory_editor::DirectoryEditor, editor::DraftView,
    entry::EntryView, sidebar::Sidebar, status::StatusBar, toolbar::Toolbar,
};
use crate::app::session::Screen;
use crate::ui::context::{Session, edit_unlocked};
use dioxus::prelude::*;

#[component]
pub(super) fn UnlockedView() -> Element {
    let language = crate::ui::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let settings_open = unlocked.page == crate::app::workspace::Page::Settings;
    let editing = unlocked.is_editing();
    let has_detail = settings_open
        || unlocked.selected.is_some()
        || unlocked.directories.selected.is_some()
        || editing;
    rsx! {
        Toolbar {}
        div { class: if has_detail { "workspace has-detail" } else { "workspace" },
            Sidebar {}
            section { class: "detail",
                if settings_open || (!editing && has_detail) {
                    IconButton { class: "mobile-back quiet", icon: Icon::ArrowLeft,
                        label: if settings_open { language.text(Text::BackToVault) } else { language.text(Text::BackToList) },
                        onclick: move |_| edit_unlocked(state, |u| {
                            if u.page == crate::app::workspace::Page::Settings { u.page = crate::app::workspace::Page::Vault; }
                            else { u.select_directory(None); }
                        }),
                    }
                }
                if let Some(error) = &unlocked.error { p { class: "error", role: "alert", {language.message(error)} } }
                if settings_open { super::settings::SettingsView {} }
                else { VaultContent {} }
            }
        }
        StatusBar {}
    }
}

#[component]
fn VaultContent() -> Element {
    let language = crate::ui::i18n::use_language();
    let mut revealed = use_signal(std::collections::BTreeSet::<crate::core::sync::Revision>::new);
    let mut revealed_drafts =
        use_signal(std::collections::BTreeSet::<crate::core::sync::Hash>::new);
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    rsx! {
        if let Some(profile) = &unlocked.vault.profile {
            if let Some(sync) = &profile.sync {
                p { class: "notice", aria_live: "polite", {language.format(Text::PendingUpload, &[language.message(unlocked.sync.status.label()), sync.outbox.len().to_string()])} }
                button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| u.sync.retry()), {language.text(Text::RetrySync)} }
            } else {
                p { class: "notice", {language.text(Text::ThisProfilePredatesSyncConfirmYourRecoveryPasswordToConnectExistingEntries)} }
                form {
                    onsubmit: move |event: FormEvent| { event.prevent_default(); edit_unlocked(state, |u| u.activate_sync()); },
                    input { r#type: "password", autocomplete: "off", aria_label: language.text(Text::RecoveryPasswordToEnableSync), placeholder: language.text(Text::RecoveryPassword), value: unlocked.sync_recovery.as_str(),
                        oninput: move |event| edit_unlocked(state, |u| u.sync_recovery = zeroize::Zeroizing::new(event.value())),
                    }
                    button { r#type: "submit", {language.text(Text::EnableSync)} }
                }
            }
        }
        if let Ok(Some(replay)) = unlocked.vault.sync_replay() {
            if !replay.history_verified {
                p { class: "notice", {language.text(Text::TheOwnerConfirmedTheInitialStateThisDeviceHasNotVerifiedEarlierChanges)} }
            }
            for record in replay.projection.conflicts() {
                details { class: "notice",
                    summary { {language.format(Text::ConflictVersions, &[record.entity.id.to_string(), record.versions.len().to_string()])} }
                    for version in &record.versions {
                        div {
                            p { {language.format(Text::Device, &[version.author.to_string()])} }
                            match &version.value {
                                Some(crate::core::sync::Value::Entry(entry)) => rsx! { p { "{entry.title} · {entry.username} · {entry.url}" } },
                                Some(crate::core::sync::Value::Folder(folder)) => rsx! { p { "{folder.name}" } },
                                None => rsx! { p { {language.text(Text::Deleted)} } },
                            }
                            button {
                                onclick: { let revision = version.revision; move |_| { let mut selected = revealed.write(); if !selected.remove(&revision) { selected.insert(revision); } } },
                                {language.text(Text::ShowHideContents)}
                            }
                            if revealed.read().contains(&version.revision) {
                                match &version.value {
                                    Some(crate::core::sync::Value::Entry(entry)) => rsx! {
                                        p { {language.format(Text::Password2, &[entry.password.to_string()])} }
                                        p { {language.format(Text::Notes, &[entry.notes.to_string()])} }
                                        p { {language.format(Text::Folder2, &[entry.folder_id.clone().unwrap_or_default().to_string()])} }
                                        if let Some(totp) = &entry.totp { p { {language.format(Text::TotpDetails, &[totp.secret.to_string(), format!("{:?}", totp.algorithm), totp.digits.to_string(), totp.period.to_string()])} } }
                                    },
                                    Some(crate::core::sync::Value::Folder(folder)) => rsx! { p { {language.format(Text::Parent, &[folder.parent_id.clone().unwrap_or_default().to_string()])} } },
                                    None => rsx! {},
                                }
                            }
                            button {
                                onclick: {
                                    let entity = record.entity.clone();
                                    let revision = version.revision;
                                    move |_| edit_unlocked(state, |u| { u.error = u.vault.resolve_sync(entity.clone(), revision).err().map(|e| e.to_string()); if u.error.is_none() { u.sync.saved(); } })
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
                                        Some(crate::core::sync::Value::Entry(entry)) => rsx! { p { "{entry.title} · {entry.username} · {entry.url}" } p { {language.format(Text::Password2, &[entry.password.to_string()])} } p { "{entry.notes}" } },
                                        Some(crate::core::sync::Value::Folder(folder)) => rsx! { p { {language.format(Text::Folder3, &[folder.name.to_string()])} } },
                                        None => rsx! { p { {language.format(Text::Deletion, &[mutation.entity.id.to_string()])} } },
                                    }
                                }
                                p { {language.text(Text::RebasingReplacesTheCurrentVersionsWithTheSelectedContentsYouWillBeTheAuthorOfTheNewChange)} }
                                button { onclick: { let hash = edit.hash; move |_| edit_unlocked(state, |u| { u.error = u.vault.rebase_edit(hash).err().map(|e| e.to_string()); if u.error.is_none() { u.sync.saved(); } }) }, {language.text(Text::RebaseAfterReview)} }
                            }
                        }
                    }
                }
            }
        }
        if unlocked.directories.draft.is_some() { DirectoryEditor {} }
        else if unlocked.draft.is_some() { DraftView {} }
        else if unlocked.selected_entry().is_some() { EntryView {} }
        else if unlocked.directories.selected.is_some() { DirectoryView {} }
        else { div { class: "empty-state",
            div { class: "empty-icon", Symbol { icon: Icon::Key } }
            if let Some(profile) = &unlocked.vault.profile {
                div { class: "profile-summary",
                    h2 { "{profile.space_name}" }
                    p { {language.format(Text::User, &[profile.username.to_string()])} }
                    if profile.sync.is_some() {
                        p { class: "notice", {language.text(Text::ChangesAreSavedOnThisDeviceAndSyncedThroughStorjWithEncryption)} }
                    }
                    details {
                        summary { {language.text(Text::IdentityAndKeys)} }
                        p { {language.text(Text::UserFingerprint)} }
                        code { "{crate::core::identity::hex(&profile.identity.user_id)}" }
                        p { {language.text(Text::DeviceFingerprint)} }
                        code { "{crate::core::identity::hex(&profile.identity.device_id)}" }
                        p { {language.text(Text::VaultID)} }
                        code { "{crate::core::identity::hex(&profile.space.space_id)}" }
                    }
                }
            } else { h2 { {language.text(Text::EverythingImportantLockedAway)} } }
            p { {language.text(Text::SelectAnEntryOnTheLeftOrCreateOne)} }
            div { class: "actions",
                IconButton { icon: Icon::FolderPlus, label: language.text(Text::NewFolder), onclick: move |_| edit_unlocked(state, |u| u.start_directory()) }
            }
        } }
    }
}

#[cfg(test)]
#[path = "tests/workspace.rs"]
mod tests;
