//! Move live records between independently encrypted vaults.

use crate::{
    records::{
        batches::{BATCH_MUTATIONS, entry_batches},
        folder::{self, Folder},
    },
    sync::protocol::{EntityKind, Hash, Replay, Value},
    vault::{Records, Vault, VaultError},
};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroize;

#[derive(Clone, Default)]
pub(crate) struct Selection {
    pub entries: BTreeSet<String>,
    pub folders: BTreeSet<String>,
    pub entire_vault: bool,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MoveError {
    #[error("Choose a different destination space")]
    SameSpace,
    #[error("Select at least one record or folder")]
    EmptySelection,
    #[error("The source selection changed. Review it before trying again.")]
    SourceChanged,
    #[error("A selected record or folder has conflicting versions. Resolve them before moving.")]
    SourceConflict,
    #[error("The destination copies changed. Review both spaces before trying again.")]
    DestinationChanged,
    #[error(transparent)]
    Vault(#[from] VaultError),
}

pub(crate) fn same_space(source: &Vault, destination: &Vault) -> bool {
    source.salt == destination.salt
        || source
            .space_descriptor()
            .zip(destination.space_descriptor())
            .is_some_and(|(a, b)| a.space == b.space)
}

pub(crate) struct Move {
    source_id: (String, Option<Hash>),
    destination_id: (String, Option<Hash>),
    entire_vault: bool,
    original: Records,
    copies: Records,
    saved: BTreeSet<String>,
    copying_started: bool,
    copied: bool,
}

impl Move {
    pub(crate) fn new(
        source: &Vault,
        destination: &Vault,
        selection: &Selection,
    ) -> Result<Self, MoveError> {
        if same_space(source, destination) {
            return Err(MoveError::SameSpace);
        }
        if selection.entries.is_empty() && selection.folders.is_empty() {
            return Err(MoveError::EmptySelection);
        }

        let hierarchy = folder::Hierarchy::new(source.folders());
        let mut folders: Vec<_> = source
            .folders()
            .iter()
            .filter_map(|folder| {
                let lineage: Vec<_> = hierarchy.lineage(&folder.id).collect();
                lineage
                    .iter()
                    .any(|ancestor| selection.folders.contains(&ancestor.id))
                    .then(|| (lineage.len(), folder.clone()))
            })
            .collect();
        folders.sort_by_key(|(depth, _)| *depth);
        let folders: Vec<_> = folders.into_iter().map(|(_, folder)| folder).collect();
        let folder_ids: BTreeSet<_> = folders.iter().map(|folder| folder.id.as_str()).collect();
        let entries: Vec<_> = source
            .entries()
            .iter()
            .filter(|entry| {
                selection.entries.contains(&entry.id)
                    || entry
                        .folder_id
                        .as_deref()
                        .is_some_and(|id| folder_ids.contains(id))
            })
            .cloned()
            .collect();
        let entry_ids: BTreeSet<_> = entries.iter().map(|entry| entry.id.as_str()).collect();
        if !selection
            .folders
            .iter()
            .all(|id| folder_ids.contains(id.as_str()))
            || !selection
                .entries
                .iter()
                .all(|id| entry_ids.contains(id.as_str()))
        {
            return Err(MoveError::SourceChanged);
        }

        let original = Records { entries, folders };
        let copies = destination_copies(&original, destination);
        Ok(Self {
            source_id: identity(source),
            destination_id: identity(destination),
            entire_vault: selection.entire_vault,
            original,
            copies,
            saved: BTreeSet::new(),
            copying_started: false,
            copied: false,
        })
    }

    #[cfg(test)]
    fn counts(&self) -> (usize, usize) {
        (self.original.entries.len(), self.original.folders.len())
    }

    pub(crate) fn copying_started(&self) -> bool {
        self.copying_started
    }

    /// Reuse assigned destination IDs when retrying the same move.
    pub(crate) fn execute(
        &mut self,
        source: &mut Vault,
        destination: &mut Vault,
    ) -> Result<(), MoveError> {
        if same_space(source, destination) {
            return Err(MoveError::SameSpace);
        }
        if identity(source) != self.source_id {
            return Err(MoveError::SourceChanged);
        }
        if identity(destination) != self.destination_id {
            return Err(MoveError::DestinationChanged);
        }

        ready_to_move(destination)?;

        let folders = remaining(
            &self.copies.folders,
            destination.folders(),
            &mut self.saved,
            |folder| &folder.id,
        )?;
        let entries = remaining(
            &self.copies.entries,
            destination.entries(),
            &mut self.saved,
            |entry| &entry.id,
        )?;

        let replay = ready_to_move(source)?;
        self.check_source(source, replay.as_ref())?;
        self.copying_started = true;

        // Parent-first folders, then entries. A source deletion can start only
        // after every destination batch was committed.
        if folders.is_empty() && entries.is_empty() {
            if !self.copied {
                // Recovery can expose a completed intent before its final save.
                destination.save()?;
            }
        } else if destination.sync_state().is_none() {
            destination.append_records(&folders, &entries)?;
        } else {
            for batch in folders.chunks(BATCH_MUTATIONS) {
                destination.append_records(batch, &[])?;
                self.saved
                    .extend(batch.iter().map(|folder| folder.id.clone()));
            }
            for batch in entry_batches(&entries) {
                destination.append_records(&[], batch)?;
                self.saved
                    .extend(batch.iter().map(|entry| entry.id.clone()));
            }
        }
        self.saved
            .extend(folders.iter().map(|folder| folder.id.clone()));
        self.saved
            .extend(entries.iter().map(|entry| entry.id.clone()));
        let retrying_cleanup = self.copied;
        self.copied = true;

        if source.sync_state().is_none() {
            source.remove_moved_records(
                self.original
                    .entries
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect(),
                self.original
                    .folders
                    .iter()
                    .map(|folder| folder.id.as_str())
                    .collect(),
            )?;
        } else {
            for batch in self.original.entries.chunks(BATCH_MUTATIONS) {
                source.remove_moved_records(
                    batch.iter().map(|entry| entry.id.as_str()).collect(),
                    BTreeSet::new(),
                )?;
            }
            for batch in self.original.folders.rchunks(BATCH_MUTATIONS) {
                source.remove_moved_records(
                    BTreeSet::new(),
                    batch.iter().map(|folder| folder.id.as_str()).collect(),
                )?;
            }
        }
        if retrying_cleanup {
            source.save()?;
        }
        Ok(())
    }

    fn check_source(&self, source: &Vault, replay: Option<&Replay>) -> Result<(), MoveError> {
        let folders: BTreeMap<_, _> = self
            .original
            .folders
            .iter()
            .map(|f| (f.id.as_str(), f))
            .collect();
        let entries: BTreeMap<_, _> = self
            .original
            .entries
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect();
        let moving_parent =
            |parent: Option<&str>| parent.is_some_and(|id| folders.contains_key(id));
        if replay.is_some_and(|replay| {
            replay.projection.conflicts().any(|record| {
                let id = record.entity.id.as_str();
                self.entire_vault
                    || (match record.entity.kind {
                        EntityKind::Entry => entries.contains_key(id),
                        EntityKind::Folder => folders.contains_key(id),
                    })
                    || record.versions.iter().any(|version| match &version.value {
                        Some(Value::Entry(entry)) => moving_parent(entry.folder_id.as_deref()),
                        Some(Value::Folder(folder)) => moving_parent(folder.parent_id.as_deref()),
                        None => false,
                    })
            })
        }) {
            return Err(MoveError::SourceConflict);
        }

        let mut found = 0;
        for item in source.entries() {
            if let Some(expected) = entries.get(item.id.as_str()) {
                if *expected != item {
                    return Err(MoveError::SourceChanged);
                }
                found += 1;
            } else if self.entire_vault || moving_parent(item.folder_id.as_deref()) {
                return Err(MoveError::SourceChanged);
            }
        }
        for item in source.folders() {
            if let Some(expected) = folders.get(item.id.as_str()) {
                if *expected != item {
                    return Err(MoveError::SourceChanged);
                }
                found += 1;
            } else if self.entire_vault || moving_parent(item.parent_id.as_deref()) {
                return Err(MoveError::SourceChanged);
            }
        }
        if !self.copied && found != entries.len() + folders.len() {
            return Err(MoveError::SourceChanged);
        }
        Ok(())
    }
}

fn identity(vault: &Vault) -> (String, Option<Hash>) {
    (
        vault.id(),
        vault.space_descriptor().map(|descriptor| descriptor.space),
    )
}

/// Recover any interrupted signing intent before inspecting the live records.
fn ready_to_move(vault: &mut Vault) -> Result<Option<Replay>, VaultError> {
    vault.ensure_exchange_committed()?;
    vault.ensure_profile_writable()?;
    let Some(state) = vault.sync_state() else {
        return Ok(None);
    };
    if let Some(error) = state.fault {
        return Err(error.into());
    }
    vault.recover_sync()?;
    if vault
        .sync_state()
        .is_some_and(|state| state.intent.is_some())
    {
        return Err(crate::sync::protocol::Error::PendingIntent.into());
    }
    let replay = vault.sync_replay_state()?;
    if let Some(replay) = &replay {
        replay.require_writer()?;
    }
    Ok(replay)
}

fn remaining<T: Clone + PartialEq>(
    planned: &[T],
    live: &[T],
    saved: &mut BTreeSet<String>,
    id: impl Fn(&T) -> &str,
) -> Result<Vec<T>, MoveError> {
    let live: BTreeMap<_, _> = live.iter().map(|item| (id(item), item)).collect();
    let mut missing = Vec::new();
    for item in planned {
        let id = id(item);
        match live.get(id) {
            Some(actual) if *actual == item => {
                saved.insert(id.to_owned());
            }
            None if !saved.contains(id) => missing.push(item.clone()),
            _ => return Err(MoveError::DestinationChanged),
        }
    }
    Ok(missing)
}

fn destination_copies(source: &Records, destination: &Vault) -> Records {
    let mut root_names: BTreeSet<_> = destination
        .folders()
        .iter()
        .filter(|folder| folder.parent_id.is_none())
        .map(|folder| folder.name.trim().to_lowercase())
        .collect();
    let mut ids = BTreeMap::new();
    let folders = source
        .folders
        .iter()
        .map(|source| {
            let parent = source
                .parent_id
                .as_ref()
                .and_then(|id| ids.get(id).cloned());
            let name = if parent.is_none() {
                available_name(&source.name, &mut root_names)
            } else {
                source.name.clone()
            };
            let folder = Folder::new(name, parent);
            ids.insert(source.id.clone(), folder.id.clone());
            folder
        })
        .collect();
    let entries = source
        .entries
        .iter()
        .cloned()
        .map(|mut entry| {
            entry.id = crate::records::id::new();
            entry.folder_id = entry.folder_id.as_ref().and_then(|id| ids.get(id).cloned());
            if destination.sync_state().is_some()
                && let Some(totp) = &mut entry.totp
            {
                let secret = crate::records::totp::normalize_secret(&totp.secret);
                totp.secret.zeroize();
                totp.secret = secret;
            }
            entry
        })
        .collect();
    Records { folders, entries }
}

fn available_name(name: &str, root_names: &mut BTreeSet<String>) -> String {
    if root_names.insert(name.trim().to_lowercase()) {
        return name.to_owned();
    }
    for index in 2.. {
        let suffix = format!(" ({index})");
        let prefix: String = name
            .trim()
            .chars()
            .take(folder::MAX_NAME_CHARS - suffix.len())
            .collect();
        let candidate = format!("{prefix}{suffix}");
        if root_names.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!()
}

impl Vault {
    fn remove_moved_records(
        &mut self,
        entries: BTreeSet<&str>,
        folders: BTreeSet<&str>,
    ) -> crate::vault::Result<()> {
        if !self
            .entries
            .iter()
            .any(|entry| entries.contains(entry.id.as_str()))
            && !self
                .folders
                .iter()
                .any(|folder| folders.contains(folder.id.as_str()))
        {
            return Ok(());
        }
        let previous = Records {
            entries: std::mem::take(&mut self.entries),
            folders: std::mem::take(&mut self.folders),
        };
        self.entries = previous
            .entries
            .iter()
            .filter(|entry| !entries.contains(entry.id.as_str()))
            .cloned()
            .collect();
        self.folders = previous
            .folders
            .iter()
            .filter(|folder| !folders.contains(folder.id.as_str()))
            .cloned()
            .collect();
        self.commit_edit(Some(previous), |_| {})
    }
}

#[cfg(test)]
#[path = "tests/move_records.rs"]
mod tests;
