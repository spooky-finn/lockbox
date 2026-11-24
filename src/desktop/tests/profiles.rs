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

fn vault(path: &Path) -> Option<CarouselStop> {
    Some(CarouselStop::Vault(path.to_path_buf()))
}

#[test]
fn carousel_walks_every_known_vault_then_the_open_or_create_page() {
    let dir = tempfile::tempdir().unwrap();
    let (settings, paths) = settings_with(dir.path(), &["a.lbx", "b.lbx", "c.lbx"]);

    assert_eq!(
        adjacent_profiles(&settings, None, &[]),
        (vault(&paths[2]), vault(&paths[0]))
    );
    assert_eq!(
        adjacent_profiles(&settings, Some(&paths[0]), &[]),
        (Some(CarouselStop::Add), vault(&paths[1]))
    );
    assert_eq!(
        adjacent_profiles(&settings, Some(&paths[1]), &[]),
        (vault(&paths[0]), vault(&paths[2]))
    );
    assert_eq!(
        adjacent_profiles(&settings, Some(&paths[2]), &[]),
        (vault(&paths[1]), Some(CarouselStop::Add))
    );
}

#[test]
fn carousel_skips_vaults_open_in_other_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let (settings, paths) = settings_with(dir.path(), &["a.lbx", "b.lbx"]);

    assert_eq!(
        adjacent_profiles(&settings, None, std::slice::from_ref(&paths[0])),
        (vault(&paths[1]), vault(&paths[1]))
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
        (Some(CarouselStop::Add), Some(CarouselStop::Add))
    );
}
