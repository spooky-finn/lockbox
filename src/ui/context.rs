//! Independent Dioxus contexts: vault session, native services and the display clock.

use crate::app::{
    auth::Locked,
    editor::{Draft, Field},
    security::Security,
    session::Screen,
    workspace::Unlocked,
};
use crate::infra::touchid::Biometrics;
use dioxus::prelude::*;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub(crate) type Session = Signal<Screen>;
pub(crate) type SecurityHandle = Rc<RefCell<Security>>;
#[derive(Clone)]
pub(crate) struct BiometricProvider(pub Arc<dyn Biometrics>);

impl BiometricProvider {
    /// Keep the native authentication prompt in the selected interface language.
    pub(crate) fn for_language(
        &self,
        language: crate::core::language::Language,
    ) -> Arc<dyn Biometrics> {
        Arc::new(LocalizedBiometrics {
            inner: self.0.clone(),
            reason: language.text(crate::ui::i18n::Text::UnlockLockbox),
        })
    }
}

struct LocalizedBiometrics {
    inner: Arc<dyn Biometrics>,
    reason: &'static str,
}

impl Biometrics for LocalizedBiometrics {
    fn domain_state(&self) -> Option<Vec<u8>> {
        self.inner.domain_state()
    }

    fn authenticate(&self, _reason: &str) -> Result<Vec<u8>, String> {
        self.inner.authenticate(self.reason)
    }

    fn load(&self, account: &str) -> Option<zeroize::Zeroizing<Vec<u8>>> {
        self.inner.load(account)
    }

    fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
        self.inner.store(account, data)
    }

    fn delete(&self, account: &str) {
        self.inner.delete(account);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Clock(pub Signal<u64>);

pub(crate) fn edit_locked(mut state: Session, edit: impl FnOnce(&mut Locked)) {
    if let Screen::Locked(locked) = &mut *state.write() {
        edit(locked);
    }
}
pub(crate) fn edit_unlocked(mut state: Session, edit: impl FnOnce(&mut Unlocked)) {
    if let Screen::Unlocked(unlocked) = &mut *state.write() {
        edit(unlocked);
    }
}
pub(crate) fn edit_draft(state: Session, edit: impl FnOnce(&mut Draft)) {
    edit_unlocked(state, |unlocked| {
        if let Some(draft) = &mut unlocked.draft {
            edit(draft);
        }
    });
}
pub(crate) fn set_field(state: Session, field: Field, value: String) {
    edit_draft(state, |draft| draft.set_field(field, value));
}
