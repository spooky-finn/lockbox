use crate::desktop::i18n::Text;

use crate::{
    access::auth::{CreationStep, MIN_PASSWORD_CHARS, Mode, StorageKind},
    desktop::{
        context::{BiometricProvider, Session, edit_locked},
        runtime,
        session::Screen,
        widgets::{
            button::IconButton,
            lock_mark::LockMark,
            symbol::{Icon, Symbol},
        },
    },
    records::editor::replace_secret,
    settings::state::Settings,
};
use dioxus::prelude::*;
use std::borrow::Cow;
use zeroize::Zeroizing;

#[component]
pub(crate) fn LockedView() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let settings = use_context::<Signal<Settings>>();
    let hardware = use_context::<BiometricProvider>();
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };

    let busy = locked.busy();
    let create = locked.mode == Mode::Create;
    let key_setup = create && locked.creation_step == CreationStep::EncryptionKey;
    let identity_setup = create && locked.creation_step == CreationStep::Identity;
    let identity_unlock = identity_setup && locked.unlocking_identity() && !locked.recovering;
    let sync_setup = create && locked.creation_step == CreationStep::Sync;
    let create_password = create && locked.creation_step == CreationStep::Password;
    let cloud_password = create_password && locked.storage_kind == Some(StorageKind::Cloud);
    let recovering = locked.recovering;
    let cloud_restore = locked.cloud_recovery.restoring;
    let ready = identity_setup
        || create_password
        || !create && locked.path.as_ref().is_some_and(|path| path.exists());
    let touch_id = ready && !create && locked.touch_id.is_some();
    let filename = locked
        .space_name
        .as_deref()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| {
            locked
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
        });
    let path = locked
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let weak_password = ((create && !identity_unlock) || recovering)
        && !locked.password.is_empty()
        && locked.password.chars().count() < MIN_PASSWORD_CHARS;
    let submit = move |event: FormEvent| {
        event.prevent_default();

        if cloud_password {
            runtime::create_cloud_profile();
            return;
        }
        edit_locked(state, |locked| {
            if !locked.busy() {
                locked.submit();
            }
        });
    };
    rsx! {
        section { class: if key_setup || identity_setup || cloud_password || recovering { "lock-screen recovery-screen" } else { "lock-screen" }, aria_label: language.text(if key_setup { Text::EncryptionKeySetup } else { Text::UnlockVault }),
            if !create && !recovering {
                div { class: "locked-space-switcher", crate::desktop::space_switcher::SpaceSwitcher {} }
            }
            div { class: "unlock-content",
                if !key_setup && !identity_setup && !cloud_password {
                    LockMark {
                        if touch_id {
                            IconButton { class: "lock-badge touch-id", icon: Icon::Fingerprint, label: language.text(Text::UnlockWithTouchID), disabled: busy,
                                onclick: move |_| {
                                    let biometrics = hardware.for_language(language);
                                    edit_locked(state, |locked| locked.start_touch_id(biometrics));
                                },
                            }
                        } else {
                            span { class: "lock-badge", aria_hidden: "true", Symbol { icon: Icon::Lock } }
                        }
                    }
                }
                h1 { class: "unlock-title",
                    if cloud_restore { {language.text(Text::CloudRestore)} }
                    else if recovering { {language.text(Text::RecoverVault)} }
                    else if key_setup { {language.text(Text::EncryptionKeySetup)} }
                    else if identity_unlock { {language.text(Text::UnlockIdentity)} }
                    else if identity_setup { {language.text(Text::ProtectIdentity)} }
                    else if create && locked.joining { {language.text(Text::SharingConnect)} }
                    else if create { {language.text(Text::CreateAVault)} }
                    else if ready { {language.text(Text::VaultIsLocked)} }
                    else { {language.text(Text::YourPasswordsLockedAway)} }
                }
                p { class: "unlock-description", title: "{path}",
                    if cloud_restore && key_setup { {language.text(Text::CloudRestoreWordsHint)} }
                    else if cloud_restore && sync_setup { {language.text(Text::CloudRestoreHint)} }
                    else if recovering { {language.text(Text::RecoveryUnlockHint)} }
                    else if key_setup { {language.text(Text::EncryptionKeySetupHint)} }
                    else if identity_unlock { {language.text(Text::UnlockIdentityHint)} }
                    else if identity_setup { {language.text(Text::ProtectIdentityHint)} }
                    else if create && locked.joining && !cloud_password { {language.text(Text::SharingJoinSetup)} }
                    else if cloud_password { {language.text(Text::ChooseAMasterPasswordToProtectYourLocalProfile)} }
                    else if create && locked.path.is_none() { {language.text(Text::ChooseAVaultType)} }
                    else if create { {language.format(Text::ChooseAMasterPasswordForTheNewVault, std::slice::from_ref(&filename))} }
                    else if touch_id { {language.format(Text::UseTouchIDOrEnterTheMasterPasswordToOpen, std::slice::from_ref(&filename))} }
                    else if ready { {language.format(Text::EnterTheMasterPasswordToOpenVault, std::slice::from_ref(&filename))} }
                    else { {language.text(Text::OpenAVaultOrCreateOneToStoreYourPasswords)} }
                }

                if key_setup {
                    super::key_setup::KeySetupView {}
                }

                if sync_setup {
                    if locked.identity.is_some() {
                        p { class: "notice", {language.text(Text::SharedIdentityHint)} }
                    } else {
                        button { r#type: "button", class: "setup-button", disabled: busy,
                            onclick: move |_| edit_locked(state, |locked| locked.edit_recovery_key()),
                            {language.text(Text::BackToEncryptionKey)}
                        }
                    }
                    if !locked.joining && !cloud_restore {
                        div { class: "storage-choices",
                            button { r#type: "button", class: "storage-choice", onclick: move |_| runtime::pick_vault(true),
                                strong { {language.text(Text::SingleUser)} }
                                span { {language.text(Text::LocalVaultOnThisDevice)} }
                            }
                            button { r#type: "button", class: "storage-choice",
                                aria_pressed: locked.storage_kind == Some(StorageKind::Cloud),
                                onclick: move |_| edit_locked(state, |locked| {
                                    locked.storage_kind = Some(StorageKind::Cloud);
                                    locked.error = None;
                                }),
                                strong { {language.text(Text::MultipleUsers)} }
                                span { {language.text(Text::SharedCloudVaultInStorj)} }
                            }
                        }
                    }
                    if locked.storage_kind == Some(StorageKind::Cloud) {
                        crate::access::views::cloud_setup::CloudSetupView {}
                    }
                }

                if create_password {
                    if cloud_password && !cloud_restore {
                        p { class: "notice", {language.text(Text::YourProfileAndKeysWillBeSavedOnThisDeviceStorjHasNotBeenContactedYet)} }
                    }
                    button { r#type: "button", class: "setup-button", disabled: busy,
                        onclick: move |_| edit_locked(state, |locked| locked.edit_sync_setup()),
                        {language.text(if cloud_password { Text::BackToConnection } else { Text::BackToSyncSetup })}
                    }
                }

                if ready {
                    form { class: "unlock-form", onsubmit: submit,
                        label { class: "sr-only", r#for: "master-password", {language.text(if recovering { Text::NewMasterPassword } else { Text::MasterPassword })} }
                        div { class: "unlock-input",
                            input { id: "master-password", r#type: "password", autocomplete: "off", autofocus: true,
                                placeholder: if recovering { language.text(Text::NewMasterPassword) } else if create { language.text(Text::MasterPassword) } else { language.text(Text::EnterMasterPassword) },
                                disabled: busy, value: locked.password.as_str(),
                                oninput: move |event| {
                                    let value = Zeroizing::new(event.value());
                                    edit_locked(state, |locked| replace_secret(&mut locked.password, &value));
                                },
                            }

                            if !create && !recovering {
                                IconButton { class: "unlock-submit quiet", kind: "submit", icon: Icon::ArrowRight, label: language.text(Text::Unlock), disabled: busy }
                            }
                        }

                        if (create && !identity_unlock) || recovering {
                            label { class: "sr-only", r#for: "repeat-password", {language.text(Text::RepeatMasterPassword)} }
                            div { class: "unlock-input",
                                input { id: "repeat-password", r#type: "password", autocomplete: "off", disabled: busy,
                                    placeholder: language.text(Text::RepeatPassword), value: locked.repeat.as_str(),
                                    oninput: move |event| {
                                        let value = Zeroizing::new(event.value());
                                        edit_locked(state, |locked| replace_secret(&mut locked.repeat, &value));
                                    },
                                }

                                if !cloud_password && !recovering && !identity_setup {
                                    IconButton { class: "unlock-submit quiet", kind: "submit", icon: Icon::ArrowRight, label: language.text(Text::CreateVault), disabled: busy }
                                }
                            }
                        }

                        if recovering {
                            p { id: "recovery-words-label", class: "recovery-label", {language.text(Text::RecoveryWords)} }
                            p { id: "recovery-words-hint", class: "muted", {language.text(Text::RecoveryWordsHint)} }
                            super::recovery_words::RecoveryWordsInput { readonly: false, disabled: busy }
                        }
                        if recovering {
                            button { id: "recovery-submit", r#type: "submit", class: "setup-button", disabled: busy, {language.text(Text::ResetPasswordAndUnlock)} }
                        }
                        if cloud_password {
                            if cloud_restore {
                                if let Some(space) = locked.cloud_recovery.selected.and_then(|i| locked.cloud_recovery.spaces.get(i)) {
                                    p { strong { {space.name()} } }
                                }
                                label { class: "recovery-saved",
                                    input { id: "cloud-recovery-confirm", r#type: "checkbox", disabled: busy, checked: locked.cloud_recovery.trust_snapshot,
                                        onchange: move |event| edit_locked(state, |locked| locked.cloud_recovery.trust_snapshot = event.checked()),
                                    }
                                    {language.text(Text::CloudRestoreConfirm)}
                                }
                            }
                            button { id: "recovery-submit", r#type: "submit", class: "setup-button", disabled: busy || (cloud_restore && !locked.cloud_recovery.trust_snapshot), {language.text(if cloud_restore { Text::CloudRestoreAction } else { Text::CreateProfileAndKeys })} }
                        }
                        if identity_setup && !recovering {
                            button { r#type: "submit", class: "setup-button", disabled: busy, {language.text(Text::Continue)} }
                        }
                    }
                }

                if ready && (!create || (identity_setup && locked.identity_source.is_none() && locked.unlocking_identity())) {
                    button { r#type: "button", class: "quiet", disabled: busy,
                        onclick: move |_| edit_locked(state, |locked| locked.toggle_recovery()),
                        if recovering { {language.text(Text::BackToPasswordUnlock)} }
                        else { {language.text(Text::ForgotMasterPassword)} }
                    }
                }

                if weak_password || locked.notice.is_some() || busy || locked.error.is_some() {
                    div { class: "unlock-feedback", aria_live: "polite",
                        if weak_password { p { class: "notice", {language.format(Text::PasswordsShorterThanCharactersAreEasierToGuess, &[MIN_PASSWORD_CHARS.to_string()])} } }
                        if let Some(notice) = &locked.notice { p { class: "notice", role: "status", {language.message(notice)} } }
                        if busy {
                            p { role: "status", class: "muted",
                                if locked.waiting_touch_id { {language.text(Text::WaitingForTouchID)} }
                                else if cloud_restore && !identity_setup { {language.text(Text::CloudRestoreProgress)} }
                                else if create && !identity_setup { {language.text(Text::CreatingVault)} }
                                else { {language.text(Text::CheckingPassword)} }
                            }
                        }

                        if let Some(error) = &locked.error { p { class: "error", role: "alert", {language.message(error)} } }
                    }
                }
            }
            nav { class: "lock-actions", aria_label: language.text(Text::VaultActions),
                IconButton { class: "quiet", icon: Icon::Folder, label: language.text(Text::OpenVault), disabled: busy,
                    onclick: move |_| runtime::pick_vault(false),
                }
                IconButton { class: "quiet", icon: Icon::FolderPlus, label: language.text(Text::CreateVault2), disabled: busy,
                    onclick: move |_| runtime::begin_create(),
                }

                button { class: "quiet", disabled: busy, onclick: move |_| runtime::begin_join(),
                    {language.text(Text::SharingConnect)}
                }
                button { class: "quiet", disabled: busy, onclick: move |_| runtime::begin_cloud_restore(),
                    {language.text(Text::CloudRestore)}
                }

                if settings.read().config.last_vault.is_some() {
                    IconButton { class: "quiet", icon: Icon::FolderX, label: language.text(Text::ForgetPath), disabled: busy,
                        onclick: move |_| runtime::forget_path(),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/locked.rs"]
mod tests;
