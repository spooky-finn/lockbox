//! Copying records from another vault into the current one.
//!
//! Imported records always receive new identifiers: in a shared space an
//! identifier is the synchronized entity, so reusing one could overwrite an
//! unrelated record. Re-importing the same file skips identical entries, which
//! makes an interrupted import safe to repeat.

use crate::records::{
    entry::{Attachment, Entry, Totp},
    folder::{self, Folder},
    totp,
};
use std::collections::BTreeMap;

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
        let folder_id = source
            .folder_id
            .as_deref()
            .and_then(|id| mapping.get(id))
            .and_then(Option::as_deref);
        let totp = source.totp.as_ref().map(|source| Totp {
            // Shared spaces accept only canonical Base32 secrets.
            secret: totp::normalize_secret(&source.secret),
            algorithm: source.algorithm,
            digits: source.digits,
            period: source.period,
        });

        if entries
            .iter()
            .chain(&plan.entries)
            .any(|known| same_entry(known, source, folder_id, totp.as_ref()))
        {
            plan.skipped += 1;
            continue;
        }

        let mut entry = source.clone();
        entry.id = crate::records::id::new();
        entry.folder_id = folder_id.map(str::to_owned);
        entry.totp = totp;
        plan.entries.push(entry);
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

    let hierarchy = folder::Hierarchy::new(folders);
    for folder in folders {
        let path = hierarchy.ancestors(&folder.id);
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

/// Compare source content after folder mapping and TOTP normalization; IDs are ignored.
fn same_entry(known: &Entry, source: &Entry, folder_id: Option<&str>, totp: Option<&Totp>) -> bool {
    known.title == source.title
        && known.username == source.username
        && known.password == source.password
        && known.url == source.url
        && known.notes == source.notes
        && known.totp.as_ref() == totp
        && known.icon == source.icon
        && known.folder_id.as_deref() == folder_id
        && same_files(&known.attachments, &source.attachments)
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
