//! Independent Dioxus contexts: vault session, native services and the display clock.
use crate::app::{
    auth::Locked,
    editor::{Draft, Field},
    security::Security,
    session::Screen,
    workspace::Unlocked,
};
use crate::touchid::Biometrics;
use dioxus::prelude::*;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub(in crate::app) type Session = Signal<Screen>;
pub(in crate::app) type SecurityHandle = Rc<RefCell<Security>>;
#[derive(Clone)]
pub(in crate::app) struct BiometricProvider(pub Arc<dyn Biometrics>);
#[derive(Clone, Copy)]
pub(in crate::app) struct Clock(pub Signal<u64>);

pub(in crate::app) fn edit_locked(mut state: Session, edit: impl FnOnce(&mut Locked)) {
    if let Screen::Locked(locked) = &mut *state.write() {
        edit(locked);
    }
}
pub(in crate::app) fn edit_unlocked(mut state: Session, edit: impl FnOnce(&mut Unlocked)) {
    if let Screen::Unlocked(unlocked) = &mut *state.write() {
        edit(unlocked);
    }
}
pub(in crate::app) fn edit_draft(state: Session, edit: impl FnOnce(&mut Draft)) {
    edit_unlocked(state, |unlocked| {
        if let Some(draft) = &mut unlocked.draft {
            edit(draft);
        }
    });
}
pub(in crate::app) fn set_field(state: Session, field: Field, value: String) {
    edit_draft(state, |draft| draft.set_field(field, value));
}
