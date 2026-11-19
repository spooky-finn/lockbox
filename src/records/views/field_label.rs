use crate::desktop::widgets::symbol::{Icon, Symbol};
use dioxus::prelude::*;

#[component]
pub(super) fn FieldLabel(icon: Icon, text: String, #[props(default)] focusable: bool) -> Element {
    rsx! {
        dt { class: "field-label", tabindex: focusable.then_some("0"),
            Symbol { icon }
            span { class: "field-label-tooltip", "{text}" }
        }
    }
}
