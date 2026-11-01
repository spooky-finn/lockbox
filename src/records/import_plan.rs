//! Copying records from another vault into the current one.
//!
//! Imported records always receive new identifiers: in a shared space an
//! identifier is the synchronized entity, so reusing one could overwrite an
//! unrelated record. Re-importing the same file skips identical entries, which
//! makes an interrupted import safe to repeat.

use crate::records::{
    entry::{Attachment, Entry},
    folder::{self, Folder},
    totp,
};
use std::collections::BTreeMap;
use zeroize::Zeroize;

/// Records to append to the current vault. Folders are ordered parents first.
#[derive(Debug, Default)]
pub struct Plan {
    pub folders: Vec<Folder>,
    pub entries: Vec<Entry>,
    /// Source entries identical to an existing entry in the same folder.
    pub skipped: usize,
}

/// Plans the import without changing either vault.
///
/// A source folder is merged into an existing sibling with the same name
/// (ignoring case and surrounding spaces). Source folders with an invalid name
/// are dropped and their records move to the parent. Cycles, missing parents
/// and excessive nesting are attached to the root, so every record stays reachable.
pub fn plan(
    folders: &[Folder],
    entries: &[Entry],
    source_folders: &[Folder],
    source_entries: &[Entry],
) -> Plan {
    let mut plan = Plan::default();
    let mapping = map_folders(folders, source_folders, &mut plan.folders);

    for source in source_entries {
        let mut entry = source.clone();
        entry.id = crate::records::id::new();
        entry.folder_id = source
            .folder_id
            .as_ref()
            .and_then(|id| mapping.get(id.as_str()).cloned().flatten());

        if let Some(totp) = &mut entry.totp {
            // Shared spaces accept only canonical Base32 secrets.
            let normalized = totp::normalize_secret(&totp.secret);
            totp.secret.zeroize();
            totp.secret = normalized;
        }

        if entries
            .iter()
            .chain(&plan.entries)
            .any(|known| same_entry(known, &entry))
        {
            plan.skipped += 1;
        } else {
            plan.entries.push(entry);
        }
    }

    plan
}

/// Maps every source folder ID to a target folder (`None` is the root).
fn map_folders<'a>(
    folders: &[Folder],
    source: &'a [Folder],
    created: &mut Vec<Folder>,
) -> BTreeMap<&'a str, Option<String>> {
    let known: BTreeMap<&str, &Folder> = source.iter().map(|f| (f.id.as_str(), f)).collect();
    let mut mapping: BTreeMap<&str, Option<String>> = BTreeMap::new();
    let mut depth: BTreeMap<Option<String>, usize> = BTreeMap::from([(None, 0)]);

    for folder in folders {
        let path = folder::ancestors(folders, &folder.id);
        depth.insert(Some(folder.id.clone()), path.len());
    }

    let mut pending: Vec<&Folder> = source.iter().collect();

    while !pending.is_empty() {
        let ready = pending.iter().position(|folder| {
            folder
                .parent_id
                .as_deref()
                .is_none_or(|parent| mapping.contains_key(parent) || !known.contains_key(parent))
        });
        // Only a cycle leaves no folder ready; break it at the first member.
        let folder = pending.remove(ready.unwrap_or(0));
        let parent = match ready {
            Some(_) => folder
                .parent_id
                .as_deref()
                .and_then(|parent| mapping.get(parent).cloned().flatten()),
            None => None,
        };
        let target = place(folders, created, &mut depth, &folder.name, parent);
        mapping.insert(folder.id.as_str(), target);
    }

    mapping
}

fn place(
    folders: &[Folder],
    created: &mut Vec<Folder>,
    depth: &mut BTreeMap<Option<String>, usize>,
    name: &str,
    parent: Option<String>,
) -> Option<String> {
    let name = name.trim();

    if name.is_empty() || name.chars().count() > folder::MAX_NAME_CHARS {
        return parent;
    }

    let parent_depth = depth.get(&parent).copied().unwrap_or(0);
    let parent = if parent_depth >= folder::MAX_DEPTH {
        None
    } else {
        parent
    };
    let key = name.to_lowercase();

    if let Some(existing) = folders
        .iter()
        .chain(created.iter())
        .find(|f| f.parent_id == parent && f.name.trim().to_lowercase() == key)
    {
        return Some(existing.id.clone());
    }

    let folder = Folder::new(name.to_owned(), parent.clone());
    let id = folder.id.clone();
    let level = depth.get(&parent).copied().unwrap_or(0) + 1;
    depth.insert(Some(id.clone()), level);
    created.push(folder);
    Some(id)
}

/// Equal content in the same folder; identifiers are ignored.
fn same_entry(a: &Entry, b: &Entry) -> bool {
    a.title == b.title
        && a.username == b.username
        && a.password == b.password
        && a.url == b.url
        && a.notes == b.notes
        && a.totp == b.totp
        && a.icon == b.icon
        && a.folder_id == b.folder_id
        && same_files(&a.attachments, &b.attachments)
}

fn same_files(a: &[Attachment], b: &[Attachment]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| a.name == b.name && a.data == b.data)
}

#[cfg(test)]
#[path = "tests/import_plan.rs"]
mod tests;
