//! Dioxus adapters for preferences and security settings.
//! Settings owns persistence; desktop security owns active timers and biometrics.

use crate::{
    desktop::{
        context::{BiometricProvider, SecurityHandle, Session},
        runtime::publish_status,
        session::Screen,
    },
    platform::autolock,
    settings::state::Settings,
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

pub(crate) fn set_theme(theme: crate::settings::appearance::ThemeMode) {
    update_preferences(|config| config.appearance.theme = theme);
}

pub(crate) fn set_theme_preset(preset: crate::settings::appearance::ThemePreset) {
    update_preferences(|config| {
        config.appearance.preset = preset;
        config.appearance.accent = None;
        config.appearance.background = None;
    });
}

pub(crate) fn set_gradient(enabled: bool) {
    update_preferences(|config| config.appearance.gradient = enabled);
}

#[cfg(test)]
pub(crate) fn set_accent_color(value: Option<&str>) {
    set_color(value, |config, color| config.appearance.accent = color);
}

#[cfg(test)]
pub(crate) fn set_background_color(value: Option<&str>) {
    set_color(value, |config, color| config.appearance.background = color);
}

#[cfg(test)]
fn set_color(
    value: Option<&str>,
    change: impl FnOnce(
        &mut crate::settings::config::Config,
        Option<crate::settings::appearance::RgbColor>,
    ),
) {
    let color = match value {
        Some(value) => {
            let Some(color) = crate::settings::appearance::RgbColor::parse(value) else {
                return;
            };
            Some(color)
        }
        None => None,
    };
    update_preferences(|config| change(config, color));
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
