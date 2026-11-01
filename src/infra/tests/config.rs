use super::*;

#[test]
fn remembered_path_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lockbox").join("config.json");
    let config = Config {
        language: crate::core::language::Language::English,
        last_vault: Some("/tmp/vault.lbx".into()),
        touch_id: vec!["00ff".into()],
        auto_lock_minutes: 15,
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
    assert_eq!(config.language, crate::core::language::Language::English);
    assert!(config.touch_id.is_empty());
    assert_eq!(
        config.auto_lock_minutes,
        crate::infra::autolock::DEFAULT_MINUTES
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
    use crate::core::language::Language;

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
