//! Dioxus adapters for preferences and security settings.
//! Settings owns persistence; desktop security owns active timers and biometrics.

use crate::{
    desktop::{
        context::{BiometricProvider, SecurityHandle, Session},
        runtime::publish_status,
        session::Screen,
    },
    platform::autolock,
    settings::{
        appearance::{Appearance, ThemeMode, ThemePreset},
        state::Settings,
    },
};
use dioxus::prelude::*;
use std::{
    cell::Cell,
    time::{Instant, SystemTime},
};

/// Auto-lock and website-icon choices made in any window. Each window keeps
/// its own Settings copy and timer, so the others adopt these on their next
/// tick. All windows run on the main thread.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shared {
    auto_lock_minutes: u32,
    website_icons: bool,
}

thread_local! {
    static SHARED: Cell<Option<Shared>> = const { Cell::new(None) };
}

fn share(settings: &Settings) {
    SHARED.set(Some(Shared {
        auto_lock_minutes: settings.config.auto_lock_minutes,
        website_icons: settings.config.website_icons,
    }));
}

/// Applies a choice made in another window to this one. Kept in memory only:
/// the window that made the change already saved it.
pub(crate) fn adopt_shared() {
    let Some(shared) = SHARED.get() else {
        return;
    };
    let mut settings = consume_context::<Signal<Settings>>();
    let (minutes, icons) = {
        let current = &settings.peek().config;
        (current.auto_lock_minutes, current.website_icons)
    };

    if minutes != shared.auto_lock_minutes {
        settings.write().config.auto_lock_minutes = shared.auto_lock_minutes;
        consume_context::<SecurityHandle>()
            .borrow_mut()
            .auto_lock
            .set_minutes(shared.auto_lock_minutes, Instant::now(), SystemTime::now());
    }
    if icons != shared.website_icons {
        settings.write().config.website_icons = shared.website_icons;
        if !shared.website_icons {
            forget_all_favicons();
            // Downloads this window had in flight may have written meanwhile.
            crate::records::favicon::clear_disk();
        }
    }
}

/// Drops the icon cache of every tab in this window, so no download still in
/// flight in a background tab writes to disk after icons are turned off.
fn forget_all_favicons() {
    // Tabs include the active session; tests may mount without tabs.
    match try_consume_context::<Signal<crate::desktop::tabs::Tabs>>() {
        Some(tabs) => tabs
            .peek()
            .sessions
            .iter()
            .copied()
            .for_each(forget_favicons),
        None => forget_favicons(consume_context::<Session>()),
    }
}

fn forget_favicons(mut state: Session) {
    if let Screen::Unlocked(workspace) = &mut *state.write() {
        workspace.favicon_cache = Default::default();
    }
}

pub(crate) fn set_touch_id(wanted: bool) {
    let mut state = consume_context::<Session>();
    let mut settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    let biometrics = hardware.for_language(settings.peek().config.language);

    if let Screen::Unlocked(workspace) = &mut *state.write()
        && !workspace.saving_draft()
    {
        workspace.error =
            workspace
                .touch_id
                .set(&workspace.vault, &mut settings.write(), biometrics, wanted);
    }
}

pub(crate) fn set_auto_lock(minutes: u32) {
    if !autolock::CHOICES.contains(&minutes) {
        return;
    }

    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    let native = consume_context::<SecurityHandle>();
    let saved = settings
        .write()
        .update(|config| config.auto_lock_minutes = minutes);
    native
        .borrow_mut()
        .auto_lock
        .set_minutes(minutes, Instant::now(), SystemTime::now());
    share(&settings.peek());

    if let Screen::Unlocked(workspace) = &mut *state.write() {
        workspace.error = saved.err();
    }
    publish_status();
}

pub(crate) fn set_language(language: crate::settings::language::Language) {
    update_preferences(|config| config.language = language);
}

pub(crate) fn set_website_icons(enabled: bool) {
    update_preferences(|config| config.website_icons = enabled);
    share(&consume_context::<Signal<Settings>>().peek());
    if !enabled {
        forget_all_favicons();
        // After the session caches are gone, so their downloads no longer write.
        crate::records::favicon::clear_disk();
    }
}

pub(crate) fn set_theme(theme: ThemeMode) {
    update_appearance(|appearance| appearance.theme = theme);
}

pub(crate) fn set_theme_preset(preset: ThemePreset) {
    update_appearance(|appearance| appearance.preset = preset);
}

pub(crate) fn set_border_radius(radius: Option<u8>) {
    update_preferences(|config| config.appearance.border_radius = radius);
}

fn update_appearance(change: impl FnOnce(&mut Appearance)) {
    let state = consume_context::<Session>();
    let id = match &*state.peek() {
        Screen::Unlocked(workspace) => workspace.vault.id(),
        Screen::Locked(_) => return,
    };
    update_preferences(|config| {
        change(
            config
                .vault_appearances
                .entry(id)
                .or_insert(config.appearance),
        );
    });
}

fn update_preferences(change: impl FnOnce(&mut crate::settings::config::Config)) {
    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    let saved = settings.write().update(change);

    if let Err(error) = saved {
        match &mut *state.write() {
            Screen::Locked(form) => form.error = Some(error),
            Screen::Unlocked(workspace) => workspace.error = Some(error),
        }
    }
}
