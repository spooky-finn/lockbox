use dioxus::prelude::*;
use std::time::Duration;

pub(super) const DURATION: Duration = Duration::from_millis(1050);

#[component]
pub(super) fn UnlockSpell() -> Element {
    let mut visible = use_signal(|| true);
    if !visible() {
        return rsx! {};
    }

    rsx! {
        style { {include_str!("../../assets/unlock-spell.css")} }
        div {
            class: "unlock-spell",
            aria_hidden: "true",
            style: "--spell-duration:{DURATION.as_millis()}ms",
            onmounted: move |_| {
                spawn(async move {
                    tokio::time::sleep(DURATION).await;
                    visible.set(false);
                });
            },
            div { class: "unlock-spell-glyph",
                div { class: "unlock-spell-lock",
                    span { class: "unlock-spell-shackle" }
                    span { class: "unlock-spell-lock-body" }
                }
                for angle in [-78, -43, -5] {
                    span { class: "unlock-spell-ember", style: "--angle:{angle}deg" }
                }
            }
        }
    }
}
