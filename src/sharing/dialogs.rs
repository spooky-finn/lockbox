//! Native dialogs return only to their originating unlocked profile. File I/O
//! belongs to a cancellable session worker, never the Dioxus event callback.

use crate::desktop::{context::Session, session::Screen};
use dioxus::prelude::*;

pub(crate) fn import_sharing_file() {
    let mut state = consume_context::<Session>();
    let ticket = match &*state.peek() {
        Screen::Unlocked(u) if !u.sharing.busy() => u.sharing.ticket(&u.vault),
        _ => return,
    };
    spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .add_filter("Lockbox", &["json"])
            .pick_file()
            .await
            && let Screen::Unlocked(u) = &mut *state.write()
        {
            u.import_sharing(ticket, path.path().to_path_buf());
        }
    });
}

pub(crate) fn export_sharing_file() {
    let mut state = consume_context::<Session>();
    let (ticket, output) = match &*state.peek() {
        Screen::Unlocked(u) if !u.sharing.busy() => {
            let Some(output) = u
                .sharing
                .output
                .as_ref()
                .or(u.sharing.saved_output.as_ref())
                .cloned()
            else {
                return;
            };
            (u.sharing.ticket(&u.vault), output)
        }
        _ => return,
    };
    spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .add_filter("Lockbox", &["json"])
            .set_file_name(output.kind.file_name())
            .save_file()
            .await
            && let Screen::Unlocked(u) = &mut *state.write()
        {
            u.export_sharing(ticket, path.path().to_path_buf(), output);
        }
    });
}
