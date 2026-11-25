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
use std::time::{Instant, SystemTime};

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
    if !enabled {
        let mut state = consume_context::<Session>();
        if let Screen::Unlocked(workspace) = &mut *state.write() {
            workspace.favicon_cache = Default::default();
        }
    }
}

pub(crate) fn set_theme(theme: ThemeMode) {
    update_appearance(|appearance| appearance.theme = theme);
}

pub(crate) fn set_theme_preset(preset: ThemePreset) {
    update_appearance(|appearance| appearance.preset = preset);
}

pub(crate) fn set_border_radius(radius: Option<u8>) {
    update_appearance(|appearance| appearance.border_radius = radius);
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
