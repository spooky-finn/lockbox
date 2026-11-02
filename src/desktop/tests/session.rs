use crate::{
    desktop::{
        session::*,
        test_support::{security, unlocked},
    },
    platform::touchid::tests::FakeBiometrics,
    settings::config::Config,
};

#[test]
fn locking_wipes_clipboard_discards_the_draft_and_keeps_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let path = workspace.vault.path().to_path_buf();
    workspace.start_new();
    workspace.draft.as_mut().unwrap().entry.title = "Unsaved".into();
    let mut screen = Screen::Unlocked(Box::new(workspace));
    let settings = Settings {
        config: Config::default(),
        path: None,
    };
    let mut native = security();
    native
        .clipboard
        .copy("secret", std::time::Instant::now())
        .unwrap();
    screen.lock(
        &settings,
        &FakeBiometrics::default(),
        &mut native,
        Some("timeout"),
    );
    assert!(
        native
            .clipboard
            .remaining(std::time::Instant::now())
            .is_none()
    );
    let Screen::Locked(form) = screen else {
        panic!("not locked");
    };
    assert_eq!(form.path.as_deref(), Some(path.as_path()));
    assert!(form.password.is_empty());
    assert!(form.notice.as_deref().unwrap().contains("timeout"));
    assert!(!form.auto_touch_id);
    let reopened =
        crate::vault::Vault::open(&path, &secrecy::SecretString::from("master".to_owned()))
            .unwrap();
    assert!(reopened.entries().is_empty());
}

#[test]
fn opening_with_password_restores_biometric_key_and_remembers_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = unlocked(&dir);
    let path = workspace.vault.path().to_path_buf();
    let id = workspace.vault.id();
    let fake = FakeBiometrics::default();
    let mut settings = Settings {
        config: Config {
            touch_id: vec![id.clone()],
            ..Config::default()
        },
        path: None,
    };
    let mut native = security();
    let mut screen = Screen::Locked(Box::new(Locked::new(None)));
    screen.open(
        Opened {
            vault: workspace.vault,
            with_password: true,
            recovery_notice: None,
        },
        &mut settings,
        &fake,
        &mut native,
    );
    assert!(fake.items.lock().unwrap().contains_key(&id));
    assert_eq!(settings.config.last_vault, Some(path));
    let Screen::Unlocked(workspace) = screen else {
        panic!("not open");
    };
    assert!(workspace.touch_id.enabled);
}

#[test]
fn joining_opens_sharing_and_survives_a_session_lock() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = unlocked(&dir);
    let mut settings = Settings {
        config: Config::default(),
        path: None,
    };
    let fake = FakeBiometrics::default();
    let mut native = security();
    let mut screen = Screen::Locked(Box::new(Locked::begin_join()));
    screen.open(
        Opened {
            vault: workspace.vault,
            with_password: true,
            recovery_notice: None,
        },
        &mut settings,
        &fake,
        &mut native,
    );
    let Screen::Unlocked(workspace) = &screen else {
        panic!("not open")
    };
    assert!(workspace.page == crate::desktop::workspace::Page::Sharing);
    assert!(workspace.sharing.joining);
    screen.lock(&settings, &fake, &mut native, None);
    let Screen::Locked(form) = &screen else {
        panic!("not locked")
    };
    assert!(form.joining);
}

#[test]
fn pending_link_resumes_joining_without_the_original_creation_screen() {
    use crate::vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
    };
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(std::sync::Arc::new(MemoryWitness::default()));
    let create = |name: &str| {
        store
            .create_shared(
                directory.path().join(format!("{name}.lbx")),
                &secrecy::SecretString::from("master".to_owned()),
                &secrecy::SecretString::from("recovery".to_owned()),
                KdfParams {
                    memory_kib: 8192,
                    passes: 1,
                    lanes: 1,
                },
                SharedProfile {
                    username: name.into(),
                    space_name: "Team".into(),
                    provider: ProviderConfiguration {
                        base_url: "https://gateway.storjshare.io".into(),
                        bucket: "fixture".into(),
                        access_key: "fictional".into(),
                        secret_key: "fictional".into(),
                    },
                },
            )
            .unwrap()
    };
    let mut owner = create("owner");
    let mut guest = create("guest");
    owner.create_link(0, 1).unwrap();
    let ticket = owner.issued_links().next().unwrap().ticket.clone();
    guest
        .request_link(
            &ticket.link().unwrap(),
            &ticket.verification_code().unwrap().to_string(),
            0,
        )
        .unwrap();
    let mut settings = Settings {
        config: Config::default(),
        path: None,
    };
    let mut screen = Screen::Locked(Box::new(Locked::new(None)));
    screen.open(
        Opened {
            vault: guest,
            with_password: true,
            recovery_notice: None,
        },
        &mut settings,
        &FakeBiometrics::default(),
        &mut security(),
    );
    let Screen::Unlocked(workspace) = screen else {
        panic!("profile did not open")
    };
    assert!(workspace.sharing.joining);
    assert!(workspace.page == crate::desktop::workspace::Page::Sharing);
    assert!(workspace.vault.pairing_waits_to_join());
}

#[test]
fn space_name_is_available_after_lock_restart_and_selecting_the_profile() {
    use crate::vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
    };
    use secrecy::SecretString;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile-random-id.lbx");
    let name = "Семья <Family>";
    let vault = ProfileStore::new(Arc::new(MemoryWitness::default()))
        .create_shared(
            &path,
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Alice".into(),
                space_name: name.into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "test-bucket".into(),
                    access_key: "private-access-marker".into(),
                    secret_key: "private-secret-marker".into(),
                },
            },
        )
        .unwrap();
    let config_path = dir.path().join("config.json");
    let mut settings = Settings {
        config: Config::default(),
        path: Some(config_path.clone()),
    };
    let fake = FakeBiometrics::default();
    let mut native = security();
    let mut screen = Screen::Locked(Box::new(Locked::new(None)));
    screen.open(
        Opened {
            vault,
            with_password: true,
            recovery_notice: None,
        },
        &mut settings,
        &fake,
        &mut native,
    );
    screen.lock(&settings, &fake, &mut native, None);
    let Screen::Locked(form) = screen else {
        panic!("not locked")
    };
    assert_eq!(form.space_name.as_deref(), Some(name));

    let settings = Settings {
        config: Config::load(&config_path),
        path: Some(config_path.clone()),
    };
    let Screen::Locked(form) = Screen::locked(&settings, &fake) else {
        panic!("not locked")
    };
    assert_eq!(form.space_name.as_deref(), Some(name));
    let selected = Locked::new(Some(path)).offer_touch_id(&settings.config, &fake, false);
    assert_eq!(selected.space_name.as_deref(), Some(name));
    let config = std::fs::read_to_string(config_path).unwrap();
    assert!(!config.contains("private-access-marker"));
    assert!(!config.contains("private-secret-marker"));
}
