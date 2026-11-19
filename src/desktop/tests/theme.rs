use super::*;

#[test]
fn presets_have_readable_text_and_fills_in_both_modes() {
    for preset in ThemePreset::ALL {
        for dark in [false, true] {
            let style = preset_style(preset, dark);
            assert_eq!(color_variable(&style, "--canvas"), colors(preset, dark).0);
            for text in [
                "--ink",
                "--muted",
                "--accent-ink",
                "--error",
                "--notice",
                "--success",
                "--lock-ink",
                "--lock-secondary",
                "--lock-focus",
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
                        "{preset:?}, dark={dark}: {text} on {surface}"
                    );
                }
            }
            for (text, fill) in [
                ("--accent-text", "--accent"),
                ("--accent-text", "--accent-hover"),
                ("--active-item-text", "--active-item"),
                ("--error-text", "--error"),
            ] {
                assert!(
                    contrast(color_variable(&style, text), color_variable(&style, fill)) >= 4.5,
                    "{preset:?}, dark={dark}: {text} on {fill}"
                );
            }
        }
    }
}

#[test]
fn explicit_appearance_overrides_system_and_system_mode_tracks_it() {
    for system_dark in [false, true] {
        assert!(!ThemeMode::Light.is_dark(system_dark));
        assert!(ThemeMode::Dark.is_dark(system_dark));
        assert_eq!(ThemeMode::System.is_dark(system_dark), system_dark);
    }
}

fn color_variable(style: &str, name: &str) -> Rgb {
    let value = style
        .split(';')
        .find_map(|property| property.trim().strip_prefix(name)?.strip_prefix(":#"))
        .unwrap();
    match value {
        "fff" => [255; 3],
        "000" => [0; 3],
        _ => std::array::from_fn(|index| {
            u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap()
        }),
    }
}
