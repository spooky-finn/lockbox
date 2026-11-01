use crate::ui::i18n::Text;

use crate::ui::components::{button::IconButton, symbol::Icon};
use crate::ui::context::{Session, edit_draft, edit_unlocked};
use crate::{app::session::Screen, core::generator};
use dioxus::prelude::*;

#[component]
pub(in crate::ui) fn GeneratorView() -> Element {
    let language = crate::ui::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };

    let Some(draft) = &unlocked.draft else {
        return rsx! {};
    };

    let options = &unlocked.generator;
    let bits = options.entropy_bits();
    let verdict = if bits < 64.0 {
        language.text(Text::Weak)
    } else if bits < 100.0 {
        language.text(Text::Fair)
    } else {
        language.text(Text::Strong)
    };
    rsx! {
        section { class: "subpanel stack", aria_label: language.text(Text::PasswordGenerator),
            label { r#for: "password-length", {language.format(Text::LengthCharacters, &[options.length.to_string()])} }
            input { id: "password-length", r#type: "range", min: "{generator::LENGTH.start()}", max: "{generator::LENGTH.end()}", value: "{options.length}",
                oninput: move |e| { if let Ok(length) = e.value().parse() { edit_unlocked(state, |u| u.generator.length = length); } } }
            div { class: "actions",
                label { class: "inline", input { r#type: "checkbox", checked: options.lower, onchange: move |e| edit_unlocked(state, |u| u.generator.lower = e.checked()) } "a–z" }
                label { class: "inline", input { r#type: "checkbox", checked: options.upper, onchange: move |e| edit_unlocked(state, |u| u.generator.upper = e.checked()) } "A–Z" }
                label { class: "inline", input { r#type: "checkbox", checked: options.digits, onchange: move |e| edit_unlocked(state, |u| u.generator.digits = e.checked()) } "0–9" }
                label { class: "inline", input { r#type: "checkbox", checked: options.symbols, onchange: move |e| edit_unlocked(state, |u| u.generator.symbols = e.checked()) } "!#$%…" }
            }
            label { class: "inline", input { r#type: "checkbox", checked: options.avoid_ambiguous, onchange: move |e| edit_unlocked(state, |u| u.generator.avoid_ambiguous = e.checked()) } {language.format(Text::ExcludeSimilarCharacters, &[generator::AMBIGUOUS.to_string()])} }
            p { class: "muted", {language.format(Text::StrengthBits, &[format!("{:.0}", bits), verdict.to_string()])} }
            div { class: "actions",
                IconButton { class: "primary", icon: Icon::Check, label: language.text(Text::UsePassword), onclick: move |_| edit_unlocked(state, |u| {
                    if let Some(draft) = &mut u.draft { draft.generate(&u.generator); }
                }) }
                IconButton { icon: Icon::Close, label: language.text(Text::CloseGenerator), onclick: move |_| edit_draft(state, |d| d.generator_open = false) }
            }

            if let Some(error) = &draft.generator_error { p { class: "error", role: "alert", {language.message(error)} } }
        }
    }
}
