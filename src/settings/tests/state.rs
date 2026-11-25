use crate::settings::{
    appearance::{ThemeMode, ThemePreset},
    language::Language,
    state::*,
};

fn at(path: &std::path::Path) -> Settings {
    Settings {
        config: Config::load(path),
        path: Some(path.to_path_buf()),
    }
}

#[test]
fn alias_overrides_shared_renames_until_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    let vault_path = dir.path().join("cloud.lbx");
    let mut settings = at(&config_path);
    settings
        .remember_profile(vault_path.clone(), Some("Shared".into()))
        .unwrap();
    settings.set_vault_alias(&vault_path, "  Mine  ").unwrap();
    settings
        .cache_space_name(&vault_path, "New shared")
        .unwrap();
    let mut settings = at(&config_path);
    assert_eq!(settings.config.vault_name(&vault_path, None), "Mine");
    settings.set_vault_alias(&vault_path, "  ").unwrap();
    let saved = at(&config_path);
    assert_eq!(saved.config.vault_name(&vault_path, None), "New shared");
    assert_eq!(saved.config.last_vault, Some(vault_path));
}

#[test]
fn tabs_keep_each_others_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut first = at(&path);
    let mut second = at(&path);

    first.update(|c| c.language = Language::Russian).unwrap();
    first
        .update(|c| c.appearance.theme = ThemeMode::Dark)
        .unwrap();
    second
        .update(|c| c.appearance.preset = ThemePreset::Paper)
        .unwrap();
    second.update(|c| c.auto_lock_minutes = 15).unwrap();
    second
        .remember_profile(dir.path().join("cloud.lbx"), Some("Семья".into()))
        .unwrap();
    first.set_touch_id("vault-id", true).unwrap();

    let saved = Config::load(&path);
    assert_eq!(saved.language, Language::Russian);
    assert_eq!(saved.appearance.theme, ThemeMode::Dark);
    assert_eq!(saved.appearance.preset, ThemePreset::Paper);
    assert_eq!(saved.auto_lock_minutes, 15);
    assert_eq!(saved.last_vault, Some(dir.path().join("cloud.lbx")));
    assert_eq!(saved.touch_id, vec!["vault-id".to_owned()]);
    assert_eq!(
        saved
            .space_names
            .get(&dir.path().join("cloud.lbx"))
            .map(String::as_str),
        Some("Семья")
    );
}

#[test]
fn profiles_survive_restart_include_legacy_cloud_names_and_do_not_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let local = dir.path().join("personal.lbx");
    let cloud = dir.path().join("cloud.lbx");
    let legacy = dir.path().join("legacy-cloud.lbx");
    let mut first = at(&path);
    first
        .update(|config| {
            config
                .space_names
                .insert(legacy.clone(), "Legacy team".into());
        })
        .unwrap();
    first.remember_profile(local.clone(), None).unwrap();
    let mut second = at(&path);
    second
        .remember_profile(cloud.clone(), Some("Team".into()))
        .unwrap();
    first.remember_profile(local.clone(), None).unwrap();
    let restarted = at(&path);
    assert_eq!(
        restarted.known_profiles(),
        vec![local.clone(), cloud.clone(), legacy.clone()]
    );
    assert_eq!(
        restarted.config.recent_vaults,
        vec![local.clone(), cloud.clone()]
    );
}
