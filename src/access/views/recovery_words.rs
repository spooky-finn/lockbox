//! Numbered BIP39 entry with local suggestions and whole-phrase paste.

use crate::{
    access::mnemonic::RecoveryPhrase,
    desktop::{
        context::{Session, edit_locked},
        i18n::Text,
        session::Screen,
    },
};
use bip39::Language;
use dioxus::prelude::*;
use zeroize::Zeroizing;

const WORD_COUNT: usize = 12;

fn slots(phrase: &str) -> [&str; WORD_COUNT] {
    // Empty positions must survive edits so clearing word 3 never moves word 4.
    let mut words = phrase.split(' ');
    std::array::from_fn(|_| words.next().unwrap_or_default())
}

fn suggestions(prefix: &str) -> Vec<&'static str> {
    if prefix.is_empty() {
        return Vec::new();
    }
    Language::English
        .words_by_prefix(prefix)
        .iter()
        .copied()
        .take(6)
        .collect()
}

/// Normalize typed/pasted words, preserving positions and rejecting overflow.
fn replace_words(phrase: &str, index: usize, input: &str) -> Option<(Zeroizing<String>, usize)> {
    let normalized = Zeroizing::new(input.to_ascii_lowercase());
    let mut incoming = normalized.split_whitespace();
    let count = incoming.clone().count();
    let start = if count == WORD_COUNT { 0 } else { index };
    if start + count > WORD_COUNT {
        return None;
    }

    let mut words = slots(phrase);
    if count == 0 {
        words[index] = "";
    } else {
        for word in &mut words[start..start + count] {
            *word = incoming.next().unwrap();
        }
    }

    let advance = count > 1
        || (count == 1
            && input.ends_with(char::is_whitespace)
            && Language::English.find_word(normalized.trim()).is_some());
    let next = if advance { start + count } else { index };
    Some((Zeroizing::new(words.join(" ")), next))
}

fn accept_word(state: Session, index: usize, word: &str) {
    edit_locked(state, |locked| {
        if let Some((phrase, _)) = replace_words(&locked.recovery_words, index, word) {
            locked.recovery_words = phrase;
            locked.words_saved = false;
            locked.error = None;
        }
    });
    focus_word(index + 1);
}

fn focus_word(index: usize) {
    // Only a numeric position enters JavaScript; recovery words stay out of scripts.
    let target = if index < WORD_COUNT {
        format!("document.getElementById('recovery-word-{index}')")
    } else {
        "(document.getElementById('recovery-saved') || document.getElementById('recovery-submit'))"
            .into()
    };
    document::eval(&format!(
        "requestAnimationFrame(() => {{ const field = {target}; field?.focus(); if (field?.type === 'text') field.select(); }});"
    ));
}

