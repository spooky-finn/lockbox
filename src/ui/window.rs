//! Native windows and macOS tabs.
//!
//! Every tab is a separate window with its own `VirtualDom`, so each one has an
//! independent vault, auto-lock timer and sync worker. macOS groups the windows
//! into native tabs through a shared tabbing identifier.

use crate::{
    app::{session::Screen, settings::Settings},
    core::language::Language,
    ui::{App, i18n::Text},
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
pub(crate) const NEW_WINDOW: &str = "lockbox-new-window";
#[cfg(target_os = "macos")]
const TABBING_ID: &str = "lockbox";

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
            .with_fullsize_content_view(true)
            .with_tabbing_identifier(TABBING_ID)
    };
    let config = Config::new()
        .with_window(window)
        .with_disable_context_menu(true)
        .with_navigation_handler(|_| false);

    if first || cfg!(not(target_os = "macos")) {
        config.with_menu(menu(Settings::load().config.language))
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
            &PredefinedMenuItem::close_window(Some(language.text(Text::MenuCloseTab))),
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

/// Opens an empty locked window; as a tab of the current one when `tab` is set.
pub(crate) fn open(tab: bool) {
    let current = dioxus::desktop::window();
    let pending = current.new_window(VirtualDom::new(App), config(false));
    spawn(async move {
        let Ok(created) = pending.try_resolve().await else {
            return;
        };

        #[cfg(target_os = "macos")]
        if tab {
            attach_tab(&current.window, &created.window);
        }

        #[cfg(not(target_os = "macos"))]
        let _ = tab;
        created.window.set_focus();
    });
}

/// `addTabbedWindow` groups the windows regardless of the user's
/// "Prefer tabs when opening documents" system setting.
#[cfg(target_os = "macos")]
fn attach_tab(
    host: &dioxus::desktop::tao::window::Window,
    tab: &dioxus::desktop::tao::window::Window,
) {
    use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
    use objc2::{msg_send, runtime::AnyObject};

    const NS_WINDOW_ABOVE: isize = 1;
    let host = host.ns_window().cast::<AnyObject>();
    let tab = tab.ns_window().cast::<AnyObject>();

    if host.is_null() || tab.is_null() || host == tab {
        return;
    }

    // SAFETY: both pointers are live NSWindow objects owned by tao on the main thread.
    unsafe {
        let _: () = msg_send![&*host, addTabbedWindow: &*tab, ordered: NS_WINDOW_ABOVE];
    }
}

/// Native tab and window-list label. Cloud profiles are named after their
/// space because their files use random names under `~/.lockbox`.
pub(crate) fn title(screen: &Screen, language: Language) -> String {
    let Screen::Unlocked(unlocked) = screen else {
        return "lockbox".into();
    };

    let vault = &unlocked.vault;
    let (name, kind) = match vault.profile_summary() {
        Some(profile) => (profile.space_name.to_owned(), Text::TabCloud),
        None => (
            vault
                .path()
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "lockbox".into()),
            Text::TabLocal,
        ),
    };
    format!("{name} — {}", language.text(kind))
}

#[cfg(test)]
#[path = "tests/window.rs"]
mod tests;
