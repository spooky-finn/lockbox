//! Encrypted folder metadata and hierarchy invariants.

use crate::records::entry::Entry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const MAX_DEPTH: usize = 64;
pub const MAX_NAME_CHARS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

impl Folder {
    pub fn new(name: String, parent_id: Option<String>) -> Self {
        Self {
            id: crate::records::id::new(),
            name,
            parent_id,
        }
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= MAX_NAME_CHARS
}

/// Reject dangling references, cycles, excessive nesting and ambiguous siblings.
pub(crate) fn validate(folders: &[Folder], entries: &[Entry]) -> Result<(), &'static str> {
    let by_id: BTreeMap<_, _> = folders
        .iter()
        .map(|folder| (folder.id.as_str(), folder))
        .collect();

    if by_id.len() != folders.len() || folders.iter().any(|folder| folder.id.is_empty()) {
        return Err("Duplicate or empty folder ID");
    }

    let mut names = BTreeSet::new();

    for folder in folders {
        if !valid_name(&folder.name) {
            return Err("Folder name must contain 1–128 characters");
        }

        if !names.insert((
            folder.parent_id.as_deref(),
            folder.name.trim().to_lowercase(),
        )) {
            return Err("A folder with this name already exists here");
        }

        let mut current = Some(folder.id.as_str());
        let mut visited = BTreeSet::new();

        while let Some(id) = current {
            if !visited.insert(id) || visited.len() > MAX_DEPTH {
                return Err("A folder cannot contain itself or exceed 64 levels");
            }
            current = by_id
                .get(id)
                .ok_or("Parent folder not found")?
                .parent_id
                .as_deref();
        }
    }

    if entries.iter().any(|entry| {
        entry
            .folder_id
            .as_deref()
            .is_some_and(|id| !by_id.contains_key(id))
    }) {
        return Err("Entry folder not found");
    }

    Ok(())
}

/// Borrowed lookups reused by breadcrumbs, selectors and subtree checks.
pub(crate) struct Hierarchy<'a> {
    folders: &'a [Folder],
    by_id: BTreeMap<&'a str, &'a Folder>,
}

impl<'a> Hierarchy<'a> {
    pub(crate) fn new(folders: &'a [Folder]) -> Self {
        let mut by_id = BTreeMap::new();
        // Preserve the first match even in malformed input with duplicate IDs.
        for folder in folders {
            by_id.entry(folder.id.as_str()).or_insert(folder);
        }
        Self { folders, by_id }
    }

    /// Leaf-to-root walk, bounded for cycles and excessive depth.
    pub(crate) fn lineage(&self, id: &str) -> impl Iterator<Item = &'a Folder> {
        let mut current = self.by_id.get(id).copied();
        let mut visited = BTreeSet::new();
        std::iter::from_fn(move || {
            let folder = current.take()?;
            if !visited.insert(folder.id.as_str()) {
                return None;
            }
            current = folder
                .parent_id
                .as_deref()
                .and_then(|id| self.by_id.get(id))
                .copied();
            Some(folder)
        })
        .take(MAX_DEPTH)
    }

    /// Root-to-leaf breadcrumbs, bounded even for an invalid in-memory hierarchy.
    pub(crate) fn ancestors(&self, id: &str) -> Vec<&'a Folder> {
        let mut path: Vec<_> = self.lineage(id).collect();
        path.reverse();
        path
    }

    fn path(&self, id: &str) -> String {
        self.ancestors(id)
            .iter()
            .map(|folder| folder.name.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub(crate) fn options(&self) -> Vec<(&'a Folder, String)> {
        let mut options: Vec<_> = self
            .folders
            .iter()
            .map(|folder| (folder, self.path(&folder.id)))
            .collect();
        options.sort_by_cached_key(|(_, path)| path.to_lowercase());
        options
    }

    pub(crate) fn contains(&self, ancestor: &str, id: &str) -> bool {
        self.lineage(id).any(|folder| folder.id == ancestor)
    }
}

#[cfg(test)]
#[path = "tests/folder.rs"]
mod tests;