#[component]
pub(super) fn RecoveryWordsInput(readonly: bool, disabled: bool) -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let mut active = use_signal(|| None::<usize>);
    let mut highlighted = use_signal(|| 0usize);
    let mut overflow = use_signal(|| false);
    let mut revision = use_signal(|| 0usize);
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };
    let words = slots(&locked.recovery_words);

    if readonly {
        return rsx! {
            ol { id: "recovery-words", class: "recovery-word-grid recovery-word-display", role: "list",
                aria_labelledby: "recovery-words-label", aria_describedby: "recovery-words-hint",
                for (index, word) in words.iter().enumerate() {
                    li { key: "{index}",
                        span { class: "recovery-word-number", aria_hidden: "true", "{index + 1}" }
                        span { "{word}" }
                    }
                }
            }
        };
    }

    let filled = words.iter().filter(|word| !word.is_empty()).count();
    let valid = filled == WORD_COUNT && RecoveryPhrase::parse(&locked.recovery_words).is_ok();
    let invalid = overflow() || (filled == WORD_COUNT && !valid);
    rsx! {
        div { id: "recovery-words", class: "recovery-word-editor", role: "group",
            aria_labelledby: "recovery-words-label", aria_describedby: "recovery-input-hint recovery-word-status",
            onmounted: move |_| { document::eval(include_str!("../../../assets/recovery-words.js")); },
            p { id: "recovery-input-hint", class: "muted recovery-input-hint", {language.text(Text::RecoveryInputHint)} }
            div { class: "recovery-word-grid",
                for (index, word) in words.iter().enumerate() {
                    {
                        let matches = if active() == Some(index) && !disabled { suggestions(word) } else { Vec::new() };
                        let expanded = !matches.is_empty();
                        let selected = highlighted().min(matches.len().saturating_sub(1));
                        let keyboard_matches = matches.clone();
                        let unknown = !word.is_empty() && Language::English.find_word(word).is_none() && active() != Some(index);
                        rsx! {
                            div { class: "recovery-word-cell", key: "{index}-{revision()}",
                                label { class: "recovery-word-number", r#for: "recovery-word-{index}",
                                    span { aria_hidden: "true", "{index + 1}" }
                                    span { class: "sr-only", {language.format(Text::RecoveryWord, &[(index + 1).to_string()])} }
                                }
                                input { id: "recovery-word-{index}", r#type: "text", role: "combobox",
                                    value: "{word}", disabled, autocomplete: "off", autocapitalize: "none", spellcheck: "false",
                                    "autocorrect": "off", aria_autocomplete: "list", aria_expanded: expanded,
                                    aria_controls: expanded.then(|| format!("recovery-suggestions-{index}")),
                                    aria_activedescendant: expanded.then(|| format!("recovery-option-{index}-{selected}")),
                                    aria_invalid: unknown,
                                    onfocus: move |_| { active.set(Some(index)); highlighted.set(0); },
                                    onblur: move |_| { active.set(None); highlighted.set(0); },
                                    oninput: move |event| {
                                        let input = Zeroizing::new(event.value());
                                        let mut next = index;
                                        edit_locked(state, |locked| {
                                            if let Some((phrase, position)) = replace_words(&locked.recovery_words, index, &input) {
                                                locked.recovery_words = phrase;
                                                locked.words_saved = false;
                                                locked.error = None;
                                                overflow.set(false);
                                                next = position;
                                            } else {
                                                locked.words_saved = false;
                                                overflow.set(true);
                                                // Restore the controlled field after rejecting an oversized paste.
                                                revision += 1;
                                                focus_word(index);
                                            }
                                        });
                                        active.set(Some(index));
                                        highlighted.set(0);
                                        if next != index { focus_word(next); }
                                    },
                                    onkeydown: move |event| {
                                        if disabled || event.is_composing() { return; }
                                        match event.key() {
                                            Key::ArrowDown if !keyboard_matches.is_empty() => {
                                                event.prevent_default();
                                                highlighted.set((selected + 1) % keyboard_matches.len());
                                            }
                                            Key::ArrowUp if !keyboard_matches.is_empty() => {
                                                event.prevent_default();
                                                highlighted.set((selected + keyboard_matches.len() - 1) % keyboard_matches.len());
                                            }
                                            Key::Enter => {
                                                event.prevent_default();
                                                if let Some(word) = keyboard_matches.get(selected) {
                                                    overflow.set(false);
                                                    accept_word(state, index, word);
                                                } else {
                                                    let screen = state.read();
                                                    if let Screen::Locked(locked) = &*screen
                                                        && Language::English.find_word(slots(&locked.recovery_words)[index]).is_some()
                                                    {
                                                        focus_word(index + 1);
                                                    }
                                                }
                                            }
                                            Key::Character(key) if key == " " && !keyboard_matches.is_empty() => {
                                                event.prevent_default();
                                                overflow.set(false);
                                                accept_word(state, index, keyboard_matches[selected]);
                                            }
                                            Key::Tab if !event.modifiers().shift() && !keyboard_matches.is_empty() => {
                                                event.prevent_default();
                                                overflow.set(false);
                                                accept_word(state, index, keyboard_matches[selected]);
                                            }
                                            Key::Escape => { active.set(None); }
                                            Key::Backspace if index > 0 => {
                                                let screen = state.read();
                                                if let Screen::Locked(locked) = &*screen
                                                    && slots(&locked.recovery_words)[index].is_empty()
                                                {
                                                    event.prevent_default();
                                                    focus_word(index - 1);
                                                }
                                            }
                                            _ => {}
                                        }
                                    },
                                }
                                if expanded {
                                    ul { id: "recovery-suggestions-{index}", class: "recovery-suggestions", role: "listbox",
                                        aria_label: language.format(Text::RecoveryWord, &[(index + 1).to_string()]),
                                        for (position, suggestion) in matches.iter().copied().enumerate() {
                                            li { id: "recovery-option-{index}-{position}", role: "option", aria_selected: position == selected,
                                                // Retain input focus until click accepts the option.
                                                onmousedown: move |event| event.prevent_default(),
                                                onclick: move |_| {
                                                    if !disabled {
                                                        overflow.set(false);
                                                        accept_word(state, index, suggestion);
                                                    }
                                                },
                                                "{suggestion}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            p { id: "recovery-word-status", class: if invalid { "error recovery-word-status" } else { "muted recovery-word-status" }, role: "status",
                if overflow() { {language.text(Text::TooManyRecoveryWords)} }
                else if invalid { {language.text(Text::InvalidRecoveryWords)} }
                else if valid { {language.text(Text::ValidRecoveryWords)} }
                else { {language.format(Text::RecoveryWordsProgress, &[filled.to_string()])} }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/recovery_words.rs"]
mod tests;
