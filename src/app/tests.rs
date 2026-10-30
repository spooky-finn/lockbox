use super::{
    auth::{Locked, with_vault_extension},
    workspace::Unlocked,
};
use crate::touchid::tests::FakeBiometrics;
use crate::{
    config::Config,
    entry::{Totp, TotpAlgorithm},
    vault::{KdfParams, Vault},
};
use secrecy::SecretString;
use std::path::PathBuf;

const FAST: KdfParams = KdfParams {
    memory_kib: 8 * 1024,
    passes: 1,
    lanes: 1,
};

pub(super) fn unlocked(dir: &tempfile::TempDir) -> Unlocked {
    let path = dir.path().join("test.lbx");
    let password = SecretString::from("master".to_owned());
    Unlocked::new(Vault::create(path, &password, FAST).unwrap())
}

pub(super) fn reopen(unlocked: &Unlocked) -> Vault {
    Vault::open(
        unlocked.vault.path(),
        &SecretString::from("master".to_owned()),
    )
    .unwrap()
}

fn add(unlocked: &mut Unlocked, title: &str, username: &str, url: &str) -> String {
    unlocked.start_new();
    let draft = &mut unlocked.draft.as_mut().unwrap().entry;
    draft.title = title.into();
    draft.username = username.into();
    draft.url = url.into();
    let id = draft.id.clone();
    unlocked.save_draft().unwrap();
    id
}

#[test]
fn search_matches_title_username_and_url_ignoring_case() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "GitLab", "a8ee", "https://gitlab.example");
    add(&mut unlocked, "Почта", "inbox", "https://mail.example");
    add(&mut unlocked, "банк", "client", "https://bank.example");
    let titles = |unlocked: &Unlocked| {
        unlocked
            .visible()
            .iter()
            .map(|entry| entry.title.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(&unlocked), ["GitLab", "банк", "Почта"]);
    unlocked.search = "GITLAB".into();
    assert_eq!(titles(&unlocked), ["GitLab"]);
    unlocked.search = "INBOX".into();
    assert_eq!(titles(&unlocked), ["Почта"]);
    unlocked.search = "bank.example".into();
    assert_eq!(titles(&unlocked), ["банк"]);
    unlocked.search = "ПОЧ".into();
    assert_eq!(titles(&unlocked), ["Почта"]);
}

#[test]
fn new_and_edited_entries_are_written_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    let id = add(&mut unlocked, "GitLab", "a8ee", "");
    assert_eq!(unlocked.selected.as_deref(), Some(id.as_str()));
    assert!(unlocked.draft.is_none());

    unlocked.start_edit();
    unlocked.draft.as_mut().unwrap().entry.password = "s3cret".into();
    unlocked.save_draft().unwrap();

    let saved = reopen(&unlocked);
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.entries[0].password, "s3cret");
}

#[test]
fn entry_without_title_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    unlocked.start_new();
    assert!(unlocked.save_draft().is_err());
    assert!(unlocked.draft.is_some());
    assert!(reopen(&unlocked).entries.is_empty());
}

#[test]
fn failed_save_keeps_the_draft_and_the_vault_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "GitLab", "a8ee", "");
    // The temporary file cannot be created inside a directory that is gone.
    std::fs::remove_dir_all(dir.path()).unwrap();
    unlocked.start_new();
    unlocked.draft.as_mut().unwrap().entry.title = "Почта".into();
    assert!(unlocked.save_draft().is_err());
    assert!(unlocked.draft.is_some());
    assert_eq!(unlocked.vault.entries.len(), 1);
}

#[test]
fn deleted_entry_is_removed_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "GitLab", "a8ee", "");
    add(&mut unlocked, "Почта", "inbox", "");
    unlocked.delete_selected().unwrap();
    assert!(unlocked.selected.is_none());
    let saved = reopen(&unlocked);
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.entries[0].title, "GitLab");
}

fn totp_with(secret: &str) -> Totp {
    Totp {
        secret: secret.into(),
        algorithm: TotpAlgorithm::Sha1,
        digits: 6,
        period: 30,
    }
}

