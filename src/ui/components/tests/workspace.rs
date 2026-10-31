use crate::app::editor::Field;
use crate::core::entry::{Totp, TotpAlgorithm};
use crate::ui::components::test_support::{mounted, update};
use crate::ui::context::{edit_unlocked, set_field};

#[test]
fn search_reacts_and_failed_save_preserves_the_form() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.search = "missing".into();
            u.start_new();
            u.draft.as_mut().unwrap().entry.totp = Some(Totp {
                secret: "invalid!".into(),
                algorithm: TotpAlgorithm::Sha1,
                digits: 6,
                period: 30,
            });
        })
    });
    update(&mut dom, |state| {
        set_field(state, Field::Title, "Unsaved".into());
        edit_unlocked(state, |u| u.error = u.save_draft().err());
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Записей не найдено"));
    assert!(html.contains("value=\"Unsaved\""));
    assert!(html.contains("TOTP:"));
    assert!(html.contains("Сохранить"));
}

#[test]
fn profile_shows_full_public_fingerprints_and_lock_removes_identity() {
    use crate::app::session::Screen;
    use crate::app::workspace::Unlocked;
    use crate::core::identity::hex;
    use crate::infra::profile::{BootstrapProfile, ProviderConfiguration};
    use crate::infra::vault::{KdfParams, Vault};
    use dioxus::prelude::WritableExt;
    use secrecy::SecretString;
    let dir = tempfile::tempdir().unwrap();
    let fast = KdfParams {
        memory_kib: 8192,
        passes: 1,
        lanes: 1,
    };
    let profile = BootstrapProfile::generate(
        "Alice Profile".into(),
        "Team Profile".into(),
        ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "team-vault".into(),
            access_key: "private-access-key".into(),
            secret_key: "private-secret-key".into(),
        },
        &SecretString::from("recovery password".to_owned()),
        fast,
    )
    .unwrap();
    let fingerprint = hex(&profile.identity.user_id);
    let device = hex(&profile.identity.device_id);
    let vault = Vault::create_profile(
        dir.path().join("profile.lbx"),
        &SecretString::from("master password".to_owned()),
        fast,
        profile,
    )
    .unwrap();
    let mut dom = mounted();
    update(&mut dom, |mut state| {
        state.set(Screen::Unlocked(Box::new(Unlocked::new(vault))))
    });
    let html = dioxus_ssr::render(&dom);
    for value in ["Alice Profile", "Team Profile", &fingerprint, &device] {
        assert!(html.contains(value));
    }
    assert!(!html.contains("private-access-key"));
    assert!(!html.contains("private-secret-key"));
    update(&mut dom, |_| crate::ui::runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(!html.contains(&fingerprint));
    assert!(!html.contains(&device));
    assert!(!html.contains("Alice Profile"));
}
