//! System appearance stays separate from vault and editor state.

use dioxus::{
    desktop::{
        DesktopContext,
        tao::{
            event::{Event, WindowEvent},
            window::Theme,
        },
        use_window, use_wry_event_handler,
    },
    prelude::*,
};

pub(super) fn use_system_theme() -> &'static str {
    let window = use_window();
    let mut theme = use_signal(|| system_theme(&window));
    let events = window.clone();
    use_wry_event_handler(move |event, _| {
        let next = match event {
            Event::WindowEvent {
                window_id, event, ..
            } if *window_id == events.id() => {
                match event {
                    WindowEvent::ThemeChanged(next) => Some(*next),
                    // Refresh after a change made while the app was inactive.
                    WindowEvent::Focused(true) => Some(system_theme(&events)),
                    _ => None,
                }
            }
            Event::Resumed => Some(system_theme(&events)),
            _ => None,
        };

        if let Some(next) = next
            && *theme.peek() != next
        {
            theme.set(next);
        }
    });
    use_effect(move || {
        // Match --canvas in app.css, including the transparent title bar's backdrop.
        let background = match theme() {
            Theme::Dark => (32, 33, 36, 255),
            _ => (245, 245, 247, 255),
        };
        window.set_background_color(Some(background));
    });

    match theme() {
        Theme::Dark => "dark",
        _ => "light",
    }
}

fn system_theme(window: &DesktopContext) -> Theme {
    // None keeps native chrome automatic and refreshes Tao's cached appearance.
    window.set_theme(None);
    window.theme()
}
