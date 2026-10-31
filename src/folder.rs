//! Encrypted folder metadata and hierarchy invariants.
use crate::entry::Entry;
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
            id: crate::id::new(),
            name,
            parent_id,
        }
    }
}

/// Reject dangling references, cycles, excessive nesting and ambiguous siblings.
pub(crate) fn validate(folders: &[Folder], entries: &[Entry]) -> Result<(), &'static str> {
    let by_id: BTreeMap<_, _> = folders
        .iter()
        .map(|folder| (folder.id.as_str(), folder))
        .collect();
    if by_id.len() != folders.len() || folders.iter().any(|folder| folder.id.is_empty()) {
        return Err("Повторяющийся или пустой идентификатор папки");
    }
    let mut names = BTreeSet::new();
    for folder in folders {
        if folder.name.trim().is_empty() || folder.name.chars().count() > MAX_NAME_CHARS {
            return Err("Название папки должно содержать от 1 до 128 символов");
        }
        if !names.insert((
            folder.parent_id.as_deref(),
            folder.name.trim().to_lowercase(),
        )) {
            return Err("Папка с таким названием уже есть в этом расположении");
        }
        let mut current = Some(folder.id.as_str());
        let mut visited = BTreeSet::new();
        while let Some(id) = current {
            if !visited.insert(id) || visited.len() > MAX_DEPTH {
                return Err("Нельзя вложить папку в себя или создать больше 64 уровней");
            }
            current = by_id
                .get(id)
                .ok_or("Родительская папка не найдена")?
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
        return Err("Папка записи не найдена");
    }
    Ok(())
}

/// Root-to-leaf breadcrumbs, bounded even for an invalid in-memory hierarchy.
pub(crate) fn ancestors<'a>(folders: &'a [Folder], id: &str) -> Vec<&'a Folder> {
    let mut path = Vec::new();
    let mut current = Some(id);
    while let Some(id) = current {
        let Some(folder) = folders.iter().find(|folder| folder.id == id) else {
            break;
        };
        if path.iter().any(|item: &&Folder| item.id == id) || path.len() == MAX_DEPTH {
            break;
        }
        path.push(folder);
        current = folder.parent_id.as_deref();
    }
    path.reverse();
    path
}

pub(crate) fn path(folders: &[Folder], id: &str) -> String {
    ancestors(folders, id)
        .iter()
        .map(|folder| folder.name.as_str())
        .collect::<Vec<_>>()
        .join(" / ")
}

pub(crate) fn options(folders: &[Folder]) -> Vec<(&Folder, String)> {
    let mut options: Vec<_> = folders
        .iter()
        .map(|folder| (folder, path(folders, &folder.id)))
        .collect();
    options.sort_by_key(|(_, path)| path.to_lowercase());
    options
}

pub(crate) fn contains(folders: &[Folder], ancestor: &str, id: &str) -> bool {
    ancestors(folders, id)
        .iter()
        .any(|folder| folder.id == ancestor)
}
