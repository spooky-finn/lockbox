//! Dioxus presentation. Each component subscribes to its own reactive inputs.
mod editor;
mod entry;
mod generator;
mod icons;
mod locked;
mod sidebar;
mod status;
mod toolbar;
mod totp;
mod workspace;

use crate::app::{runtime, session::Screen};
use dioxus::prelude::*;
use locked::LockedView;
use workspace::UnlockedView;
const CSS: &str = include_str!("../../../assets/app.css");

#[component]
pub fn App() -> Element {
    let state = runtime::use_runtime();
    let last_motion =
        use_hook(|| std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now())));
    let locked = matches!(&*state.read(), Screen::Locked(_));
    rsx! {
        style { "{CSS}" }
        main {
            class: "app",
            onpointerdown: move |_| runtime::record_activity(),
            onkeydown: move |_| runtime::record_activity(),
            onwheel: move |_| runtime::record_activity(),
            onpointermove: move |_| {
                if last_motion.get().elapsed() >= std::time::Duration::from_secs(1) {
                    last_motion.set(std::time::Instant::now());
                    runtime::record_activity();
                }
            },
            if locked { LockedView {} } else { UnlockedView {} }
        }
    }
}

#[cfg(test)]
mod tests;
