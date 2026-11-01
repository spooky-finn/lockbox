//! A filtered hierarchy for presentation. Nodes borrow titles, never copy secrets.

use crate::{
    desktop::workspace::matches,
    records::{
        entry::Entry,
        folder::{Folder, MAX_DEPTH},
    },
    vault::Vault,
};
use std::collections::BTreeMap;

pub(crate) enum Node<'a> {
    Folder(&'a Folder, Vec<Node<'a>>),
    Record(&'a Entry),
}

pub(crate) fn project<'a>(vault: &'a Vault, query: &str) -> Vec<Node<'a>> {
    use std::collections::BTreeSet;
    let query = query.trim().to_lowercase();
    let mut nodes = Hierarchy::new(vault).branch(None, &query, false, 0);
    // Concurrent valid folder moves can create a cycle or dangling parent.
    // Keep every unreachable record and folder available for search and repair.
    fn placed<'a>(
        nodes: &[Node<'a>],
        folders: &mut BTreeSet<&'a str>,
        entries: &mut BTreeSet<&'a str>,
    ) {
        for node in nodes {
            match node {
                Node::Folder(folder, children) => {
                    folders.insert(&folder.id);
                    placed(children, folders, entries);
                }
                Node::Record(entry) => {
                    entries.insert(&entry.id);
                }
            }
        }
    }

    let mut folders = BTreeSet::new();
    let mut entries = BTreeSet::new();
    placed(&nodes, &mut folders, &mut entries);

    for folder in vault.folders() {
        if !folders.contains(folder.id.as_str())
            && (query.is_empty() || folder.name.to_lowercase().contains(&query))
        {
            nodes.push(Node::Folder(folder, Vec::new()));
        }
    }

    for entry in vault.entries() {
        if !entries.contains(entry.id.as_str()) && matches(entry, &query) {
            nodes.push(Node::Record(entry));
        }
    }
    nodes
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
            siblings.sort_by_key(|folder| folder.name.to_lowercase());
        }

        let mut entries: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for entry in vault.entries() {
            entries
                .entry(entry.folder_id.as_deref())
                .or_default()
                .push(entry);
        }
        for siblings in entries.values_mut() {
            siblings.sort_by_key(|entry| entry.title.to_lowercase());
        }

        Self { folders, entries }
    }

    fn branch(
        &self,
        parent: Option<&str>,
        query: &str,
        inherited: bool,
        depth: usize,
    ) -> Vec<Node<'a>> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }

        let mut nodes = Vec::new();
        for folder in self.folders.get(&parent).into_iter().flatten().copied() {
            let matching = inherited || folder.name.to_lowercase().contains(query);
            let children = self.branch(Some(&folder.id), query, matching, depth + 1);

            if query.is_empty() || matching || !children.is_empty() {
                nodes.push(Node::Folder(folder, children));
            }
        }

        nodes.extend(
            self.entries
                .get(&parent)
                .into_iter()
                .flatten()
                .copied()
                .filter(|entry| inherited || matches(entry, query))
                .map(Node::Record),
        );
        nodes
    }
}

#[cfg(test)]
#[path = "tests/tree.rs"]
mod tests;
