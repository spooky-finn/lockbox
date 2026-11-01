use super::*;

#[test]
fn explicit_appearance_overrides_system_and_system_mode_tracks_it() {
    for system_dark in [false, true] {
        assert!(!ThemeMode::Light.is_dark(system_dark));
        assert!(ThemeMode::Dark.is_dark(system_dark));
        assert_eq!(ThemeMode::System.is_dark(system_dark), system_dark);
    }
}

#[test]
fn custom_accents_keep_fills_exact_and_text_readable_on_both_appearances() {
    assert!(appearance_style(Appearance::default(), false).is_empty());
    assert!(appearance_style(Appearance::default(), true).is_empty());
    for color in [
        "#000000", "#ffffff", "#777777", "#ff0000", "#00ff00", "#0000ff", "#af52de",
    ] {
        let accent = RgbColor::parse(color).unwrap();
        for dark in [false, true] {
            let style = appearance_style(
                Appearance {
                    accent: Some(accent),
                    ..Appearance::default()
                },
                dark,
            );
            assert!(style.contains(&format!("--accent:{color};")));
            assert!(style.contains(&format!("--active-item:{color};")));
            let foreground = if style.contains("--accent-text:#000;") {
                [0; 3]
            } else {
                [255; 3]
            };
            assert!(contrast(accent.rgb(), foreground) >= 4.5, "{color}");
            let hover = mix(
                accent.rgb(),
                if foreground == [0; 3] { 255 } else { 0 },
                0.12,
            );
            assert!(contrast(hover, foreground) >= 4.5, "{color} hover");
            let surfaces = if dark {
                [[32, 33, 36], [37, 38, 40], [44, 46, 47]]
            } else {
                [[255; 3], [245, 246, 248], [241, 242, 243]]
            };
            for background in surfaces {
                assert!(
                    contrast(readable_ink(accent.rgb(), dark, &surfaces), background) >= 4.5,
                    "{color} on {background:?}"
                );
            }
        }
    }
}

fn color_variable(style: &str, name: &str) -> [u8; 3] {
    let value = style
        .split(&format!("{name}:"))
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    match value {
        "#fff" => [255; 3],
        "#000" => [0; 3],
        _ => RgbColor::parse(value).unwrap().rgb(),
    }
}

#[test]
fn custom_backgrounds_keep_the_chosen_color_and_readable_text_across_surfaces() {
    for background in [
        "#000000", "#ffffff", "#757575", "#767676", "#777777", "#ff0000", "#00ff00", "#0000ff",
        "#f3e8d2", "#182432",
    ] {
        let background = RgbColor::parse(background).unwrap();
        for accent in [
            None,
            RgbColor::parse("#000000"),
            RgbColor::parse("#ffffff"),
            RgbColor::parse("#af52de"),
        ] {
            let appearance = Appearance {
                accent,
                background: Some(background),
                ..Appearance::default()
            };
            for system_dark in [false, true] {
                let dark = effective_dark(appearance, system_dark);
                let style = appearance_style(appearance, dark);
                for variable in ["--canvas", "--surface", "--lock-canvas"] {
                    assert_eq!(color_variable(&style, variable), background.rgb());
                }
                for text in [
                    "--ink",
                    "--muted",
                    "--lock-ink",
                    "--lock-secondary",
                    "--accent-ink",
                    "--lock-focus",
                    "--error",
                    "--notice",
                    "--success",
                ] {
                    for surface in [
                        "--canvas",
                        "--surface",
                        "--subtle",
                        "--control",
                        "--lock-canvas",
                        "--lock-field",
                    ] {
                        assert!(
                            contrast(
                                color_variable(&style, text),
                                color_variable(&style, surface)
                            ) >= 4.5,
                            "{text} on {surface} with {}",
                            background.hex()
                        );
                    }
                }
                assert!(
                    contrast(
                        color_variable(&style, "--accent"),
                        color_variable(&style, "--accent-text")
                    ) >= 4.5
                );
                assert!(
                    contrast(
                        color_variable(&style, "--error"),
                        color_variable(&style, "--error-text")
                    ) >= 4.5
                );
                if let Some(accent) = accent {
                    assert_eq!(color_variable(&style, "--accent"), accent.rgb());
                }
            }
        }
    }
}

#[test]
fn background_reset_restores_the_preferred_mode_and_keeps_accent() {
    for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
        let mut appearance = Appearance {
            theme: mode,
            accent: RgbColor::parse("#af52de"),
            background: RgbColor::parse("#ffffff"),
        };
        assert!(!effective_dark(appearance, true));
        appearance.background = RgbColor::parse("#000000");
        assert!(effective_dark(appearance, false));
        appearance.background = None;
        for system_dark in [false, true] {
            let dark = effective_dark(appearance, system_dark);
            assert_eq!(dark, mode.is_dark(system_dark));
            let style = appearance_style(appearance, dark);
            assert!(!style.contains("--canvas:"));
            assert_eq!(color_variable(&style, "--accent"), [175, 82, 222]);
        }
    }
}
