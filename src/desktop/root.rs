//! Dioxus presentation. Each component subscribes to its own reactive inputs.

use crate::access::views::locked::LockedView;
use crate::desktop::{runtime, session::Screen, widgets::theme, workspace_view::UnlockedView};
use dioxus::prelude::*;
const CSS: &str = include_str!("../../assets/app.css");

#[component]
pub fn App() -> Element {
    let state = runtime::use_runtime();
    let theme = theme::use_theme();
    let settings = use_context::<Signal<crate::settings::state::Settings>>();
    let appearance_style =
        theme::appearance_style(settings.read().config.appearance, theme.is_dark());
    let language = crate::desktop::i18n::use_language();
    let last_motion =
        use_hook(|| std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now())));
    let locked = matches!(&*state.read(), Screen::Locked(_));
    let title = crate::desktop::window::title(&state.read(), language);
    let shown_title = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(String::new())));

    if *shown_title.borrow() != title {
        dioxus::desktop::window().set_title(&title);
        *shown_title.borrow_mut() = title;
    }
    rsx! {
        style { "{CSS}" }
        main {
            lang: language.code(),
            class: if locked { "app is-locked" } else { "app" },
            "data-theme": theme.name(),
            style: appearance_style,
            onpointerdown: move |_| runtime::record_activity(),
            onkeydown: move |_| runtime::record_activity(),
            onwheel: move |_| runtime::record_activity(),
            onpointermove: move |_| {
                if last_motion.get().elapsed() >= std::time::Duration::from_secs(1) {
                    last_motion.set(std::time::Instant::now());
                    runtime::record_activity();
                }
            },
            crate::desktop::browser::BrowserConsent {}
            if locked { LockedView {} } else { UnlockedView {} }
        }
    }
}
