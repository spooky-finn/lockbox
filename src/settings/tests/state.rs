use crate::settings::{language::Language, state::*};

fn at(path: &std::path::Path) -> Settings {
    Settings {
        config: Config::load(path),
        path: Some(path.to_path_buf()),
    }
}

#[test]
fn tabs_keep_each_others_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut first = at(&path);
    let mut second = at(&path);

    first.update(|c| c.language = Language::Russian).unwrap();
    second.update(|c| c.auto_lock_minutes = 15).unwrap();
    second
        .remember_profile(dir.path().join("cloud.lbx"), Some("Семья".into()))
        .unwrap();
    first.set_touch_id("vault-id", true).unwrap();

    let saved = Config::load(&path);
    assert_eq!(saved.language, Language::Russian);
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
    first.remember(None).unwrap();
    let forgotten = Config::load(&path);
    assert!(forgotten.last_vault.is_none());
    assert!(forgotten.space_names.is_empty());
}
