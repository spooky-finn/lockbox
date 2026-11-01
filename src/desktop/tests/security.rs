use super::*;
use crate::{
    platform::clipboard::tests::FakeBoard,
    records::entry::{Totp, TotpAlgorithm},
};

#[test]
fn totp_copy_keeps_pending_clipboard_for_missing_or_invalid_settings() {
    let board = FakeBoard::default();
    let mut security = Security {
        clipboard: Guard::new(Box::new(board.clone())),
        auto_lock: AutoLock::new(5, Instant::now(), SystemTime::now()),
        screen_watch: ScreenWatch::default(),
    };
    let now = Instant::now();
    security.clipboard.copy("earlier copy", now).unwrap();
    let pending = security.clipboard.remaining(now);
    let mut entry = Entry::new();

    security.copy(&entry, CopyField::Totp).unwrap();
    assert_eq!(board.0.borrow().as_deref(), Some("earlier copy"));
    assert_eq!(security.clipboard.remaining(now), pending);

    entry.totp = Some(Totp {
        secret: "invalid!".into(),
        algorithm: TotpAlgorithm::Sha256,
        digits: 8,
        period: 30,
    });
    assert_eq!(
        security.copy(&entry, CopyField::Totp),
        Err(totp::TotpError::InvalidSecret.to_string())
    );
    assert_eq!(board.0.borrow().as_deref(), Some("earlier copy"));
    assert_eq!(security.clipboard.remaining(now), pending);

    entry.totp.as_mut().unwrap().secret = "JBSWY3DPEHPK3PXP".into();
    let settings = entry.totp.as_ref().unwrap();
    let before = unix_now();
    security.copy(&entry, CopyField::Totp).unwrap();
    let after = unix_now();
    let copied = board.0.borrow().clone().unwrap();
    assert!(
        copied == totp::code(settings, before).unwrap()
            || copied == totp::code(settings, after).unwrap()
    );
    security.clipboard.clear_now();
    assert_eq!(*board.0.borrow(), None);
}
