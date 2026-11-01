use crate::{
    desktop::{session::Screen, window::*, workspace::Unlocked},
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
    },
};
use secrecy::SecretString;
use std::sync::Arc;

#[test]
fn locked_tabs_are_named_after_the_app() {
    let screen = Screen::Locked(Box::new(crate::access::auth::Locked::new(None)));
    assert_eq!(title(&screen, Language::English), "lockbox");
}

#[test]
fn local_tabs_show_the_file_and_cloud_tabs_the_space() {
    let dir = tempfile::tempdir().unwrap();
    let local = Screen::Unlocked(Box::new(crate::desktop::test_support::unlocked(&dir)));
    assert_eq!(title(&local, Language::English), "test — local");
    assert_eq!(title(&local, Language::Russian), "test — локальное");

    let vault = ProfileStore::new(Arc::new(MemoryWitness::default()))
        .create_shared(
            dir.path().join("profile-0123.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Alice".into(),
                space_name: "Семья".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "test-bucket".into(),
                    access_key: "access".into(),
                    secret_key: "secret".into(),
                },
            },
        )
        .unwrap();
    let cloud = Screen::Unlocked(Box::new(Unlocked::new(vault)));
    assert_eq!(title(&cloud, Language::English), "Семья — cloud");
}
