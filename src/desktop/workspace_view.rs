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
        if unlocked.record_created > 0 {
            CreationCelebration { key: "{unlocked.record_created}" }
        }
        div { class: if has_detail { "workspace has-detail" } else { "workspace" },
            onkeydown: move |event| handle_record_shortcut(state, event),
            Sidebar {}
            if unlocked.pending_navigation.is_some() {
                dialog { id: "record-save-dialog", class: "record-dialog stack", aria_labelledby: "save-entry-question",
                    onmounted: move |_| { document::eval("document.getElementById('record-save-dialog')?.showModal();"); },
                    oncancel: move |event| { event.prevent_default(); edit_unlocked(state, |u| u.pending_navigation = None); },
                        h2 { id: "save-entry-question", {language.text(Text::SaveEntryChanges)} }
                        if let Some(error) = &unlocked.error { p { class: "error", role: "alert", {language.message(error)} } }
                        div { class: "actions",
                            button { r#type: "button", class: "primary", autofocus: true, onclick: move |_| edit_unlocked(state, |u| u.resolve_navigation(true)), {language.text(Text::Save)} }
                            button { r#type: "button", class: "danger quiet", onclick: move |_| edit_unlocked(state, |u| u.resolve_navigation(false)), {language.text(Text::DiscardChanges)} }
                            button { r#type: "button", class: "quiet", onclick: move |_| edit_unlocked(state, |u| u.pending_navigation = None), {language.text(Text::Cancel)} }
                        }
                }
            }
            section { class: "detail",
                if !records_open && (settings_open || sharing_open || (!editing && has_detail)) {
                    IconButton { class: "mobile-back quiet", icon: Icon::ArrowLeft,
                        label: if settings_open || sharing_open { language.text(Text::BackToVault) } else { language.text(Text::BackToList) },
                        onclick: move |_| edit_unlocked(state, |u| {
                            if u.page != crate::desktop::workspace::Page::Vault { u.page = crate::desktop::workspace::Page::Vault; }
                            else { u.navigate(crate::desktop::workspace::Destination::Directory(None)); }
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

/// Keep composition and held keys from closing or saving a record accidentally.
pub(crate) fn handle_record_shortcut(state: Session, event: KeyboardEvent) {
    if event.is_composing() || event.is_auto_repeating() {
        return;
    }
    let modifiers = event.modifiers();
    let save = event.code() == Code::KeyS
        && modifiers.intersects(Modifiers::CONTROL | Modifiers::META)
        && !modifiers.intersects(Modifiers::ALT | Modifiers::SHIFT);
    let close = event.key() == Key::Escape && modifiers.is_empty();
    let active = matches!(&*state.peek(), Screen::Unlocked(u) if
        (save && u.draft.is_some() && u.pending_navigation.is_none()) ||
        (close && (u.pending_navigation.is_some() || u.draft.is_some() || u.selected.is_some())));
    if !active {
        return;
    }
    event.prevent_default();
    event.stop_propagation();
    crate::desktop::runtime::record_activity();
    edit_unlocked(state, |u| {
        if save {
            u.error = u.save_draft().err();
        } else if u.pending_navigation.is_some() {
            u.pending_navigation = None;
        } else {
            u.navigate(crate::desktop::workspace::Destination::Close);
        }
    });
}

/// A bounded, non-interactive burst, removed on animation completion or lock.
#[component]
fn CreationCelebration() -> Element {
    let language = crate::desktop::i18n::use_language();
    let mut visible = use_signal(|| true);
    rsx! {
        if visible() {
            style { {include_str!("../../assets/celebration.css")} }
            div { class: "creation-celebration", role: "status", aria_live: "polite",
                onanimationend: move |_| visible.set(false),
                div { class: "creation-sparks", aria_hidden: "true",
                    onanimationend: move |event| event.stop_propagation(),
                    for index in 0..16 {
                        span { class: "creation-spark", style: "--angle:{index * 360 / 16}deg;" }
                    }
                }
                span { class: "creation-message", {language.text(Text::RecordCreated)} }
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
