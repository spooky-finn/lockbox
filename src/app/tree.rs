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
    branch(vault, None, &query.trim().to_lowercase(), false, 0)
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
