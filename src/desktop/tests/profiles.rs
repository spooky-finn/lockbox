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

#[test]
fn renaming_updates_the_open_session_and_persisted_profile_path() {
    use crate::desktop::{runtime, view_test_support};

    let directory = tempfile::tempdir().unwrap();
    let preferences = directory.path().join("config.json");
    let mut dom = view_test_support::mounted();
    view_test_support::update(&mut dom, |state| {
        let source = match &*state.peek() {
            Screen::Unlocked(workspace) => workspace.vault.path().to_path_buf(),
            Screen::Locked(_) => panic!("expected an open vault"),
        };
        let mut settings = consume_context::<Signal<Settings>>();
        let mut settings = settings.write();
        settings.path = Some(preferences.clone());
        settings.remember_profile(source.clone(), None).unwrap();
        drop(settings);

        rename_vault_for_me("My personal vault").unwrap();
        rename_local_vault("Personal").unwrap();
        let destination = source.with_file_name("Personal.lbx");
        let saved = Config::load(&preferences);
        assert_eq!(saved.last_vault, Some(destination.clone()));
        assert_eq!(saved.recent_vaults, vec![destination.clone()]);
        assert_eq!(saved.vault_name(&destination, None), "My personal vault");
        runtime::lock(None);
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("expected password unlock");
        };
        assert_eq!(form.path.as_ref(), Some(&destination));
        assert_eq!(
            screen.vault_name(&saved).as_deref(),
            Some("My personal vault")
        );
        assert!(destination.is_file());
        assert!(!source.exists());
    });
}
