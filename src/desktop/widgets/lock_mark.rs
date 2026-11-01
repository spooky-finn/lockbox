use dioxus::prelude::*;

const MARK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/lockbox-mark.svg"
));

#[component]
pub(crate) fn LockMark(children: Element) -> Element {
    rsx! {
        div { class: "lock-emblem",
            // Trusted, bundled SVG; no vault or user-provided markup enters it.
            div { class: "lock-mark", aria_hidden: "true", dangerous_inner_html: MARK }
            {children}
        }
    }
}
