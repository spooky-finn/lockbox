//! Resolve the native light/dark mode and supply palette tokens for the app and previews.

use crate::{
    desktop::context::Session,
    settings::{
        appearance::{Appearance, ThemeMode, ThemePreset},
        state::Settings,
    },
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
        mode_name(self.is_dark())
    }
}

pub(crate) fn mode_name(dark: bool) -> &'static str {
    if dark { "dark" } else { "light" }
}

pub(crate) fn use_active_appearance() -> Memo<Appearance> {
    let settings = use_context::<Signal<Settings>>();
    let session = use_context::<Session>();
    let vault_id = use_memo(move || session.read().vault_id());
    use_memo(move || {
        settings
            .read()
            .config
            .appearance_for(vault_id.read().as_deref())
    })
}

pub(crate) fn use_theme(appearance: Memo<Appearance>) -> ResolvedTheme {
    let window = use_window();
    let mut theme = use_signal(|| window.theme());
    let dark = use_memo(move || appearance().theme.is_dark(theme() == Theme::Dark));
    let resolved = ResolvedTheme(dark);
    use_context_provider(|| resolved);
    let events = window.clone();
    use_wry_event_handler(move |event, _| {
        if appearance.peek().theme != ThemeMode::System {
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
        let requested = match appearance().theme {
            ThemeMode::System => None,
            ThemeMode::Light => Some(Theme::Light),
            ThemeMode::Dark => Some(Theme::Dark),
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
        // Match the canvas, including the transparent title bar's backdrop.
        let (canvas, _) = colors(appearance().preset, dark());
        let [red, green, blue] = canvas;
        window.set_background_color(Some((red, green, blue, 255)));
    });

    resolved
}

fn system_theme(window: &DesktopContext) -> Theme {
    // None keeps native chrome automatic and refreshes Tao's cached appearance.
    window.set_theme(None);
    window.theme()
}

type Rgb = [u8; 3];

// Canvas and accent. The same palette is used in the preview, app, and native window.
fn colors(preset: ThemePreset, dark: bool) -> (Rgb, Rgb) {
    use ThemePreset::*;
    match (preset, dark) {
        (Classic, false) => ([246, 241, 229], [67, 59, 47]),
        (Classic, true) => ([30, 28, 24], [215, 200, 173]),
        (Paper, false) => ([249, 242, 221], [43, 78, 134]),
        (Paper, true) => ([37, 33, 27], [165, 191, 248]),
        (Forest, false) => ([235, 238, 221], [51, 82, 56]),
        (Forest, true) => ([26, 35, 28], [166, 201, 145]),
        (Ocean, false) => ([229, 239, 249], [35, 79, 150]),
        (Ocean, true) => ([19, 33, 52], [142, 185, 248]),
        (Slate, false) => ([237, 239, 238], [62, 77, 82]),
        (Slate, true) => ([32, 37, 39], [188, 203, 210]),
        (Amber, false) => ([236, 218, 190], [119, 62, 29]),
        (Amber, true) => ([42, 31, 23], [233, 174, 117]),
        (Rose, false) => ([250, 236, 233], [142, 49, 69]),
        (Rose, true) => ([44, 28, 33], [244, 165, 178]),
        (Teal, false) => ([225, 241, 235], [27, 89, 75]),
        (Teal, true) => ([19, 38, 34], [133, 224, 199]),
        (Plum, false) => ([240, 231, 244], [93, 52, 116]),
        (Plum, true) => ([32, 25, 39], [216, 171, 239]),
        (Indigo, false) => ([232, 236, 229], [34, 88, 54]),
        (Indigo, true) => ([16, 25, 21], [139, 227, 168]),
    }
}

pub(crate) fn preset_style(preset: ThemePreset, dark: bool) -> String {
    let (canvas, accent) = colors(preset, dark);
    let (paper, ink_channel) = if dark { (0, 255) } else { (255, 0) };
    let surface = mix(canvas, 255, if dark { 0.04 } else { 0.40 });
    let subtle = mix(canvas, ink_channel, if dark { 0.08 } else { 0.04 });
    let control = mix(canvas, paper, if dark { 0.12 } else { 0.65 });
    let selected_opacity = if dark { 18 } else { 11 };
    let accent_fill = blend(subtle, accent, f64::from(selected_opacity) / 100.0);
    let surfaces = [canvas, surface, subtle, control, accent_fill];
    let readable = |rgb| hex(readable_ink(rgb, dark, &surfaces));
    let ink = hex(mix(canvas, ink_channel, 0.88));
    let muted = readable(if dark { [167, 173, 182] } else { [93, 91, 96] });
    let error = hex(readable_ink(
        if dark { [255, 153, 153] } else { [163, 40, 43] },
        dark,
        &surfaces,
    ));
    let error_text = if dark { "#000" } else { "#fff" };
    let notice = readable(if dark { [239, 188, 107] } else { [128, 74, 16] });
    let success = readable(if dark { [123, 217, 150] } else { [36, 108, 61] });
    let accent_ink = readable(accent);
    let accent_text = if dark { "#000" } else { "#fff" };
    let hover = mix(accent, ink_channel, 0.12);
    let [red, green, blue] = accent;
    format!(
        "--canvas:{};--surface:{};--subtle:{};--control:{};\
         --glass:{};--glass-solid:{};--glass-edge:rgb({ink_channel} {ink_channel} {ink_channel} / 10%);\
         --ink:{ink};--muted:{muted};--error:{error};--error-text:{error_text};--notice:{notice};--success:{success};\
         --border:rgb({ink_channel} {ink_channel} {ink_channel} / 18%);\
         --separator:rgb({ink_channel} {ink_channel} {ink_channel} / 12%);\
         --lock-canvas:{};--lock-ink:{ink};--lock-secondary:{muted};\
         --lock-field:{};--lock-field-border:rgb({ink_channel} {ink_channel} {ink_channel} / 28%);\
         --accent:{};--accent-hover:{};--accent-text:{accent_text};--accent-ink:{accent_ink};\
         --selected:rgb({red} {green} {blue} / {selected_opacity}%);--selected-text:{accent_ink};\
         --active-item:{};--active-item-text:{accent_text};--accent-shadow:rgb({red} {green} {blue} / 17%);\
         --lock-focus:{accent_ink};",
        hex(canvas),
        hex(surface),
        hex(subtle),
        hex(control),
        hex(control),
        hex(control),
        hex(canvas),
        hex(surface),
        hex(accent),
        hex(hover),
        hex(accent),
    )
}

fn hex([red, green, blue]: [u8; 3]) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

fn mix(rgb: [u8; 3], target: u8, amount: f64) -> [u8; 3] {
    blend(rgb, [target; 3], amount)
}

fn blend(rgb: Rgb, target: Rgb, amount: f64) -> Rgb {
    std::array::from_fn(|index| {
        (f64::from(rgb[index]) * (1.0 - amount) + f64::from(target[index]) * amount).round() as u8
    })
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
    // Leave room for the low-opacity patterns that sit on top of these surfaces.
    for step in 0..=20 {
        let candidate = mix(rgb, if dark { 255 } else { 0 }, f64::from(step) / 20.0);
        if backgrounds
            .iter()
            .all(|background| contrast(candidate, *background) >= 5.5)
        {
            return candidate;
        }
    }
    if dark { [255; 3] } else { [0; 3] }
}

#[cfg(test)]
#[path = "../tests/theme.rs"]
mod tests;