#[test]
fn pasted_link_fills_totp_and_empty_fields_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    unlocked.start_new();
    let draft = unlocked.draft.as_mut().unwrap();
    draft.entry.username = "kept".into();
    draft.entry.totp = Some(totp_with(
        "otpauth://totp/GitLab:a8ee?secret=jbsw%20y3dpehpk3pxp&digits=8",
    ));
    draft.apply_link();
    assert_eq!(draft.totp_error, None);
    assert_eq!(draft.entry.title, "GitLab");
    assert_eq!(draft.entry.username, "kept");
    let totp = draft.entry.totp.as_ref().unwrap();
    assert_eq!((totp.secret.as_str(), totp.digits), ("JBSWY3DPEHPK3PXP", 8));
}

#[test]
fn broken_link_is_reported_and_left_for_correction() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    unlocked.start_new();
    let draft = unlocked.draft.as_mut().unwrap();
    draft.entry.totp = Some(totp_with("otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP"));
    draft.apply_link();
    assert!(draft.totp_error.as_deref().unwrap().contains("hotp"));
    assert!(
        draft
            .entry
            .totp
            .as_ref()
            .unwrap()
            .secret
            .starts_with("otpauth://")
    );
}

#[test]
fn invalid_totp_secret_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    unlocked.start_new();
    let draft = unlocked.draft.as_mut().unwrap();
    draft.entry.title = "GitLab".into();
    draft.entry.totp = Some(totp_with("not base32!"));
    assert!(unlocked.save_draft().unwrap_err().starts_with("TOTP:"));
    assert!(reopen(&unlocked).entries.is_empty());
}

#[test]
fn totp_secret_is_saved_normalized() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    unlocked.start_new();
    let draft = unlocked.draft.as_mut().unwrap();
    draft.entry.title = "GitLab".into();
    draft.entry.totp = Some(totp_with("jbsw y3dp ehpk 3pxp"));
    unlocked.save_draft().unwrap();
    let saved = reopen(&unlocked);
    assert_eq!(
        saved.entries[0].totp.as_ref().unwrap().secret,
        "JBSWY3DPEHPK3PXP"
    );
}

#[test]
fn touch_id_is_offered_only_where_it_was_turned_on() {
    let dir = tempfile::tempdir().unwrap();
    let unlocked = unlocked(&dir);
    let path = unlocked.vault.path().to_path_buf();
    let mut config = Config::default();
    let fake = FakeBiometrics::default();

    let locked = Locked::new(Some(path.clone())).offer_touch_id(&config, &fake, true);
    assert!(locked.touch_id.is_none());
    assert!(!locked.auto_touch_id);

    config.touch_id.push(unlocked.vault.id());
    let locked = Locked::new(Some(path.clone())).offer_touch_id(&config, &fake, true);
    assert_eq!(locked.touch_id, Some(unlocked.vault.id()));
    assert!(locked.auto_touch_id);

    *fake.fingers.lock().unwrap() = None;
    let locked = Locked::new(Some(path)).offer_touch_id(&config, &fake, true);
    assert!(locked.touch_id.is_none(), "no sensor, no Touch ID button");
}

#[test]
fn remembered_file_that_disappeared_is_reported() {
    let locked = Locked::new(Some("/nonexistent/vault.lbx".into()));
    assert!(locked.error.unwrap().contains("Файл не найден"));
}

#[test]
fn picked_files_get_the_vault_extension() {
    assert_eq!(
        with_vault_extension("/tmp/secrets".into()),
        PathBuf::from("/tmp/secrets.lbx")
    );
    assert_eq!(
        with_vault_extension("/tmp/vault.lbx".into()),
        PathBuf::from("/tmp/vault.lbx")
    );
}

pub(super) fn security() -> super::security::Security {
    use std::time::{Instant, SystemTime};
    super::security::Security {
        clipboard: crate::clipboard::Guard::new(Box::new(
            crate::clipboard::tests::FakeBoard::default(),
        )),
        auto_lock: crate::autolock::AutoLock::new(5, Instant::now(), SystemTime::now()),
        screen_watch: crate::autolock::ScreenWatch::default(),
    }
}
