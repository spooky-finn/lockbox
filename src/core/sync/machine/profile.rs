//! Explicit protocol version boundaries. Downgrades never resume old authority.

use crate::core::sync::{
    Error, Result,
    crypto::{CHANGE_DOMAIN, CONTROL_DOMAIN},
};

pub(super) const LEGACY_PROFILE: &str = "LBX-SYNC-0";
pub(super) const PROFILE: &str = "LBX-SYNC-1";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Profile {
    Legacy,
    Current,
}

impl Profile {
    pub(super) fn parse(profile: &str) -> Result<Self> {
        match profile {
            LEGACY_PROFILE => Ok(Self::Legacy),
            PROFILE => Ok(Self::Current),
            _ => Err(Error::UnsupportedProtocol),
        }
    }

    pub(super) fn control_domain(self) -> &'static [u8] {
        match self {
            Self::Legacy => CONTROL_DOMAIN,
            Self::Current => b"lockbox.sync.control.v1\0",
        }
    }

    pub(super) fn change_domain(self) -> &'static [u8] {
        match self {
            Self::Legacy => CHANGE_DOMAIN,
            Self::Current => b"lockbox.sync.change.v1\0",
        }
    }
}
