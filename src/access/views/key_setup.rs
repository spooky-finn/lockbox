//! Confirm the recovery key before choosing local or cloud storage.

use crate::desktop::{
    context::{Session, edit_locked},
    i18n::Text,
    session::Screen,
};
use dioxus::prelude::*;
use zeroize::Zeroize;

#[component]
pub(crate) fn KeySetupView() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };
    let busy = locked.pending.is_some();

    rsx! {
        form { class: "unlock-form", onsubmit: move |event: FormEvent| {
                event.prevent_default();
                edit_locked(state, |locked| locked.continue_key_setup());
            },
            p { id: "recovery-words-label", class: "recovery-label", {language.text(Text::RecoveryWords)} }
            p { id: "recovery-words-hint", class: "muted", {language.text(Text::RecoveryWordsHint)} }
            if locked.importing_words || locked.show_recovery_words {
                super::recovery_words::RecoveryWordsInput {
                    readonly: !locked.importing_words, disabled: busy,
                }
            }
            div { class: "recovery-actions",
                if !locked.importing_words {
                    button { r#type: "button", class: "setup-button", disabled: busy,
                        onclick: move |_| edit_locked(state, |locked| locked.show_recovery_words = !locked.show_recovery_words),
                        if locked.show_recovery_words { {language.text(Text::HideRecoveryWords)} }
                        else { {language.text(Text::ShowRecoveryWords)} }
                    }
                }
                button { r#type: "button", class: "setup-button", disabled: busy,
                    onclick: move |_| edit_locked(state, |locked| {
                        if locked.importing_words { locked.generate_recovery_words(); }
                        else {
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
            label { class: "recovery-saved",
                input { id: "recovery-saved", r#type: "checkbox", disabled: busy || (!locked.show_recovery_words && !locked.importing_words), checked: locked.words_saved,
                    onchange: move |event| edit_locked(state, |locked| locked.words_saved = event.checked()),
                }
                {language.text(Text::RecoveryWordsSaved)}
            }
            button { id: "recovery-submit", r#type: "submit", class: "setup-button", disabled: busy, {language.text(Text::Continue)} }
        }
    }
}
