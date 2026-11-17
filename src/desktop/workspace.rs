//! Entry collection, selection and transactional save/delete.

use crate::{
    access::biometrics::TouchId,
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
    NewDirectory,
    Entry(String),
    Directory(Option<String>),
    ToggleDirectory(String),
    Page(Page),
}

pub(crate) struct Unlocked {
    pub(crate) pending_navigation: Option<Destination>,
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
    pub(crate) directories: Directories,
    pub(crate) show_password: bool,
    pub(crate) confirm_delete: bool,
    pub(crate) error: Option<String>,
    /// UI-only event counter, incremented after the first durable record save.
    pub(crate) record_created: u64,
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
            pending_navigation: None,
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
            directories,
            show_password: false,
            confirm_delete: false,
            error: None,
            record_created: 0,
            generator: generator::Options::default(),
        }
    }

    pub(crate) fn draft_changed(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| !draft.is_empty() && !self.vault.entries().contains(&draft.entry))
    }

    pub(crate) fn navigate(&mut self, destination: Destination) {
        if self.directories.draft.is_some() {
            return;
        }
        if self.draft_changed() {
            self.pending_navigation = Some(destination);
        } else {
            self.finish_navigation(destination);
        }
    }

    fn finish_navigation(&mut self, destination: Destination) {
        self.draft = None;
        self.pending_navigation = None;
        self.show_password = false;
        self.error = None;
        match destination {
            Destination::Close => {
                self.selected = None;
                self.confirm_delete = false;
            }
            Destination::NewEntry => self.start_new(),
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

    pub(crate) fn resolve_navigation(&mut self, save: bool) {
        let Some(destination) = self.pending_navigation.clone() else {
            return;
        };
        if save && let Err(error) = self.close_draft() {
            self.error = Some(error);
            return;
        }
        self.finish_navigation(destination);
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
        self.draft = Some(Draft::new(entry, is_new));
        self.confirm_delete = false;
        self.error = None;
    }

    pub(crate) fn open_records_page(&mut self, page: Page) {
        if self.is_editing() {
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
        self.page = Page::Settings;
        self.show_password = false;
        self.confirm_delete = false;
        self.directories.confirm_delete = false;
    }

    /// Persist a validated snapshot without closing the editor or resetting its controls.
    pub(crate) fn autosave_draft(&mut self) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };

        if draft.is_empty() {
            return Ok(());
        }

        let entry = draft.prepare_save()?;
        if self.vault.entries().contains(&entry) {
            if let Some(draft) = &mut self.draft {
                draft.is_new = false;
            }
            return Ok(());
        }

        let created = draft.is_new;
        self.vault
            .upsert_entry(entry)
            .map_err(|error| format!("Could not save: {error}"))?;
        if let Some(draft) = &mut self.draft {
            draft.is_new = false;
        }
        self.sync.saved();
        if created {
            self.record_created = self.record_created.wrapping_add(1);
        }
        Ok(())
    }

    /// Flush pending edits before closing. A failed save keeps the editor open.
    pub(crate) fn save_draft(&mut self) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        // Explicit saves still validate new entries; closing an untouched empty
        // editor is handled separately.
        draft.validate()?;
        let id = draft.entry.id.clone();
        self.autosave_draft()?;

        self.draft = None;
        self.search.clear();
        self.select(id);
        self.sync.saved();
        Ok(())
    }

    pub(crate) fn close_draft(&mut self) -> Result<(), String> {
        if self.draft.as_ref().is_some_and(Draft::is_empty) {
            self.draft = None;
            self.error = None;
            return Ok(());
        }
        self.save_draft()
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
