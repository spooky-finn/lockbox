use crate::ui::i18n::Text;

use super::{
    button::IconButton,
    lock_mark::LockMark,
    symbol::{Icon, Symbol},
};
use crate::app::{
    auth::{MIN_PASSWORD_CHARS, Mode, StorageKind},
    editor::replace_secret,
    session::Screen,
    settings::Settings,
};
use crate::ui::context::{BiometricProvider, Session, edit_locked};
use crate::ui::runtime;
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
pub(super) fn LockedView() -> Element {
    let language = crate::ui::i18n::use_language();
    let state = use_context::<Session>();
    let settings = use_context::<Signal<Settings>>();
    let hardware = use_context::<BiometricProvider>();
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };

    let busy = locked.pending.is_some();
    let create = locked.mode == Mode::Create;
    let cloud_password = create && locked.cloud_password_step;
    let recovering = locked.recovering;
    let ready = cloud_password
        || locked
            .path
            .as_ref()
            .is_some_and(|path| create || path.exists());
    let touch_id = ready && !create && locked.touch_id.is_some();
    let filename = locked
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = locked
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let weak_password = (create || recovering)
        && !locked.password.is_empty()
        && locked.password.chars().count() < MIN_PASSWORD_CHARS;
    let submit = move |event: FormEvent| {
        event.prevent_default();

        if cloud_password {
            runtime::create_cloud_profile();
            return;
        }
        edit_locked(state, |locked| {
            if locked.pending.is_none() {
                locked.submit();
            }
        });
    };
    rsx! {
        section { class: if cloud_password || recovering { "lock-screen recovery-screen" } else { "lock-screen" }, aria_label: language.text(Text::UnlockVault),
            div { class: "unlock-content",
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
                h1 { class: "unlock-title",
                    if recovering { {language.text(Text::RecoverVault)} }
                    else if create && locked.joining { {language.text(Text::SharingConnect)} }
                    else if create { {language.text(Text::CreateAVault)} }
                    else if ready { {language.text(Text::VaultIsLocked)} }
                    else { {language.text(Text::YourPasswordsLockedAway)} }
                }
                p { class: "unlock-description", title: "{path}",
                    if recovering { {language.text(Text::RecoveryUnlockHint)} }
                    else if create && locked.joining && !cloud_password { {language.text(Text::SharingJoinSetup)} }
                    else if cloud_password { {language.text(Text::ChooseAMasterPasswordToProtectYourLocalProfile)} }
                    else if create && locked.path.is_none() { {language.text(Text::ChooseAVaultType)} }
                    else if create { {language.format(Text::ChooseAMasterPasswordForTheNewVault, &[filename.to_string()])} }
                    else if touch_id { {language.format(Text::UseTouchIDOrEnterTheMasterPasswordToOpen, &[filename.to_string()])} }
                    else if ready { {language.format(Text::EnterTheMasterPasswordToOpenVault, &[filename.to_string()])} }
                    else { {language.text(Text::OpenAVaultOrCreateOneToStoreYourPasswords)} }
                }

                if create && locked.path.is_none() && !cloud_password {
                    if !locked.joining {
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
                        super::cloud_setup::CloudSetupView {}
                    }
                }

                if cloud_password {
                    p { class: "notice", {language.text(Text::YourProfileAndKeysWillBeSavedOnThisDeviceStorjHasNotBeenContactedYet)} }
                    button { r#type: "button", class: "setup-button", disabled: busy, onclick: move |_| edit_locked(state, |locked| locked.edit_cloud_connection()), {language.text(Text::BackToConnection)} }
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

                        if create || recovering {
                            label { class: "sr-only", r#for: "repeat-password", {language.text(Text::RepeatMasterPassword)} }
                            div { class: "unlock-input",
                                input { id: "repeat-password", r#type: "password", autocomplete: "off", disabled: busy,
                                    placeholder: language.text(Text::RepeatPassword), value: locked.repeat.as_str(),
                                    oninput: move |event| {
                                        let value = Zeroizing::new(event.value());
                                        edit_locked(state, |locked| replace_secret(&mut locked.repeat, &value));
                                    },
                                }

                                if !cloud_password && !recovering {
                                    IconButton { class: "unlock-submit quiet", kind: "submit", icon: Icon::ArrowRight, label: language.text(Text::CreateVault), disabled: busy }
                                }
                            }
                        }

                        if cloud_password || recovering {
                            label { r#for: "recovery-words", {language.text(Text::RecoveryWords)} }
                            p { class: "muted", {language.text(Text::RecoveryWordsHint)} }
                            if recovering || locked.importing_words || locked.show_recovery_words {
                                textarea { id: "recovery-words", rows: 4, autocomplete: "off", spellcheck: "false",
                                    readonly: !recovering && !locked.importing_words, disabled: busy,
                                    value: locked.recovery_words.as_str(),
                                    oninput: move |event| {
                                        let value = Zeroizing::new(event.value());
                                        edit_locked(state, |locked| {
                                            replace_secret(&mut locked.recovery_words, &value);
                                            locked.words_saved = false;
                                        });
                                    },
                                }
                            }
                            if cloud_password {
                                if !locked.importing_words {
                                    button { r#type: "button", class: "quiet", disabled: busy,
                                        onclick: move |_| edit_locked(state, |locked| locked.show_recovery_words = !locked.show_recovery_words),
                                        if locked.show_recovery_words { {language.text(Text::HideRecoveryWords)} }
                                        else { {language.text(Text::ShowRecoveryWords)} }
                                    }
                                }
                                button { r#type: "button", class: "quiet", disabled: busy,
                                    onclick: move |_| edit_locked(state, |locked| {
                                        if locked.importing_words { locked.generate_recovery_words(); }
                                        else {
                                            zeroize::Zeroize::zeroize(&mut locked.recovery_words);
                                            locked.importing_words = true;
                                            locked.words_saved = false;
                                            locked.show_recovery_words = false;
                                        }
                                    }),
                                    if locked.importing_words { {language.text(Text::GenerateRecoveryWords)} }
                                    else { {language.text(Text::UseRecoveryWords)} }
                                }
                                label { class: "recovery-saved",
                                    input { id: "recovery-saved", r#type: "checkbox", disabled: busy || (!locked.show_recovery_words && !locked.importing_words), checked: locked.words_saved,
                                        onchange: move |event| edit_locked(state, |locked| locked.words_saved = event.checked()),
                                    }
                                    {language.text(Text::RecoveryWordsSaved)}
                                }
                            }
                        }
                        if recovering {
                            button { r#type: "submit", class: "setup-button", disabled: busy, {language.text(Text::ResetPasswordAndUnlock)} }
                        }
                        if cloud_password {
                            button { r#type: "submit", class: "setup-button", disabled: busy, {language.text(Text::CreateProfileAndKeys)} }
                        }
                    }
                }

                if ready && !create {
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
                                else if create { {language.text(Text::CreatingVault)} }
                                else { {language.text(Text::CheckingPassword)} }
                            }
                        }

                        if let Some(error) = &locked.error { p { class: "error", role: "alert", {language.message(error)} } }
                    }
                }
            }
            nav { class: "lock-actions", aria_label: language.text(Text::VaultActions),
                super::settings::LanguageSelect { id: "locked-language" }
                IconButton { class: "quiet", icon: Icon::Folder, label: language.text(Text::OpenVault), disabled: busy,
                    onclick: move |_| runtime::pick_vault(false),
                }
                IconButton { class: "quiet", icon: Icon::FolderPlus, label: language.text(Text::CreateVault2), disabled: busy,
                    onclick: move |_| runtime::begin_create(),
                }

                button { class: "quiet", disabled: busy, onclick: move |_| runtime::begin_join(),
                    {language.text(Text::SharingConnect)}
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
#[path = "tests/locked.rs"]
mod tests;
