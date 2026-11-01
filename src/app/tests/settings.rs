use super::*;
use crate::core::language::Language;

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
    second.remember(Some(dir.path().join("cloud.lbx"))).unwrap();
    first.set_touch_id("vault-id", true).unwrap();

    let saved = Config::load(&path);
    assert_eq!(saved.language, Language::Russian);
    assert_eq!(saved.auto_lock_minutes, 15);
    assert_eq!(saved.last_vault, Some(dir.path().join("cloud.lbx")));
    assert_eq!(saved.touch_id, vec!["vault-id".to_owned()]);
}
