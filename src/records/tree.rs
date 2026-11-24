//! A filtered hierarchy for presentation. Nodes borrow titles, never copy secrets.

use crate::{
    records::{
        entry::Entry,
        folder::{Folder, MAX_DEPTH},
    },
    vault::Vault,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) enum Node<'a> {
    Folder(&'a Folder, Vec<Node<'a>>),
    Record(&'a Entry),
}

/// Cached hierarchy contains only the labels and IDs needed by sidebar rows.
#[derive(PartialEq)]
pub(crate) enum DisplayNode {
    Folder {
        id: String,
        name: String,
        children: Vec<DisplayNode>,
    },
    Record {
        id: String,
        title: String,
        /// Icon choice and address, so the row shows the same picture as the record.
        icon: Option<String>,
        url: String,
    },
}

impl From<Node<'_>> for DisplayNode {
    fn from(node: Node<'_>) -> Self {
        match node {
            Node::Folder(folder, children) => Self::Folder {
                id: folder.id.clone(),
                name: folder.name.clone(),
                children: children.into_iter().map(Self::from).collect(),
            },
            Node::Record(entry) => Self::Record {
                id: entry.id.clone(),
                title: entry.title.clone(),
                icon: entry.icon.clone(),
                url: entry.url.clone(),
            },
        }
    }
}

#[cfg(test)]
pub(crate) fn project<'a>(vault: &'a Vault, query: &str) -> Vec<Node<'a>> {
    project_indexed(vault, query, &SortedTopology::new(vault))
}

pub(crate) fn project_indexed<'a>(
    vault: &'a Vault,
    query: &str,
    topology: &SortedTopology,
) -> Vec<Node<'a>> {
    #[cfg(test)]
    PROJECTIONS.set(PROJECTIONS.get() + 1);
    let query = query.trim().to_lowercase();
    let mut placed = Placed::default();
    let mut nodes =
        Hierarchy::from_index(vault, topology).branch(None, &query, false, 0, &mut placed);
    // Concurrent valid folder moves can create a cycle or dangling parent.
    // Keep every unreachable record and folder available for search and repair.
    for folder in vault.folders() {
        if !placed.folders.contains(folder.id.as_str())
            && (query.is_empty() || folder.name.to_lowercase().contains(&query))
        {
            nodes.push(Node::Folder(folder, Vec::new()));
        }
    }

    for entry in vault.entries() {
        if !placed.entries.contains(entry.id.as_str()) && entry_matches(entry, &query) {
            nodes.push(Node::Record(entry));
        }
    }
    nodes
}

#[cfg(test)]
thread_local! {
    pub(crate) static PROJECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static METADATA_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn entry_matches(entry: &Entry, query: &str) -> bool {
    query.is_empty()
        || [&entry.title, &entry.username, &entry.url]
            .iter()
            .any(|field| field.to_lowercase().contains(query))
}

#[derive(Default)]
struct Placed<'a> {
    folders: BTreeSet<&'a str>,
    entries: BTreeSet<&'a str>,
}

struct Hierarchy<'a> {
    folders: BTreeMap<Option<&'a str>, Vec<&'a Folder>>,
    entries: BTreeMap<Option<&'a str>, Vec<&'a Entry>>,
}

/// Sorted positions only. No secret fields or normalized usernames/URLs retained.
#[derive(PartialEq)]
pub(crate) struct SortedTopology {
    folders: BTreeMap<Option<String>, Vec<usize>>,
    entries: BTreeMap<Option<String>, Vec<usize>>,
}

impl SortedTopology {
    pub(crate) fn new(vault: &Vault) -> Self {
        #[cfg(test)]
        METADATA_SCANS.set(METADATA_SCANS.get() + 1);
        let mut folders: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (index, folder) in vault.folders().iter().enumerate() {
            folders
                .entry(folder.parent_id.clone())
                .or_default()
                .push(index);
        }
        for siblings in folders.values_mut() {
            siblings.sort_by_cached_key(|&index| vault.folders()[index].name.to_lowercase());
        }
        let mut entries: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (index, entry) in vault.entries().iter().enumerate() {
            entries
                .entry(entry.folder_id.clone())
                .or_default()
                .push(index);
        }
        for siblings in entries.values_mut() {
            siblings
                .sort_by_cached_key(|&index| vault.entries()[index].title.trim().to_lowercase());
        }
        Self { folders, entries }
    }
}

impl<'a> Hierarchy<'a> {
    fn from_index(vault: &'a Vault, topology: &SortedTopology) -> Self {
        let folders = topology
            .folders
            .values()
            .map(|indices| {
                let parent = vault.folders()[indices[0]].parent_id.as_deref();
                (
                    parent,
                    indices
                        .iter()
                        .map(|&index| &vault.folders()[index])
                        .collect(),
                )
            })
            .collect();
        let entries = topology
            .entries
            .values()
            .map(|indices| {
                let parent = vault.entries()[indices[0]].folder_id.as_deref();
                (
                    parent,
                    indices
                        .iter()
                        .map(|&index| &vault.entries()[index])
                        .collect(),
                )
            })
            .collect();
        Self { folders, entries }
    }

    fn branch(
        &self,
        parent: Option<&str>,
        query: &str,
        inherited: bool,
        depth: usize,
        placed: &mut Placed<'a>,
    ) -> Vec<Node<'a>> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }

        let mut nodes = Vec::new();
        for folder in self.folders.get(&parent).into_iter().flatten().copied() {
            let matching = inherited || folder.name.to_lowercase().contains(query);
            let children = self.branch(Some(&folder.id), query, matching, depth + 1, placed);

            if query.is_empty() || matching || !children.is_empty() {
                placed.folders.insert(folder.id.as_str());
                nodes.push(Node::Folder(folder, children));
            }
        }

        nodes.extend(
            self.entries
                .get(&parent)
                .into_iter()
                .flatten()
                .copied()
                .filter(|entry| inherited || entry_matches(entry, query))
                .map(|entry| {
                    placed.entries.insert(entry.id.as_str());
                    Node::Record(entry)
                }),
        );
        nodes
    }
}

#[cfg(test)]
#[path = "tests/tree.rs"]
mod tests;
