use crate::app::test_support::{reopen, unlocked};
use crate::core::entry::{Totp, TotpAlgorithm};

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
