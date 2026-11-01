//! Vault lifecycle. Collaborators are borrowed for transitions, never owned here.

use crate::{
    access::{
        auth::{Locked, Opened},
        biometrics::TouchId,
    },
    desktop::{security::Security, workspace::Unlocked},
    platform::touchid::Biometrics,
    settings::state::Settings,
};

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
            locked.joining = unlocked.sharing.joining;
            locked.notice = reason.map(|reason| format!("Vault locked: {reason}"));
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
        let joining = matches!(self, Self::Locked(form) if form.joining)
            || opened
                .vault
                .accepted_invitations()
                .iter()
                .any(|invitation| !invitation.joined);
        let (touch_id, biometric_error) =
            TouchId::open(&opened.vault, settings, biometrics, opened.with_password);
        let config_error = settings
            .remember(Some(opened.vault.path().to_path_buf()))
            .err();
        let mut unlocked = Unlocked::new(opened.vault);
        unlocked.touch_id = touch_id;
        if joining {
            unlocked.sharing.joining = true;
            unlocked.open_sharing();
        }
        let errors: Vec<_> = biometric_error
            .into_iter()
            .chain(config_error)
            .chain(opened.recovery_notice)
            .collect();
        unlocked.error = (!errors.is_empty()).then(|| errors.join("; "));
        *self = Self::Unlocked(Box::new(unlocked));
        security.activity();
    }
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
