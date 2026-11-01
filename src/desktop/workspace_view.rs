use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        session::Screen,
        sync_review::SyncReview,
        widgets::{
            button::IconButton,
            symbol::{Icon, Symbol},
        },
    },
    records::views::{
        directories::DirectoryView, directory_editor::DirectoryEditor, editor::DraftView,
        entry::EntryView, sidebar::Sidebar, toolbar::Toolbar,
    },
};
use dioxus::prelude::*;

#[component]
pub(crate) fn UnlockedView() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };

    let records_open = matches!(
        unlocked.page,
        crate::desktop::workspace::Page::History | crate::desktop::workspace::Page::Trash
    );
    let settings_open = unlocked.page == crate::desktop::workspace::Page::Settings;
    let sharing_open = unlocked.page == crate::desktop::workspace::Page::Sharing;
    let editing = unlocked.is_editing();
    let has_detail = records_open
        || settings_open
        || sharing_open
        || unlocked.selected.is_some()
        || unlocked.directories.selected.is_some()
        || editing;
    rsx! {
        Toolbar {}
        div { class: if has_detail { "workspace has-detail" } else { "workspace" },
            Sidebar {}
            section { class: "detail",
                if !records_open && (settings_open || sharing_open || (!editing && has_detail)) {
                    IconButton { class: "mobile-back quiet", icon: Icon::ArrowLeft,
                        label: if settings_open || sharing_open { language.text(Text::BackToVault) } else { language.text(Text::BackToList) },
                        onclick: move |_| edit_unlocked(state, |u| {
                            if u.page != crate::desktop::workspace::Page::Vault { u.page = crate::desktop::workspace::Page::Vault; }
                            else { u.select_directory(None); }
                        }),
                    }
                }

                if let Some(error) = &unlocked.error { p { class: "error", role: "alert", {language.message(error)} } }
                if let Some(error) = unlocked.vault.signing_error() { ProtectionNotice { error } }
                if settings_open { crate::settings::view::SettingsView {} }
                else if sharing_open { crate::sharing::views::SharingView {} }
                else if records_open { crate::records::views::history::RecordsView { key: "{records_open}:{unlocked.selected:?}:{unlocked.page == crate::desktop::workspace::Page::Trash}" } }
                else { VaultContent {} }
            }
        }
    }
}

#[component]
fn VaultContent() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    rsx! {
        if let Some(profile) = unlocked.vault.profile_summary() {
            if !profile.sync_enabled {
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

        SyncReview {}

        if unlocked.directories.draft.is_some() { DirectoryEditor {} }
        else if unlocked.draft.is_some() { DraftView {} }
        else if unlocked.selected_entry().is_some() { EntryView {} }
        else if unlocked.directories.selected.is_some() { DirectoryView {} }
        else { div { class: "empty-state vault-welcome",
            div { class: "empty-icon", Symbol { icon: Icon::Key } }
            if let Some(profile) = unlocked.vault.profile_summary() {
                div { class: "profile-summary",
                    h2 { "{profile.space_name}" }
                    p { {language.format(Text::User, &[profile.username])} }
                    if profile.sync_enabled {
                        p { class: "notice", {language.text(Text::ChangesAreSavedOnThisDeviceAndSyncedThroughStorjWithEncryption)} }
                    }
                    details {
                        summary { {language.text(Text::IdentityAndKeys)} }
                        p { {language.text(Text::UserFingerprint)} }
                        code { "{crate::sync::identity::hex(&profile.user_id)}" }
                        p { {language.text(Text::DeviceFingerprint)} }
                        code { "{crate::sync::identity::hex(&profile.device_id)}" }
                        p { {language.text(Text::VaultID)} }
                        code { "{crate::sync::identity::hex(&profile.space_id)}" }
                    }
                }
            } else { h2 { {language.text(Text::EverythingImportantLockedAway)} } }
            p { {language.text(Text::SelectAnEntryOnTheLeftOrCreateOne)} }
            div { class: "actions",
                button { r#type: "button", class: "welcome-create", disabled: unlocked.vault.signing_error().is_some(), onclick: move |_| edit_unlocked(state, |u| u.start_directory()),
                    Symbol { icon: Icon::FolderPlus }
                    {language.text(Text::NewFolder)}
                }
            }
        } }
    }
}

#[component]
fn ProtectionNotice(error: crate::vault::anchor::Error) -> Element {
    use crate::vault::anchor::Error;
    let language = crate::desktop::i18n::use_language();
    let text = match error {
        Error::MissingBinding => Text::ProfileImportedReadOnly,
        Error::Rollback => Text::ProfileRollbackReadOnly,
        Error::Legacy => Text::LegacyProfileReadOnly,
        Error::Unavailable => Text::LocalSecurityStorageUnavailable,
        Error::StorageAccessDenied => Text::LocalSecurityStorageAccessDenied,
        Error::InUse => Text::SigningDeviceAlreadyOpen,
        Error::Exhausted => Text::ProfileRequiresReplacement,
    };
    let state = use_context::<Session>();
    rsx! {
        p { class: "notice", role: "status", {language.text(text)} }
        if matches!(error, Error::Unavailable | Error::StorageAccessDenied) {
            button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| {
                u.retry_profile_protection();
            }), {language.text(Text::RetrySync)} }
        }
    }
}

#[cfg(test)]
#[path = "tests/workspace_view.rs"]
mod tests;
