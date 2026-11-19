//! Native pickers for vault imports and attachment transfers.
//! Delayed results recheck the target vault and record before file access.
//! Vault decryption runs in the session's import worker.

use crate::desktop::{context::Session, session::Screen};
use dioxus::prelude::*;

pub(crate) fn choose_import_file() {
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

pub(crate) fn add_attachment() {
    let state = consume_context::<Session>();
    let selected = match &*state.peek() {
        Screen::Unlocked(u) => u
            .draft
            .as_ref()
            .map(|d| (u.vault.path().to_path_buf(), d.entry.id.clone())),
        _ => None,
    };
    let Some((vault_path, draft_id)) = selected else {
        return;
    };

    // A blocking native dialog can re-enter Dioxus with its callback borrowed.
    spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new().pick_file().await {
            finish_add_attachment(state, &vault_path, &draft_id, path.path());
        }
    });
}

fn finish_add_attachment(
    mut state: Session,
    vault_path: &std::path::Path,
    draft_id: &str,
    path: &std::path::Path,
) {
    let mut screen = state.write();
    let Screen::Unlocked(u) = &mut *screen else {
        return;
    };
    if u.vault.path() != vault_path {
        return;
    }
    let Some(draft) = &mut u.draft else {
        return;
    };
    if draft.entry.id != draft_id {
        return;
    }
    u.error = (|| {
        let (name, mut data) = crate::records::attachments::read(path)?;
        draft
            .entry
            .add_attachment(name, std::mem::take(&mut *data))
            .map_err(str::to_owned)
    })()
    .err();
    if u.error.is_none() {
        draft.schedule_autosave();
    }
}

pub(crate) fn export_attachment(id: &str) {
    let state = consume_context::<Session>();
    let selected = match &*state.peek() {
        Screen::Unlocked(u) => u.selected_entry().and_then(|e| {
            e.attachments
                .iter()
                .find(|f| f.id == id)
                .map(|f| (u.vault.path().to_path_buf(), e.id.clone(), f.name.clone()))
        }),
        _ => None,
    };
    let Some((vault_path, entry_id, name)) = selected else {
        return;
    };
    let id = id.to_owned();

    spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_file_name(&name)
            .save_file()
            .await
        {
            finish_export_attachment(state, &vault_path, &entry_id, &id, path.path());
        }
    });
}

fn finish_export_attachment(
    mut state: Session,
    vault_path: &std::path::Path,
    entry_id: &str,
    id: &str,
    path: &std::path::Path,
) {
    let mut screen = state.write();
    let Screen::Unlocked(u) = &mut *screen else {
        return;
    };
    if u.vault.path() != vault_path {
        return;
    }
    let Some(entry) = u.selected_entry().filter(|e| e.id == entry_id) else {
        return;
    };
    let Some(file) = entry.attachments.iter().find(|f| f.id == id) else {
        return;
    };
    u.error = crate::records::attachments::export(path, &file.data).err();
}

#[cfg(test)]
#[path = "tests/dialogs.rs"]
mod tests;
