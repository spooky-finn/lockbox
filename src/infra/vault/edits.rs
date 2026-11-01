//! Record commands own staging and rollback; shared writes retain durable intents.

use super::{Records, Result, Vault, VaultError};
use crate::core::{
    entry::Entry,
    folder::{self, Folder},
};
use std::collections::BTreeSet;

impl Vault {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    // Recovery may replace either collection. Retain the complete readable view
    // for shared operations, without rolling back reserved intents or witnesses.
    fn shared_snapshot(&self) -> Option<Records> {
        self.sync_state().is_some().then(|| Records {
            entries: self.entries.clone(),
            folders: self.folders.clone(),
        })
    }

    fn commit_edit(
        &mut self,
        shared: Option<Records>,
        undo_local: impl FnOnce(&mut Self),
    ) -> Result<()> {
        let result = self.save_changes();
        if result.is_err() {
            if let Some(previous) = shared {
                self.entries = previous.entries;
                self.folders = previous.folders;
            } else {
                undo_local(self);
            }
        }
        result
    }

    /// Inserts or replaces a record, preserving its position on update.
    /// Failure restores the readable view, while retaining any signing intent.
    pub fn upsert_entry(&mut self, entry: Entry) -> Result<()> {
        let shared = self.shared_snapshot();
        let index = self
            .entries
            .iter()
            .position(|item| item.id == entry.id)
            .unwrap_or(self.entries.len());
        let previous = if index == self.entries.len() {
            self.entries.push(entry);
            None
        } else {
            Some(std::mem::replace(&mut self.entries[index], entry))
        };
        self.commit_edit(shared, move |vault| {
            if let Some(entry) = previous {
                vault.entries[index] = entry;
            } else {
                vault.entries.remove(index);
            }
        })
    }

    /// Deletes a record through the history-aware persistence boundary.
    pub fn delete_entry(&mut self, id: &str) -> Result<()> {
        let shared = self.shared_snapshot();
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.entries.len() {
            if self.entries[index].id == id {
                removed.push((index + removed.len(), self.entries.remove(index)));
            } else {
                index += 1;
            }
        }
        self.commit_edit(shared, move |vault| {
            for (index, entry) in removed {
                vault.entries.insert(index, entry);
            }
        })
    }

    pub fn save_folder(&mut self, folder: Folder) -> Result<()> {
        let shared = self.shared_snapshot();
        let previous = self.folders.clone();
        if let Some(slot) = self.folders.iter_mut().find(|item| item.id == folder.id) {
            *slot = folder;
        } else {
            self.folders.push(folder);
        }
        if let Err(error) = folder::validate(&self.folders, &self.entries) {
            self.folders = previous;
            return Err(VaultError::InvalidFolders(error));
        }
        self.commit_edit(shared, move |vault| vault.folders = previous)
    }

    /// Removes a subtree, moving its records into the removed folder's parent.
    pub fn delete_folder(&mut self, id: &str) -> Result<()> {
        let Some(selected) = self.folders.iter().find(|folder| folder.id == id) else {
            return Ok(());
        };
        let parent = selected.parent_id.clone();
        let removed: BTreeSet<_> = self
            .folders
            .iter()
            .filter(|item| folder::contains(&self.folders, id, &item.id))
            .map(|item| item.id.clone())
            .collect();
        let shared = self.shared_snapshot();
        let previous = self.folders.clone();
        let mut assignments = Vec::new();
        for (index, entry) in self.entries.iter_mut().enumerate() {
            if entry
                .folder_id
                .as_ref()
                .is_some_and(|id| removed.contains(id))
            {
                assignments.push((index, entry.folder_id.clone()));
                entry.folder_id = parent.clone();
            }
        }
        self.folders.retain(|folder| !removed.contains(&folder.id));
        self.commit_edit(shared, move |vault| {
            vault.folders = previous;
            for (index, folder) in assignments {
                vault.entries[index].folder_id = folder;
            }
        })
    }

    /// Appends one import batch atomically. Earlier batches are independent.
    pub fn append_records(&mut self, folders: &[Folder], entries: &[Entry]) -> Result<()> {
        let previous = Records {
            entries: self.entries.clone(),
            folders: self.folders.clone(),
        };
        self.folders.extend_from_slice(folders);
        self.entries.extend_from_slice(entries);
        self.commit_edit(Some(previous), |_| {})
    }
}

#[cfg(test)]
impl Vault {
    pub(crate) fn test_entries_mut(&mut self) -> &mut Vec<Entry> {
        &mut self.entries
    }

    pub(crate) fn test_folders_mut(&mut self) -> &mut Vec<Folder> {
        &mut self.folders
    }
}
