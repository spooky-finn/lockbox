//! Confirm the recovery key before choosing local or cloud storage.

use crate::desktop::{
    context::{SecurityHandle, Session, edit_locked},
    i18n::Text,
    session::Screen,
};
use dioxus::prelude::*;
use zeroize::Zeroize;

#[component]
pub(crate) fn KeySetupView() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let native = use_context::<SecurityHandle>();
    let mut copied = use_signal(|| false);
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };
    let busy = locked.busy();

    rsx! {
        form { class: "unlock-form", onsubmit: move |event: FormEvent| {
                event.prevent_default();
                edit_locked(state, |locked| locked.continue_key_setup());
            },
            p { id: "recovery-words-label", class: "recovery-label", {language.text(Text::RecoveryWords)} }
            p { id: "recovery-words-hint", class: "muted", {language.text(Text::RecoveryWordsHint)} }
            if let Some(indices) = locked.word_challenge {
                p { class: "muted", {language.text(Text::RecoveryCheckHint)} }
                div { class: "recovery-check",
                    for (slot, index) in indices.iter().copied().enumerate() {
                        label {
                            {language.format(Text::RecoveryWord, &[(index + 1).to_string()])}
                            input { r#type: "password", value: locked.word_answers[slot].as_str(),
                                disabled: busy, autocomplete: "off", spellcheck: "false", autocapitalize: "none",
                                oninput: move |event| edit_locked(state, |locked| {
                                    locked.word_answers[slot] = zeroize::Zeroizing::new(event.value());
                                    locked.error = None;
                                }),
                            }
                        }
                    }
                }
                button { r#type: "button", class: "setup-button", disabled: busy,
                    onclick: move |_| edit_locked(state, |locked| { locked.reset_word_challenge(); locked.error = None; }),
                    {language.text(Text::BackToWords)}
                }
            }
            if locked.word_challenge.is_none() && (locked.importing_words || locked.show_recovery_words) {
                super::recovery_words::RecoveryWordsInput {
                    readonly: !locked.importing_words, disabled: busy,
                }
            }
            if !locked.cloud_recovery.restoring && locked.word_challenge.is_none() {
            div { class: "recovery-actions",
                if !locked.importing_words {
                    button { r#type: "button", class: "setup-button", disabled: busy,
                        onclick: move |_| edit_locked(state, |locked| locked.show_recovery_words = !locked.show_recovery_words),
                        if locked.show_recovery_words { {language.text(Text::HideRecoveryWords)} }
                        else { {language.text(Text::ShowRecoveryWords)} }
                    }
                }
                if !locked.importing_words {
                    button { r#type: "button", class: "setup-button", disabled: busy,
                        onclick: move |_| {
                            edit_locked(state, |locked| {
                                let words = column_words(&locked.recovery_words);
                                match native.borrow_mut().clipboard.copy(&words, std::time::Instant::now()) {
                                    Ok(()) => { copied.set(true); locked.error = None; }
                                    Err(error) => { copied.set(false); locked.error = Some(error); }
                                }
                            });
                        },
                        {language.text(Text::CopyRecoveryWords)}
                    }
                    button { r#type: "button", class: "setup-button", disabled: busy,
                        onclick: move |_| { download_words(state); },
                        {language.text(Text::DownloadRecoveryWords)}
                    }
                    if copied() {
                        span { role: "status", {language.text(Text::Copied)} }
                    }
                }
                button { r#type: "button", class: "setup-button", disabled: busy,
                    onclick: move |_| edit_locked(state, |locked| {
                        if locked.importing_words { locked.generate_recovery_words(); }
                        else {
                            locked.reset_word_challenge();
                            locked.recovery_words.zeroize();
                            locked.importing_words = true;
                            locked.words_saved = false;
                            locked.show_recovery_words = false;
                        }
                    }),
                    if locked.importing_words { {language.text(Text::GenerateRecoveryWords)} }
                    else { {language.text(Text::UseRecoveryWords)} }
                }
            }
            }
            if locked.importing_words {
            label { class: "recovery-saved",
                input { id: "recovery-saved", r#type: "checkbox", disabled: busy || (!locked.show_recovery_words && !locked.importing_words), checked: locked.words_saved,
                    onchange: move |event| edit_locked(state, |locked| locked.words_saved = event.checked()),
                }
                {language.text(Text::RecoveryWordsSaved)}
            }
            }
            button { id: "recovery-submit", r#type: "submit", class: "setup-button", disabled: busy, {language.text(Text::Continue)} }
        }
    }
}

fn column_words(phrase: &str) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(phrase.replace(' ', "\n"))
}

fn download_words(state: Session) {
    let words = match &*state.peek() {
        Screen::Locked(locked) if !locked.busy() && !locked.importing_words => {
            column_words(&locked.recovery_words)
        }
        _ => return,
    };
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_file_name("lockbox-recovery-words.txt")
            .add_filter("Text", &["txt"])
            .save_file()
            .await
        else {
            return;
        };
        use std::io::Write;
        let result =
            crate::platform::files::create_new_private(file.path()).and_then(|mut output| {
                output.write_all(words.as_bytes())?;
                output.sync_all()
            });
        edit_locked(state, |locked| {
            locked.error = result.err().map(|error| error.to_string())
        });
    });
}
