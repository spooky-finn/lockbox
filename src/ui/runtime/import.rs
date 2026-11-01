//! Native file picker for importing another vault. Decryption runs in the
//! session's import worker, never in the Dioxus event callback.

use crate::{app::session::Screen, ui::context::Session};
use dioxus::prelude::*;

pub(in crate::ui) fn choose_import_file() {
    let mut state = consume_context::<Session>();

    if !matches!(&*state.peek(), Screen::Unlocked(u) if !u.import.busy()) {
        return;
    }

    let session = match &*state.peek() {
        Screen::Unlocked(u) => u.vault.path().to_path_buf(),
        _ => return,
    };
    spawn(async move {
        if let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Lockbox", &["lbx"])
            .pick_file()
            .await
            && let Screen::Unlocked(u) = &mut *state.write()
            // The vault may have been locked and another one opened meanwhile.
            && u.vault.path() == session
        {
            u.choose_import(file.path().to_path_buf());
        }
    });
}
