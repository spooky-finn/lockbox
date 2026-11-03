use crate::settings::config::*;

#[test]
fn remembered_path_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lockbox").join("config.json");
    let config = Config {
        language: crate::settings::language::Language::English,
        last_vault: Some("/tmp/vault.lbx".into()),
        recent_vaults: vec!["/tmp/vault.lbx".into()],
        touch_id: vec!["00ff".into()],
        auto_lock_minutes: 15,
        website_icons: true,
        space_names: [("/tmp/vault.lbx".into(), "Family".into())].into(),
        appearance: crate::settings::appearance::Appearance::default(),
    };
    config.save(&path).unwrap();
    assert_eq!(Config::load(&path), config);
}

#[test]
fn config_from_before_touch_id_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, br#"{"last_vault":"/tmp/vault.lbx"}"#).unwrap();
    let config = Config::load(&path);
    assert_eq!(
        config.last_vault.as_deref(),
        Some(Path::new("/tmp/vault.lbx"))
    );
    assert_eq!(
        config.language,
        crate::settings::language::Language::English
    );
    assert!(config.touch_id.is_empty());
    assert!(config.space_names.is_empty());
    assert!(!config.website_icons);
    assert_eq!(
        config.auto_lock_minutes,
        crate::platform::autolock::DEFAULT_MINUTES
    );
}

#[test]
fn missing_or_broken_config_remembers_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    assert_eq!(Config::load(&path), Config::default());
    fs::write(&path, b"{not json").unwrap();
    assert_eq!(Config::load(&path), Config::default());
}

#[cfg(unix)]
#[test]
fn config_file_is_private_to_the_owner() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    Config::default().save(&path).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn language_defaults_to_english_and_preserves_explicit_russian() {
    use crate::settings::language::Language;

    assert_eq!(Config::default().language, Language::English);
    for (code, expected) in [
        ("en", Language::English),
        ("ru", Language::Russian),
        ("unknown", Language::English),
    ] {
        let config: Config = serde_json::from_value(serde_json::json!({"language": code})).unwrap();
        assert_eq!(config.language, expected);
    }
}

#[test]
fn appearance_defaults_and_invalid_colors_preserve_existing_preferences() {
    use crate::settings::appearance::{Appearance, ThemeMode};

    let legacy: Config =
        serde_json::from_str(r#"{"language":"ru","auto_lock_minutes":15}"#).unwrap();
    assert_eq!(legacy.appearance, Appearance::default());
    for color in ["red", "#123", "#fffffg", "#123456;--canvas:red", "#ééé"] {
        let config: Config = serde_json::from_value(serde_json::json!({
            "language": "ru",
            "auto_lock_minutes": 15,
            "appearance": {"theme": "unknown", "accent": color, "background": color},
        }))
        .unwrap();
        assert_eq!(config.language, legacy.language);
        assert_eq!(config.auto_lock_minutes, 15);
        assert_eq!(config.appearance.theme, ThemeMode::System);
        assert_eq!(config.appearance.accent, None);
        assert_eq!(config.appearance.background, None);
    }
}

#[test]
fn appearance_survives_a_restart() {
    use crate::settings::appearance::{Appearance, RgbColor, ThemeMode};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    for theme in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
        let config = Config {
            appearance: Appearance {
                theme,
                accent: RgbColor::parse("#AF52De"),
                background: RgbColor::parse("#F3E8D2"),
            },
            ..Config::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path), config);
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["appearance"]["accent"], "#af52de");
        assert_eq!(json["appearance"]["background"], "#f3e8d2");
    }
}

#[test]
fn accent_only_preferences_load_without_a_background() {
    use crate::settings::appearance::{RgbColor, ThemeMode};
    let config: Config =
        serde_json::from_str(r##"{"appearance":{"theme":"dark","accent":"#af52de"}}"##).unwrap();
    assert_eq!(config.appearance.theme, ThemeMode::Dark);
    assert_eq!(config.appearance.accent, RgbColor::parse("#af52de"));
    assert_eq!(config.appearance.background, None);
}
