use super::symbol::{Icon, Symbol};
use dioxus::prelude::*;

/// Icon actions always retain a tooltip and a screen-reader name.
#[component]
pub(super) fn IconButton(
    icon: Icon,
    label: String,
    #[props(default)] class: String,
    #[props(default = "button")] kind: &'static str,
    #[props(default = false)] disabled: bool,
    pressed: Option<bool>,
    expanded: Option<bool>,
    onclick: Option<EventHandler<MouseEvent>>,
) -> Element {
    rsx! {
        button {
            class: "icon-button {class}", r#type: kind,
            title: "{label}", aria_label: "{label}", disabled,
            aria_pressed: pressed.map(|value| value.to_string()),
            aria_expanded: expanded.map(|value| value.to_string()),
            onclick: move |event| {
                if let Some(handler) = onclick { handler.call(event); }
            },
            Symbol { icon }
        }
    }
}
