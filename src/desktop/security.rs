//! Native clipboard and inactivity monitoring. No dependency on Dioxus.

use crate::{
    desktop::time::unix_now,
    platform::{
        autolock::{self, AutoLock, ScreenWatch},
        clipboard::{Guard, SystemBoard},
    },
    records::{entry::Entry, totp},
};
use std::time::{Instant, SystemTime};
use zeroize::Zeroizing;

pub(crate) struct Security {
    pub clipboard: Guard,
    pub auto_lock: AutoLock,
    pub screen_watch: ScreenWatch,
}

#[derive(Clone, Default, PartialEq)]
pub(crate) struct Status {
    pub lock_seconds: Option<u64>,
    pub clipboard_seconds: Option<u64>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum CopyField {
    Username,
    Password,
    Url,
    Totp,
}

impl Security {
    pub fn new(minutes: u32) -> Self {
        Self {
            clipboard: Guard::new(Box::new(SystemBoard::new())),
            auto_lock: AutoLock::new(minutes, Instant::now(), SystemTime::now()),
            screen_watch: ScreenWatch::start(|| {}),
        }
    }

    pub fn lock_reason(&self) -> Option<&'static str> {
        if self.screen_watch.take() {
            Some("the Mac screen is locked")
        } else if matches!(
            self.auto_lock.check(Instant::now(), SystemTime::now()),
            autolock::State::Lock
        ) {
            Some("no activity for a while")
        } else {
            None
        }
    }

    pub fn activity(&mut self) {
        self.auto_lock.activity(Instant::now(), SystemTime::now());
    }

    pub fn status(&self) -> Status {
        Status {
            lock_seconds: match self.auto_lock.check(Instant::now(), SystemTime::now()) {
                autolock::State::Warning(left) => Some(left.as_secs() + 1),
                _ => None,
            },
            clipboard_seconds: self
                .clipboard
                .remaining(Instant::now())
                .map(|left| left.as_secs() + 1),
        }
    }

    pub fn copy(&mut self, entry: &Entry, field: CopyField) -> Result<(), String> {
        let text = match field {
            CopyField::Username => Zeroizing::new(entry.username.clone()),
            CopyField::Password => Zeroizing::new(entry.password.clone()),
            CopyField::Url => Zeroizing::new(entry.url.clone()),
            CopyField::Totp => match &entry.totp {
                Some(settings) => Zeroizing::new(
                    totp::code(settings, unix_now()).map_err(|error| error.to_string())?,
                ),
                None => return Ok(()),
            },
        };

        self.clipboard
            .copy(&text, Instant::now())
            .map_err(|error| format!("Could not copy: {error}"))
    }
}
