//! Compact, copyable display for public identifiers and hashes.
use crate::desktop::{
    context::{SecurityHandle, Session, edit_unlocked},
    i18n::{Text, use_language},
    runtime,
};
use dioxus::prelude::*;

#[component]
pub(crate) fn Identifier(value: String) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let native = use_context::<SecurityHandle>();
    let short = abbreviate(&value);
    let label = language.format(Text::CopyIdentifier, std::slice::from_ref(&short));
    rsx! {
        button {
            r#type: "button", class: "quiet identifier",
            title: "{label}", aria_label: "{label}",
            onclick: move |_| {
                edit_unlocked(state, |u| {
                    u.error = native.borrow_mut().clipboard
                        .copy(&value, std::time::Instant::now())
                        .err().map(|error| format!("Could not copy: {error}"));
                });
                runtime::publish_status();
            },
            code { "{short}" }
        }
    }
}

fn abbreviate(value: &str) -> String {
    if value.chars().count() <= 8 {
        return value.to_owned();
    }
    let prefix: String = value.chars().take(4).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}…{suffix}")
}

#[cfg(test)]
#[path = "tests/identifier.rs"]
mod tests;
