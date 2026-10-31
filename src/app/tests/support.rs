//! Shared fixtures for application tests.
use super::workspace::Unlocked;
use crate::infra::vault::{KdfParams, Vault};
use secrecy::SecretString;

const FAST: KdfParams = KdfParams {
    memory_kib: 8 * 1024,
    passes: 1,
    lanes: 1,
};

pub(crate) fn unlocked(dir: &tempfile::TempDir) -> Unlocked {
    let path = dir.path().join("test.lbx");
    let password = SecretString::from("master".to_owned());
    Unlocked::new(Vault::create(path, &password, FAST).unwrap())
}

pub(crate) fn reopen(unlocked: &Unlocked) -> Vault {
    Vault::open(
        unlocked.vault.path(),
        &SecretString::from("master".to_owned()),
    )
    .unwrap()
}

pub(crate) fn add(unlocked: &mut Unlocked, title: &str, username: &str, url: &str) -> String {
    unlocked.start_new();
    let draft = &mut unlocked.draft.as_mut().unwrap().entry;
    draft.title = title.into();
    draft.username = username.into();
    draft.url = url.into();
    let id = draft.id.clone();
    unlocked.save_draft().unwrap();
    id
}

pub(crate) fn security() -> super::security::Security {
    use std::time::{Instant, SystemTime};
    super::security::Security {
        clipboard: crate::infra::clipboard::Guard::new(Box::new(
            crate::infra::clipboard::tests::FakeBoard::default(),
        )),
        auto_lock: crate::infra::autolock::AutoLock::new(5, Instant::now(), SystemTime::now()),
        screen_watch: crate::infra::autolock::ScreenWatch::default(),
    }
}
