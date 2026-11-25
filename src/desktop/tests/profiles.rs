use super::*;
use crate::settings::config::Config;

fn settings_with(dir: &Path, names: &[&str]) -> (Settings, Vec<PathBuf>) {
    let mut config = Config::load(&dir.join("config.json"));
    let paths: Vec<_> = names
        .iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(&path, b"fictional").unwrap();
            path
        })
        .collect();
    config.recent_vaults = paths.clone();
    config.last_vault = paths.first().cloned();
    (Settings { config, path: None }, paths)
}

#[test]
fn carousel_wraps_between_vaults() {
    let dir = tempfile::tempdir().unwrap();
    let (settings, paths) = settings_with(dir.path(), &["a.lbx", "b.lbx", "c.lbx"]);

    assert_eq!(
        adjacent_profiles(&settings, None, &[]),
        (Some(paths[2].clone()), Some(paths[0].clone()))
    );
    assert_eq!(
        adjacent_profiles(&settings, Some(&paths[0]), &[]),
        (Some(paths[2].clone()), Some(paths[1].clone()))
    );
    assert_eq!(
        adjacent_profiles(&settings, Some(&paths[1]), &[]),
        (Some(paths[0].clone()), Some(paths[2].clone()))
    );
    assert_eq!(
        adjacent_profiles(&settings, Some(&paths[2]), &[]),
        (Some(paths[1].clone()), Some(paths[0].clone()))
    );
}

#[test]
fn carousel_skips_vaults_open_in_other_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let (settings, paths) = settings_with(dir.path(), &["a.lbx", "b.lbx"]);

    assert_eq!(
        adjacent_profiles(&settings, None, std::slice::from_ref(&paths[0])),
        (Some(paths[1].clone()), Some(paths[1].clone()))
    );
    assert_eq!(adjacent_profiles(&settings, None, &paths), (None, None));
}

#[test]
fn missing_files_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let (mut settings, paths) = settings_with(dir.path(), &["a.lbx"]);
    settings
        .config
        .recent_vaults
        .push(dir.path().join("gone.lbx"));

    assert_eq!(
        adjacent_profiles(&settings, Some(&paths[0]), &[]),
        (None, None)
    );
}
