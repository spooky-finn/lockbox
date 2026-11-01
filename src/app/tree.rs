//! A filtered hierarchy for presentation. Nodes borrow titles, never copy secrets.

use super::workspace::matches;
use crate::{
    core::entry::Entry,
    core::folder::{Folder, MAX_DEPTH},
    infra::vault::Vault,
};

pub(crate) enum Node<'a> {
    Folder(&'a Folder, Vec<Node<'a>>),
    Record(&'a Entry),
}

pub(crate) fn project<'a>(vault: &'a Vault, query: &str) -> Vec<Node<'a>> {
    use std::collections::BTreeSet;
    let query = query.trim().to_lowercase();
    let mut nodes = branch(vault, None, &query, false, 0);
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

    for folder in &vault.folders {
        if !folders.contains(folder.id.as_str())
            && (query.is_empty() || folder.name.to_lowercase().contains(&query))
        {
            nodes.push(Node::Folder(folder, Vec::new()));
        }
    }

    for entry in &vault.entries {
        if !entries.contains(entry.id.as_str()) && matches(entry, &query) {
            nodes.push(Node::Record(entry));
        }
    }
    nodes
}

fn branch<'a>(
    vault: &'a Vault,
    parent: Option<&str>,
    query: &str,
    inherited: bool,
    depth: usize,
) -> Vec<Node<'a>> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }

    let mut folders: Vec<_> = vault
        .folders
        .iter()
        .filter(|folder| folder.parent_id.as_deref() == parent)
        .collect();
    folders.sort_by_key(|folder| folder.name.to_lowercase());
    let mut nodes = Vec::new();

    for folder in folders {
        let matching = inherited || folder.name.to_lowercase().contains(query);
        let children = branch(vault, Some(&folder.id), query, matching, depth + 1);

        if query.is_empty() || matching || !children.is_empty() {
            nodes.push(Node::Folder(folder, children));
        }
    }

    let mut entries: Vec<_> = vault
        .entries
        .iter()
        .filter(|entry| {
            entry.folder_id.as_deref() == parent && (inherited || matches(entry, query))
        })
        .collect();
    entries.sort_by_key(|entry| entry.title.to_lowercase());
    nodes.extend(entries.into_iter().map(Node::Record));
    nodes
}

#[cfg(test)]
#[path = "tests/tree.rs"]
mod tests;
