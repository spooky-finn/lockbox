//! Resolve saved appearance against the system theme and build readable color palettes.

use crate::settings::{
    appearance::{Appearance, RgbColor, ThemeMode},
    state::Settings,
};

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

#[derive(Clone, Copy)]
pub(crate) struct ResolvedTheme(Memo<bool>);

impl ResolvedTheme {
    pub(crate) fn is_dark(self) -> bool {
        (self.0)()
    }

    pub(crate) fn name(self) -> &'static str {
        if self.is_dark() { "dark" } else { "light" }
    }
}

pub(crate) fn use_theme() -> ResolvedTheme {
    let settings = use_context::<Signal<Settings>>();
    let window = use_window();
    let mut theme = use_signal(|| window.theme());
    let dark =
        use_memo(move || effective_dark(settings.read().config.appearance, theme() == Theme::Dark));
    let resolved = ResolvedTheme(dark);
    use_context_provider(|| resolved);
    let events = window.clone();
    use_wry_event_handler(move |event, _| {
        let appearance = settings.peek().config.appearance;
        if appearance.theme != ThemeMode::System || appearance.background.is_some() {
            return;
        }
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
    let native = window.clone();
    use_effect(move || {
        let appearance = settings.read().config.appearance;
        let requested = if appearance.background.is_some() {
            Some(if dark() { Theme::Dark } else { Theme::Light })
        } else {
            match appearance.theme {
                ThemeMode::System => None,
                ThemeMode::Light => Some(Theme::Light),
                ThemeMode::Dark => Some(Theme::Dark),
            }
        };
        native.set_theme(requested);
        // Refresh the system value after leaving a forced light/dark appearance.
        if requested.is_none() {
            let next = native.theme();
            if *theme.peek() != next {
                theme.set(next);
            }
        }
    });
    use_effect(move || {
        // Match --canvas in app.css, including the transparent title bar's backdrop.
        let [red, green, blue] =
            backgrounds(settings.read().config.appearance.background, dark())[0];
        window.set_background_color(Some((red, green, blue, 255)));
    });

    resolved
}

pub(crate) fn effective_dark(appearance: Appearance, system_dark: bool) -> bool {
    appearance.background.map_or_else(
        || appearance.theme.is_dark(system_dark),
        |background| luminance(background.rgb()) <= 0.179,
    )
}

fn system_theme(window: &DesktopContext) -> Theme {
    // None keeps native chrome automatic and refreshes Tao's cached appearance.
    window.set_theme(None);
    window.theme()
}

pub(crate) fn appearance_style(appearance: Appearance, dark: bool) -> String {
    let mut style = appearance
        .background
        .map_or_else(String::new, |color| background_style(color, dark));
    // Default accents also need readable text when the background is customized.
    let accent = appearance.accent.or_else(|| {
        appearance
            .background
            .and_then(|_| RgbColor::parse(if dark { "#409cff" } else { "#007aff" }))
    });
    style.push_str(&accent_style(
        accent,
        dark,
        &backgrounds(appearance.background, dark),
    ));
    style
}

fn backgrounds(background: Option<RgbColor>, dark: bool) -> [[u8; 3]; 3] {
    if let Some(background) = background {
        let rgb = background.rgb();
        let target = if dark { 0 } else { 255 };
        [rgb, mix(rgb, target, 0.05), mix(rgb, target, 0.10)]
    } else if dark {
        [[32, 33, 36], [37, 38, 40], [44, 46, 47]]
    } else {
        [[245, 245, 247], [255; 3], [241, 242, 243]]
    }
}

fn background_style(background: RgbColor, dark: bool) -> String {
    let surfaces = backgrounds(Some(background), dark);
    let canvas = background.hex();
    let subtle = hex(surfaces[1]);
    let control = hex(surfaces[2]);
    let ink = if dark { "#fff" } else { "#000" };
    let channel = if dark { 255 } else { 0 };
    let readable = |rgb| hex(readable_ink(rgb, dark, &surfaces));
    let muted = readable(if dark {
        [161, 167, 181]
    } else {
        [113, 119, 134]
    });
    let error_rgb = readable_ink(
        if dark { [255, 133, 133] } else { [196, 53, 56] },
        dark,
        &surfaces,
    );
    let error = hex(error_rgb);
    let error_text = if luminance(error_rgb) > 0.179 {
        "#000"
    } else {
        "#fff"
    };
    let notice = readable(if dark { [239, 188, 107] } else { [146, 91, 7] });
    let success = readable(if dark { [50, 215, 75] } else { [36, 138, 61] });
    format!(
        "--canvas:{canvas};--surface:{canvas};--subtle:{subtle};--control:{control};\
         --glass:{control};--glass-solid:{control};\
         --glass-edge:rgb({channel} {channel} {channel} / 10%);\
         --ink:{ink};--muted:{muted};--error:{error};--error-text:{error_text};--notice:{notice};--success:{success};\
         --border:rgb({channel} {channel} {channel} / 18%);\
         --separator:rgb({channel} {channel} {channel} / 12%);\
         --lock-canvas:{canvas};--lock-ink:{ink};--lock-secondary:{muted};\
         --lock-field:{subtle};--lock-field-border:rgb({channel} {channel} {channel} / 28%);"
    )
}

fn accent_style(accent: Option<RgbColor>, dark: bool, backgrounds: &[[u8; 3]]) -> String {
    let Some(accent) = accent else {
        return String::new();
    };
    let rgb = accent.rgb();
    let [red, green, blue] = rgb;
    let foreground = if luminance(rgb) > 0.179 {
        "#000"
    } else {
        "#fff"
    };
    let ink = readable_ink(rgb, dark, backgrounds);
    let hover = mix(rgb, if foreground == "#000" { 255 } else { 0 }, 0.12);
    let opacity = if dark { "18%" } else { "11%" };
    format!(
        "--accent:{};--accent-hover:{};--accent-text:{foreground};\
         --accent-ink:{};--selected:rgb({red} {green} {blue} / {opacity});\
         --selected-text:{};--active-item:{};--active-item-text:{foreground};\
         --accent-shadow:rgb({red} {green} {blue} / 17%);\
         --lock-focus:{};--lock-focus-shadow:rgb({red} {green} {blue} / 45%);",
        accent.hex(),
        hex(hover),
        hex(ink),
        hex(ink),
        accent.hex(),
        hex(ink),
    )
}

fn hex([red, green, blue]: [u8; 3]) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

fn mix(rgb: [u8; 3], target: u8, amount: f64) -> [u8; 3] {
    rgb.map(|value| (f64::from(value) * (1.0 - amount) + f64::from(target) * amount).round() as u8)
}

fn luminance(rgb: [u8; 3]) -> f64 {
    let linear = rgb.map(|value| {
        let channel = f64::from(value) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    });
    linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}

fn contrast(first: [u8; 3], second: [u8; 3]) -> f64 {
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn readable_ink(rgb: [u8; 3], dark: bool, backgrounds: &[[u8; 3]]) -> [u8; 3] {
    // Keep custom color fills exact. Adjust text and focus rings for each surface.
    for step in 0..=20 {
        let candidate = mix(rgb, if dark { 255 } else { 0 }, f64::from(step) / 20.0);
        if backgrounds
            .iter()
            .all(|background| contrast(candidate, *background) >= 4.5)
        {
            return candidate;
        }
    }
    if dark { [255; 3] } else { [0; 3] }
}

#[cfg(test)]
#[path = "../tests/theme.rs"]
mod tests;
