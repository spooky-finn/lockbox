//! Vault lifecycle. Collaborators are borrowed for transitions, never owned here.

use super::{
    auth::{Locked, Opened},
    biometrics::TouchId,
    security::Security,
    settings::Settings,
    workspace::Unlocked,
};
use crate::infra::touchid::Biometrics;

pub(crate) enum Screen {
    Locked(Box<Locked>),
    Unlocked(Box<Unlocked>),
}

impl Screen {
    pub fn locked(settings: &Settings, biometrics: &dyn Biometrics) -> Self {
        Self::Locked(Box::new(
            Locked::new(settings.config.last_vault.clone()).offer_touch_id(
                &settings.config,
                biometrics,
                true,
            ),
        ))
    }

    pub fn lock(
        &mut self,
        settings: &Settings,
        biometrics: &dyn Biometrics,
        security: &mut Security,
        reason: Option<&str>,
    ) {
        security.clipboard.clear_now();
        if let Self::Unlocked(unlocked) = self {
            let mut locked = Locked::new(Some(unlocked.vault.path().to_path_buf())).offer_touch_id(
                &settings.config,
                biometrics,
                false,
            );
            locked.notice = reason.map(|reason| format!("Хранилище заблокировано: {reason}"));
            *self = Self::Locked(Box::new(locked));
        }
    }

    pub fn open(
        &mut self,
        opened: Opened,
        settings: &mut Settings,
        biometrics: &dyn Biometrics,
        security: &mut Security,
    ) {
        let (touch_id, biometric_error) =
            TouchId::open(&opened.vault, settings, biometrics, opened.with_password);
        let config_error = settings
            .remember(Some(opened.vault.path().to_path_buf()))
            .err();
        let mut unlocked = Unlocked::new(opened.vault);
        unlocked.touch_id = touch_id;
        let errors: Vec<_> = biometric_error.into_iter().chain(config_error).collect();
        unlocked.error = (!errors.is_empty()).then(|| errors.join("; "));
        *self = Self::Unlocked(Box::new(unlocked));
        security.activity();
    }
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
