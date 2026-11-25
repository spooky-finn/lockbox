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
        vault_aliases: [("/tmp/vault.lbx".into(), "Home".into())].into(),
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
fn legacy_appearance_keeps_the_preset_and_mode() {
    use crate::settings::appearance::{Appearance, ThemeMode, ThemePreset};

    let config: Config =
        serde_json::from_str(r#"{"language":"ru","auto_lock_minutes":15}"#).unwrap();
    assert_eq!(config.appearance, Appearance::default());

    let legacy: Config = serde_json::from_value(serde_json::json!({
        "language": "ru",
        "auto_lock_minutes": 15,
        "appearance": {
            "theme": "dark",
            "preset": "paper",
            "accent": "#af52de",
            "background": "#f3e8d2",
            "gradient": true,
            "corners": "rounded"
        }
    }))
    .unwrap();
    assert_eq!(legacy.language, config.language);
    assert_eq!(legacy.auto_lock_minutes, config.auto_lock_minutes);
    assert_eq!(
        legacy.appearance,
        Appearance {
            theme: ThemeMode::Dark,
            preset: ThemePreset::Paper,
        }
    );
    assert_eq!(
        serde_json::to_value(legacy.appearance).unwrap(),
        serde_json::json!({"theme": "dark", "preset": "paper"})
    );
}

#[test]
fn every_preset_and_mode_survives_a_restart() {
    use crate::settings::appearance::{Appearance, ThemeMode, ThemePreset};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    for preset in ThemePreset::ALL {
        for theme in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            let config = Config {
                appearance: Appearance { theme, preset },
                ..Config::default()
            };
            config.save(&path).unwrap();
            assert_eq!(Config::load(&path), config);
            let json: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(json["appearance"]["theme"], theme.code());
            assert_eq!(json["appearance"]["preset"], preset.code());
        }
    }

    let unknown: Appearance =
        serde_json::from_str(r#"{"theme":"future","preset":"future"}"#).unwrap();
    assert_eq!(unknown, Appearance::default());
}
