use super::folder;
use crate::records::{
    folder::{self as folder_model, Folder},
    import_plan::{Targets, map_folders},
};
use std::collections::BTreeMap;

fn normalize_created(created: &mut [Folder], mapping: &mut BTreeMap<&str, Option<String>>) {
    let aliases: BTreeMap<_, _> = created
        .iter()
        .enumerate()
        .map(|(i, folder)| (folder.id.clone(), format!("created-{i}")))
        .collect();
    for folder in created {
        folder.id = aliases[&folder.id].clone();
        if let Some(parent) = &mut folder.parent_id
            && let Some(alias) = aliases.get(parent.as_str())
        {
            *parent = alias.clone();
        }
    }
    for target in mapping.values_mut().flatten() {
        if let Some(alias) = aliases.get(target.as_str()) {
            *target = alias.clone();
        }
    }
}

#[test]
fn folder_mapping_preserves_source_priority_cycles_aliases_and_duplicate_ids() {
    let mut cases = vec![
        vec![],
        vec![
            folder("child", "Child", Some("parent")),
            folder("parent", "Work", None),
            folder("alias", " work ", None),
            folder("drop", " ", Some("parent")),
            folder("orphan", "Orphan", Some("absent")),
        ],
        vec![
            folder("a", "A", Some("b")),
            folder("b", "B", Some("c")),
            folder("c", "C", Some("a")),
            folder("self", "Self", Some("self")),
        ],
        vec![
            folder("kid", "Kid", Some("dup")),
            folder("dup", "First", Some("missing")),
            folder("dup", "Second", None),
            folder("kid2", "Child", Some("dup")),
            folder("dup", " third ", Some("kid")),
        ],
        vec![
            folder("", "Empty ID", Some("")),
            folder("child", "Child", Some("")),
            folder("", "Other", None),
            folder("space", "\u{2001}", Some("child")),
            folder("long", &"雪".repeat(129), Some("space")),
            folder("limit", &"雪".repeat(128), Some("long")),
        ],
    ];
    cases.push(
        (0..folder_model::MAX_DEPTH + 5)
            .rev()
            .map(|i| {
                let parent = (i > 0).then(|| format!("level-{}", i - 1));
                folder(
                    &format!("level-{i}"),
                    &format!("Level {i}"),
                    parent.as_deref(),
                )
            })
            .collect(),
    );
    for seed in 0..32 {
        cases.push(
            (0..12)
                .map(|i| {
                    let id = format!("id-{}", (i + seed) % 6);
                    let parent = match (seed + i * 3) % 9 {
                        0 => None,
                        1 => Some("absent".into()),
                        _ => Some(format!("id-{}", (seed * 3 + i * 5) % 6)),
                    };
                    let name =
                        ["Work", " work ", "Personal", " ", "Child", "雪", "X"][(seed + i) % 7];
                    folder(&id, name, parent.as_deref())
                })
                .collect(),
        );
    }
    let layouts = [
        vec![],
        vec![
            folder("work", "Work", None),
            folder("home", "Personal", None),
            folder("work-alias", " WORK ", None),
            folder("sub", "Child", Some("work")),
        ],
    ];
    for existing in &layouts {
        let existing_before = existing.clone();
        for case in &cases {
            for reversed in [false, true] {
                for rotation in [0, 1, 3] {
                    let mut source = case.clone();
                    if reversed {
                        source.reverse();
                    }
                    if !source.is_empty() {
                        let amount = rotation % source.len();
                        source.rotate_left(amount);
                    }
                    let before = source.clone();
                    let mut expected = Vec::new();
                    let mut actual = Vec::new();
                    let mut expected_mapping = legacy_map_folders(existing, &source, &mut expected);
                    let mut actual_mapping = map_folders(existing, &source, &mut actual);
                    normalize_created(&mut expected, &mut expected_mapping);
                    normalize_created(&mut actual, &mut actual_mapping);
                    assert_eq!(actual, expected);
                    assert_eq!(actual_mapping, expected_mapping);
                    assert_eq!(source, before);
                }
            }
        }
        assert_eq!(*existing, existing_before);
    }
}

pub(super) fn legacy_map_folders<'a>(
    folders: &[Folder],
    source: &'a [Folder],
    created: &mut Vec<Folder>,
) -> BTreeMap<&'a str, Option<String>> {
    if source.is_empty() {
        return BTreeMap::new();
    }

    let known: BTreeMap<&str, &Folder> = source.iter().map(|f| (f.id.as_str(), f)).collect();
    let mut mapping: BTreeMap<&str, Option<String>> = BTreeMap::new();
    let mut targets = Targets::new(folders, created);
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
        let target = targets.place(&folder.name, parent);
        mapping.insert(folder.id.as_str(), target);
    }

    mapping
}
