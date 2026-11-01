//! Entry collection, selection and transactional save/delete.

use super::{biometrics::TouchId, directories::Directories, editor::Draft};
use crate::{core::entry::Entry, core::generator, core::totp, infra::vault::Vault};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Vault,
    Settings,
}

pub(crate) struct Unlocked {
    pub(crate) page: Page,
    pub(crate) sync: super::space_sync::SpaceSync,
    pub(crate) sync_recovery: zeroize::Zeroizing<String>,
    pub(crate) vault: Vault,
    pub(crate) touch_id: TouchId,
    pub(crate) search: String,
    pub(crate) selected: Option<String>,
    pub(crate) draft: Option<Draft>,
    pub(crate) directories: Directories,
    pub(crate) show_password: bool,
    pub(crate) confirm_delete: bool,
    pub(crate) error: Option<String>,
    /// Generator settings last until the window closes; they are never saved.
    pub(crate) generator: generator::Options,
}

impl Unlocked {
    pub(crate) fn new(vault: Vault) -> Self {
        Self {
            page: Page::Vault,
            sync: super::space_sync::SpaceSync::default(),
            sync_recovery: zeroize::Zeroizing::new(String::new()),
            vault,
            touch_id: TouchId::default(),
            search: String::new(),
            selected: None,
            draft: None,
            directories: Directories::default(),
            show_password: false,
            confirm_delete: false,
            error: None,
            generator: generator::Options::default(),
        }
    }

    pub(crate) fn selected_entry(&self) -> Option<&Entry> {
        let id = self.selected.as_ref()?;
        self.vault.entries.iter().find(|entry| &entry.id == id)
    }

    pub(crate) fn activate_sync(&mut self) {
        let recovery = secrecy::SecretString::from(std::mem::take(&mut *self.sync_recovery));
        self.error = self
            .vault
            .activate_sync(&recovery)
            .err()
            .map(|error| error.to_string());
        if self.error.is_none() {
            self.sync.saved();
        }
    }

    pub(crate) fn select(&mut self, id: String) {
        self.page = Page::Vault;
        if let Some(entry) = self.vault.entries.iter().find(|entry| entry.id == id) {
            self.directories.selected = entry.folder_id.clone();
            self.directories.reveal(&self.vault);
        }
        self.directories.confirm_delete = false;
        self.selected = Some(id);
        self.show_password = false;
        self.confirm_delete = false;
        self.error = None;
    }

    pub(crate) fn start_new(&mut self) {
        if self.is_editing() {
            return;
        }
        let mut entry = Entry::new();
        entry.folder_id = self.directories.selected.clone();
        self.start_draft(entry, true);
    }

    pub(crate) fn is_editing(&self) -> bool {
        self.draft.is_some() || self.directories.draft.is_some()
    }

    pub(crate) fn start_edit(&mut self) {
        if let Some(entry) = self.selected_entry().cloned() {
            self.start_draft(entry, false);
        }
    }

    pub(crate) fn start_draft(&mut self, entry: Entry, is_new: bool) {
        self.page = Page::Vault;
        self.draft = Some(Draft {
            entry,
            is_new,
            show_password: false,
            show_totp: false,
            totp_error: None,
            generator_open: false,
            generator_error: None,
            icon_picker_open: false,
            icon_query: String::new(),
        });
        self.confirm_delete = false;
        self.error = None;
    }

    pub(crate) fn open_settings(&mut self) {
        self.page = Page::Settings;
        self.show_password = false;
        self.confirm_delete = false;
        self.directories.confirm_delete = false;
    }

    /// Writes the draft to disk. On failure the vault is unchanged and the draft is kept.
    pub(crate) fn save_draft(&mut self) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        if draft.entry.title.trim().is_empty() {
            return Err("Enter an entry name".into());
        }
        if let Some(totp) = &draft.entry.totp {
            totp::validate(totp).map_err(|error| format!("TOTP: {error}"))?;
        }
        let mut entry = draft.entry.clone();
        if let Some(totp) = &mut entry.totp {
            totp.secret = totp::normalize_secret(&totp.secret);
        }
        let id = entry.id.clone();
        let previous = self.vault.entries.clone();
        match self.vault.entries.iter_mut().find(|e| e.id == id) {
            Some(slot) => *slot = entry,
            None => self.vault.entries.push(entry),
        }
        if let Err(error) = self.vault.save_changes() {
            self.vault.entries = previous;
            return Err(format!("Could not save: {error}"));
        }
        self.draft = None;
        self.search.clear();
        self.select(id);
        self.sync.saved();
        Ok(())
    }

    pub(crate) fn delete_selected(&mut self) -> Result<(), String> {
        let Some(id) = self.selected.clone() else {
            return Ok(());
        };
        let previous = self.vault.entries.clone();
        self.vault.entries.retain(|entry| entry.id != id);
        if let Err(error) = self.vault.save_changes() {
            self.vault.entries = previous;
            return Err(format!("Could not save: {error}"));
        }
        self.selected = None;
        self.confirm_delete = false;
        self.sync.saved();
        Ok(())
    }
}

pub(crate) fn matches(entry: &Entry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || [&entry.title, &entry.username, &entry.url]
            .iter()
            .any(|field| field.to_lowercase().contains(&query))
}

#[cfg(test)]
#[path = "tests/workspace.rs"]
mod tests;
