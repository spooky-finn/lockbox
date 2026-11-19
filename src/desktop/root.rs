//! Dioxus presentation. Each component subscribes to its own reactive inputs.

use crate::access::views::locked::LockedView;
use crate::desktop::{runtime, session::Screen, widgets::theme, workspace_view::UnlockedView};
use dioxus::prelude::*;
const CSS: &str = include_str!("../../assets/app.css");

#[component]
pub fn App() -> Element {
    let state = runtime::use_runtime();
    crate::desktop::window::use_interface_zoom();
    let tabs = use_context::<Signal<crate::desktop::tabs::Tabs>>();
    let active_tab = tabs.read().active;
    let theme = theme::use_theme();
    let settings = use_context::<Signal<crate::settings::state::Settings>>();
    let appearance = settings.read().config.appearance;
    let appearance_style = theme::appearance_style(appearance, theme.is_dark());
    let language = crate::desktop::i18n::use_language();
    let activity = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<document::Eval>)));
    let cleanup = activity.clone();
    use_drop(move || {
        if let Some(eval) = cleanup.take() {
            let _ = eval.send(());
        }
    });
    let locked = use_memo(move || matches!(&*state.read(), Screen::Locked(_)));
    let title = use_memo(move || {
        crate::desktop::window::title(&state.read(), settings.read().config.language)
    });
    let shown_title = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(String::new())));

    if *shown_title.borrow() != *title.read() {
        dioxus::desktop::window().set_title(&title.read());
        *shown_title.borrow_mut() = title();
    }
    rsx! {
        style { "{CSS}" }
        main {
            lang: language.code(),
            class: if locked() { "app is-locked" } else { "app" },
            "data-theme": theme.name(),
            "data-preset": appearance.preset.code(),
            "data-corners": appearance.corners.code(),
            style: appearance_style,
            onmounted: move |_| {
                if activity.get().is_none() {
                    let mut eval = document::eval(include_str!("../../assets/continuous-activity.js"));
                    activity.set(Some(eval));
                    spawn(async move {
                        while let Ok(()) = eval.recv::<()>().await {
                            runtime::record_activity();
                        }
                    });
                }
            },
            onpointerdown: move |_| runtime::record_activity(),
            onkeydown: move |event| {
                runtime::record_activity();
                crate::desktop::workspace_view::handle_record_shortcut(state, event);
            },
            crate::desktop::tabs::TabBar {}
            crate::desktop::browser::BrowserConsent {}
            div { id: "space-tab-content", class: "space-tab-content", role: "tabpanel", aria_labelledby: "space-tab-{active_tab}",
                if locked() { LockedView { key: "{active_tab}" } } else { UnlockedView { key: "{active_tab}" } }
            }
        }
    }
}
