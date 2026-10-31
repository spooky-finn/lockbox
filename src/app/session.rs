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
    Locked(Locked),
    Unlocked(Box<Unlocked>),
}

impl Screen {
    pub fn locked(settings: &Settings, biometrics: &dyn Biometrics) -> Self {
        Self::Locked(
            Locked::new(settings.config.last_vault.clone()).offer_touch_id(
                &settings.config,
                biometrics,
                true,
            ),
        )
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
            *self = Self::Locked(locked);
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
mod tests {
    use super::*;
    use crate::app::tests::{security, unlocked};
    use crate::{infra::config::Config, infra::touchid::tests::FakeBiometrics};

    #[test]
    fn locking_wipes_clipboard_discards_the_draft_and_keeps_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = unlocked(&dir);
        let path = workspace.vault.path().to_path_buf();
        workspace.start_new();
        workspace.draft.as_mut().unwrap().entry.title = "Unsaved".into();
        let mut screen = Screen::Unlocked(Box::new(workspace));
        let settings = Settings {
            config: Config::default(),
            path: None,
        };
        let mut native = security();
        native
            .clipboard
            .copy("secret", std::time::Instant::now())
            .unwrap();
        screen.lock(
            &settings,
            &FakeBiometrics::default(),
            &mut native,
            Some("timeout"),
        );
        assert!(
            native
                .clipboard
                .remaining(std::time::Instant::now())
                .is_none()
        );
        let Screen::Locked(form) = screen else {
            panic!("not locked");
        };
        assert_eq!(form.path.as_deref(), Some(path.as_path()));
        assert!(form.password.is_empty());
        assert!(form.notice.as_deref().unwrap().contains("timeout"));
        assert!(!form.auto_touch_id);
        let reopened = crate::infra::vault::Vault::open(
            &path,
            &secrecy::SecretString::from("master".to_owned()),
        )
        .unwrap();
        assert!(reopened.entries.is_empty());
    }

    #[test]
    fn opening_with_password_restores_biometric_key_and_remembers_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = unlocked(&dir);
        let path = workspace.vault.path().to_path_buf();
        let id = workspace.vault.id();
        let fake = FakeBiometrics::default();
        let mut settings = Settings {
            config: Config {
                touch_id: vec![id.clone()],
                ..Config::default()
            },
            path: None,
        };
        let mut native = security();
        let mut screen = Screen::Locked(Locked::new(None));
        screen.open(
            Opened {
                vault: workspace.vault,
                with_password: true,
            },
            &mut settings,
            &fake,
            &mut native,
        );
        assert!(fake.items.lock().unwrap().contains_key(&id));
        assert_eq!(settings.config.last_vault, Some(path));
        let Screen::Unlocked(workspace) = screen else {
            panic!("not open");
        };
        assert!(workspace.touch_id.enabled);
    }
}
