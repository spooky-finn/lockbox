//! Native windows, the application menu, and interface zoom.

use crate::{
    desktop::{App, i18n::Text, session::Screen},
    settings::{language::Language, state::Settings},
};
use dioxus::desktop::{
    Config, LogicalSize, WindowBuilder,
    muda::{
        Menu, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, CMD_OR_CTRL, Code},
    },
};
use dioxus::prelude::*;

pub(crate) const NEW_TAB: &str = "lockbox-new-tab";
pub(crate) const CLOSE_TAB: &str = "lockbox-close-tab";
pub(crate) const NEW_WINDOW: &str = "lockbox-new-window";

/// Configuration of every lockbox window. The application menu is global on
/// macOS, so only the first window installs it.
pub fn config(first: bool) -> Config {
    let window = WindowBuilder::new()
        .with_title("lockbox")
        .with_theme(None)
        .with_inner_size(LogicalSize::new(1040.0, 740.0))
        .with_min_inner_size(LogicalSize::new(360.0, 480.0));
    #[cfg(target_os = "macos")]
    let window = {
        use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
        window
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            // Space tabs share the title bar with the native window buttons.
            .with_fullsize_content_view(true)
            .with_automatic_window_tabbing(false)
    };
    let config = Config::new()
        .with_window(window)
        .with_disable_context_menu(true)
        .with_navigation_handler(|_| false);

    if first || cfg!(not(target_os = "macos")) {
        config
            .with_menu(menu(Settings::load().config.language))
            .with_custom_event_handler(|event, _| {
                if let dioxus::desktop::tao::event::Event::Opened { urls } = event {
                    for url in urls {
                        crate::desktop::browser::receive(url.as_str());
                    }
                }
            })
    } else {
        config.with_menu(None)
    }
}

fn menu(language: Language) -> Menu {
    let shortcut = |key| Some(Accelerator::new(Some(CMD_OR_CTRL), key));
    let menu = Menu::new();
    // On macOS the first submenu becomes the application menu.
    let app = Submenu::new("lockbox", true);
    let file = Submenu::new(language.text(Text::MenuFile), true);
    let edit = Submenu::new(language.text(Text::MenuEdit), true);
    let window = Submenu::new(language.text(Text::MenuWindow), true);
    let built = (|| {
        app.append_items(&[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])?;
        file.append_items(&[
            &MenuItem::with_id(
                NEW_TAB,
                language.text(Text::MenuNewTab),
                true,
                shortcut(Code::KeyT),
            ),
            &MenuItem::with_id(
                NEW_WINDOW,
                language.text(Text::MenuNewWindow),
                true,
                shortcut(Code::KeyN),
            ),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                CLOSE_TAB,
                language.text(Text::MenuCloseTab),
                true,
                shortcut(Code::KeyW),
            ),
        ])?;
        // Without these items macOS WebViews lose copy, paste and undo shortcuts.
        edit.append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::select_all(None),
        ])?;
        window.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::fullscreen(None),
        ])?;
        menu.append_items(&[&app, &file, &edit, &window])
    })();

    if let Err(error) = built {
        dioxus::logger::tracing::warn!("could not build the menu: {error}");
    }

    menu
}

pub(crate) fn use_interface_zoom() {
    let window = dioxus::desktop::use_window();
    let shortcuts = dioxus_core::use_hook_with_cleanup(
        || document::eval(include_str!("../../assets/interface-zoom.js")),
        |shortcuts| {
            let _ = shortcuts.send(());
        },
    );
    use_future(move || {
        let window = window.clone();
        let mut shortcuts = shortcuts;
        async move {
            let mut current_scale = 1.0;
            while let Ok(scale) = shortcuts.recv::<f64>().await {
                crate::desktop::runtime::record_activity();
                if current_scale != scale {
                    window.set_zoom_level(scale);
                    current_scale = scale;
                }
            }
        }
    });
}

/// Opens a separate locked window.
pub(crate) fn open() {
    let pending = dioxus::desktop::window().new_window(VirtualDom::new(App), config(false));
    spawn(async move {
        if let Ok(created) = pending.try_resolve().await {
            created.window.set_focus();
        }
    });
}

/// Window-list and tab tooltip label. Cloud profiles are named after their
/// space because their files use random names under `~/.lockbox`.
pub(crate) fn title(screen: &Screen, config: &crate::settings::config::Config) -> String {
    let Screen::Unlocked(unlocked) = screen else {
        return "lockbox".into();
    };

    let kind = if unlocked.vault.profile_summary().is_some() {
        Text::TabCloud
    } else {
        Text::TabLocal
    };
    let name = screen
        .vault_name(config)
        .unwrap_or_else(|| "lockbox".into());
    format!("{name} — {}", config.language.text(kind))
}

#[cfg(test)]
#[path = "tests/window.rs"]
mod tests;
