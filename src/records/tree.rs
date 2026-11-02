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

pub(crate) fn project<'a>(vault: &'a Vault, query: &str) -> Vec<Node<'a>> {
    let query = query.trim().to_lowercase();
    let mut placed = Placed::default();
    let mut nodes = Hierarchy::new(vault).branch(None, &query, false, 0, &mut placed);
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

impl<'a> Hierarchy<'a> {
    fn new(vault: &'a Vault) -> Self {
        let mut folders: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for folder in vault.folders() {
            folders
                .entry(folder.parent_id.as_deref())
                .or_default()
                .push(folder);
        }
        for siblings in folders.values_mut() {
            siblings.sort_by_cached_key(|folder| folder.name.to_lowercase());
        }

        let mut entries: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for entry in vault.entries() {
            entries
                .entry(entry.folder_id.as_deref())
                .or_default()
                .push(entry);
        }
        for siblings in entries.values_mut() {
            siblings.sort_by_cached_key(|entry| entry.title.to_lowercase());
        }

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
