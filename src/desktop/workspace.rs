//! Entry collection, selection and transactional save/delete.

use crate::{
    access::biometrics::TouchId,
    desktop::draft_save::DraftSaves,
    records::{directories::Directories, editor::Draft, entry::Entry, generator},
    vault::Vault,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Vault,
    Settings,
    Sharing,
    History,
    Trash,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Destination {
    Close,
    NewEntry,
    NewEntryInFolder(String),
    NewDirectory,
    Entry(String),
    Directory(Option<String>),
    ToggleDirectory(String),
    Page(Page),
}

pub(crate) struct Unlocked {
    pub(crate) favicon_cache: crate::records::favicon::Cache,
    pub(crate) page: Page,
    pub(crate) sync: crate::sync::scheduler::SpaceSync,
    pub(crate) sharing: crate::sharing::actions::Sharing,
    pub(crate) import: crate::records::import::VaultImport,
    pub(crate) sync_recovery: zeroize::Zeroizing<String>,
    pub(crate) vault: Vault,
    pub(crate) touch_id: TouchId,
    pub(crate) search: String,
    pub(crate) selected: Option<String>,
    pub(crate) draft: Option<Draft>,
    pub(super) draft_saves: DraftSaves,
    pub(crate) directories: Directories,
    pub(crate) show_password: bool,
    pub(crate) confirm_delete: bool,
    pub(crate) error: Option<String>,
    /// UI-only event counter, incremented after the first durable record save.
    pub(crate) record_created: u64,
    /// One-shot presentation event for the newly opened workspace.
    pub(super) unlock_spell: std::cell::Cell<Option<std::time::Instant>>,
    /// Generator settings last until the window closes; they are never saved.
    pub(crate) generator: generator::Options,
}

impl Unlocked {
    pub(crate) fn new(vault: Vault) -> Self {
        let directories = Directories {
            collapsed: vault
                .folders()
                .iter()
                .map(|folder| folder.id.clone())
                .collect(),
            ..Directories::default()
        };
        Self {
            favicon_cache: Default::default(),
            page: Page::Vault,
            sync: crate::sync::scheduler::SpaceSync::default(),
            sharing: crate::sharing::actions::Sharing::default(),
            import: crate::records::import::VaultImport::default(),
            sync_recovery: zeroize::Zeroizing::new(String::new()),
            vault,
            touch_id: TouchId::default(),
            search: String::new(),
            selected: None,
            draft: None,
            draft_saves: DraftSaves::default(),
            directories,
            show_password: false,
            confirm_delete: false,
            error: None,
            record_created: 0,
            unlock_spell: Default::default(),
            generator: generator::Options::default(),
        }
    }

    pub(super) fn finish_navigation(&mut self, destination: Destination) {
        self.draft = None;
        self.show_password = false;
        self.error = None;
        match destination {
            Destination::Close => {
                self.selected = None;
                self.confirm_delete = false;
            }
            Destination::NewEntry => self.start_new(),
            Destination::NewEntryInFolder(id) => {
                if self.vault.signing_error().is_none()
                    && self.vault.folders().iter().any(|folder| folder.id == id)
                {
                    self.start_new_in(Some(id));
                }
            }
            Destination::NewDirectory => self.start_directory(),
            Destination::Entry(id) => self.select(id),
            Destination::Directory(id) => {
                let collapsed = id
                    .as_ref()
                    .filter(|id| self.directories.collapsed.contains(*id))
                    .cloned();
                self.select_directory(id);
                // Keyboard selection leaves the selected branch's disclosure unchanged.
                if let Some(id) = collapsed {
                    self.directories.collapsed.insert(id);
                }
            }
            Destination::ToggleDirectory(id) => {
                let collapse =
                    self.search.trim().is_empty() && !self.directories.collapsed.contains(&id);
                self.select_directory(Some(id.clone()));
                // Selection reveals ancestors; restore the requested closed state.
                if collapse {
                    self.directories.collapsed.insert(id);
                }
            }
            Destination::Page(Page::Settings) => self.open_settings(),
            Destination::Page(Page::Sharing) => self.open_sharing(),
            Destination::Page(page) => self.open_records_page(page),
        }
    }

    pub(crate) fn edit_field(&mut self, field: crate::records::editor::Field) {
        if self.vault.signing_error().is_some() {
            return;
        }
        self.start_edit();
        if let Some(draft) = &mut self.draft {
            draft.focus_field = Some(field);
        }
    }

    pub(crate) fn selected_entry(&self) -> Option<&Entry> {
        let id = self.selected.as_ref()?;
        self.vault.entries().iter().find(|entry| &entry.id == id)
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
        if !self.editor_is_clear() {
            return;
        }
        self.select_entry(id);
    }

    fn select_entry(&mut self, id: String) {
        self.page = Page::Vault;

        if let Some(entry) = self.vault.entries().iter().find(|entry| entry.id == id) {
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
        self.start_new_in(self.directories.selected.clone());
    }

    fn start_new_in(&mut self, folder_id: Option<String>) {
        if !self.editor_is_clear() {
            return;
        }

        let mut entry = Entry::new();
        entry.folder_id = folder_id;
        self.start_draft(entry, true);
    }

    pub(crate) fn is_editing(&self) -> bool {
        self.draft.is_some() || self.directories.draft.is_some()
    }

    pub(crate) fn start_edit(&mut self) {
        if !self.editor_is_clear() {
            return;
        }
        if let Some(entry) = self.selected_entry().cloned() {
            self.start_draft(entry, false);
        }
    }

    pub(crate) fn start_draft(&mut self, entry: Entry, is_new: bool) {
        self.page = Page::Vault;
        self.draft = Some(Draft::new(entry, is_new));
        self.confirm_delete = false;
        self.error = None;
    }

    pub(crate) fn open_records_page(&mut self, page: Page) {
        if !self.editor_is_clear() {
            return;
        }
        self.page = page;
        self.show_password = false;
        self.confirm_delete = false;
        self.error = None;
    }

    pub(crate) fn restore_record(&mut self, id: &str) {
        match self.vault.restore_record(id) {
            Ok(()) => {
                self.error = None;
                self.sync.saved();
                self.select(id.to_owned());
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(crate) fn open_settings(&mut self) {
        if !self.editor_is_clear() {
            return;
        }
        self.page = Page::Settings;
        self.show_password = false;
        self.confirm_delete = false;
        self.directories.confirm_delete = false;
    }

    /// Commands that require a clear editor run after request_navigation.
    pub(crate) fn editor_is_clear(&self) -> bool {
        !self.is_editing() && !self.saving_draft()
    }

    pub(super) fn record_draft_saved(&mut self) {
        if let Some(draft) = &mut self.draft
            && draft.is_new
        {
            draft.is_new = false;
            let id = draft.entry.id.clone();
            self.record_created = self.record_created.wrapping_add(1);
            self.search.clear();
            self.select_entry(id);
        }
        self.sync.saved();
    }

    pub(super) fn finish_saved_draft(&mut self, id: String) {
        self.draft = None;
        self.search.clear();
        self.select(id);
        self.sync.saved();
    }

    pub(crate) fn delete_selected(&mut self) -> Result<(), String> {
        let Some(id) = self.selected.as_deref() else {
            return Ok(());
        };
        self.vault
            .delete_entry(id)
            .map_err(|error| format!("Could not save: {error}"))?;

        self.selected = None;
        self.confirm_delete = false;
        self.sync.saved();
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/workspace.rs"]
mod tests;
