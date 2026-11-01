//! Folder navigation and transactional hierarchy changes; no UI dependencies.

use crate::{
    desktop::workspace::Unlocked,
    records::folder::{self, Folder},
    vault::Vault,
};
use std::collections::BTreeSet;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Default)]
pub(crate) struct Directories {
    pub selected: Option<String>,
    pub collapsed: BTreeSet<String>,
    pub draft: Option<DirectoryDraft>,
    pub confirm_delete: bool,
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub(crate) struct DirectoryDraft {
    pub id: Option<String>,
    pub name: String,
    pub parent_id: Option<String>,
}

impl Directories {
    pub fn reveal(&mut self, vault: &Vault) {
        if let Some(id) = &self.selected {
            for folder in folder::Hierarchy::new(vault.folders()).lineage(id) {
                self.collapsed.remove(&folder.id);
            }
        }
    }

    pub fn toggle(&mut self, id: String) {
        if !self.collapsed.remove(&id) {
            self.collapsed.insert(id);
        }
    }

    pub fn save(&mut self, vault: &mut Vault) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };

        let id = draft.id.clone().unwrap_or_else(crate::records::id::new);
        let folder = Folder {
            id: id.clone(),
            name: draft.name.trim().to_owned(),
            parent_id: draft.parent_id.clone(),
        };

        vault
            .save_folder(folder)
            .map_err(|error| format!("Could not save folder: {error}"))?;

        self.draft = None;
        self.selected = Some(id);
        self.confirm_delete = false;
        self.reveal(vault);
        Ok(())
    }

    /// Removing a subtree preserves every record, moving them into its parent.
    pub fn delete(&mut self, vault: &mut Vault) -> Result<(), String> {
        let Some(selected) = self
            .selected
            .as_deref()
            .and_then(|id| vault.folders().iter().find(|folder| folder.id == id))
        else {
            return Ok(());
        };

        let id = selected.id.clone();
        let parent = selected.parent_id.clone();
        vault
            .delete_folder(&id)
            .map_err(|error| format!("Could not delete folder: {error}"))?;

        self.selected = parent;
        self.confirm_delete = false;
        self.collapsed
            .retain(|id| vault.folders().iter().any(|folder| &folder.id == id));
        self.reveal(vault);
        Ok(())
    }
}

impl Unlocked {
    pub(crate) fn save_directory(&mut self) -> Result<(), String> {
        self.directories.save(&mut self.vault)?;
        self.sync.saved();
        self.search.clear();
        Ok(())
    }

    pub(crate) fn select_directory(&mut self, id: Option<String>) {
        if self.is_editing() {
            return;
        }

        self.page = crate::desktop::workspace::Page::Vault;
        self.selected = None;
        self.show_password = false;
        self.confirm_delete = false;
        self.directories.selected = id;
        self.directories.confirm_delete = false;
        self.directories.reveal(&self.vault);
        self.error = None;
    }

    pub(crate) fn start_directory(&mut self) {
        if self.is_editing() {
            return;
        }

        self.page = crate::desktop::workspace::Page::Vault;
        self.selected = None;
        self.show_password = false;
        self.directories.draft = Some(DirectoryDraft {
            id: None,
            name: String::new(),
            parent_id: self.directories.selected.clone(),
        });
        self.directories.confirm_delete = false;
        self.error = None;
    }

    pub(crate) fn edit_directory(&mut self) {
        if self.is_editing() {
            return;
        }

        if let Some(folder) = self
            .directories
            .selected
            .as_deref()
            .and_then(|id| self.vault.folders().iter().find(|folder| folder.id == id))
        {
            self.directories.draft = Some(DirectoryDraft {
                id: Some(folder.id.clone()),
                name: folder.name.clone(),
                parent_id: folder.parent_id.clone(),
            });
            self.directories.confirm_delete = false;
            self.error = None;
        }
    }
}

#[cfg(test)]
#[path = "tests/directories.rs"]
mod tests;
