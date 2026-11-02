use crate::records::{
    entry::{Totp, TotpAlgorithm},
    import_plan::*,
};

#[path = "import_plan/folders.rs"]
mod folder_queue;

fn entry(title: &str, folder: Option<&str>) -> Entry {
    let mut entry = Entry::new();
    entry.title = title.into();
    entry.password = format!("{title}-password");
    entry.folder_id = folder.map(str::to_owned);
    entry
}

fn folder(id: &str, name: &str, parent: Option<&str>) -> Folder {
    Folder {
        id: id.into(),
        name: name.into(),
        parent_id: parent.map(str::to_owned),
    }
}

#[test]
fn imported_entries_get_new_ids_and_keep_their_contents() {
    let source = vec![entry("GitLab", None), entry("Почта", None)];
    let plan = plan(&[], &[], &[], &source);

    assert_eq!(plan.entries.len(), 2);
    assert_eq!(plan.skipped, 0);

    for (imported, original) in plan.entries.iter().zip(&source) {
        assert_ne!(imported.id, original.id);
        assert_eq!(imported.title, original.title);
        assert_eq!(imported.password, original.password);
    }
}

#[test]
fn folders_merge_by_name_and_keep_the_hierarchy() {
    let existing = vec![folder("work", "Work", None)];
    let source_folders = vec![
        folder("s-child", "Servers", Some("s-work")),
        folder("s-work", "  work ", None),
        folder("s-home", "Home", None),
    ];
    let source = vec![
        entry("ssh", Some("s-child")),
        entry("router", Some("s-home")),
        entry("jira", Some("s-work")),
    ];
    let plan = plan(&existing, &[], &source_folders, &source);

    assert_eq!(plan.folders.len(), 2);
    let servers = plan.folders.iter().find(|f| f.name == "Servers").unwrap();
    let home = plan.folders.iter().find(|f| f.name == "Home").unwrap();
    assert_eq!(servers.parent_id.as_deref(), Some("work"));
    assert_eq!(home.parent_id, None);

    let folder_of = |title: &str| {
        plan.entries
            .iter()
            .find(|e| e.title == title)
            .unwrap()
            .folder_id
            .clone()
    };
    assert_eq!(folder_of("ssh"), Some(servers.id.clone()));
    assert_eq!(folder_of("router"), Some(home.id.clone()));
    assert_eq!(folder_of("jira"), Some("work".into()));

    let mut all = existing.clone();
    all.extend(plan.folders.iter().cloned());
    folder::validate(&all, &plan.entries).unwrap();
}

#[test]
fn sibling_merging_keeps_parents_first_matches_and_new_folder_aliases() {
    let existing = [
        folder("work", "Work", None),
        folder("home", "Personal", None),
        folder("work-first", " Серверы ", Some("work")),
        folder("work-second", "СЕРВЕРЫ", Some("work")),
        folder("home-servers", "серверы", Some("home")),
    ];
    let source_folders = [
        folder("s-work-servers", "СЕРВЕРЫ", Some("s-work")),
        folder("s-work", "work", None),
        folder("s-home-servers", " Серверы ", Some("s-home")),
        folder("s-home", "PERSONAL", None),
        folder("s-db", "Databases", Some("s-work")),
        folder("s-db-alias", " databases ", Some("s-work")),
    ];
    let source_entries = [
        entry("Work server", Some("s-work-servers")),
        entry("Home server", Some("s-home-servers")),
        entry("First database", Some("s-db")),
        entry("Second database", Some("s-db-alias")),
    ];
    let before = existing.clone();
    let imported = plan(&existing, &[], &source_folders, &source_entries);

    assert_eq!(imported.folders.len(), 1);
    assert_eq!(imported.folders[0].name, "Databases");
    assert_eq!(imported.folders[0].parent_id.as_deref(), Some("work"));
    let locations: Vec<_> = imported
        .entries
        .iter()
        .map(|entry| entry.folder_id.as_deref())
        .collect();
    assert_eq!(
        locations,
        [
            Some("work-first"),
            Some("home-servers"),
            Some(imported.folders[0].id.as_str()),
            Some(imported.folders[0].id.as_str()),
        ]
    );
    assert_eq!(existing, before);
}

#[test]
fn children_beyond_existing_depth_limit_merge_at_root() {
    let mut existing: Vec<_> = (0..folder::MAX_DEPTH)
        .map(|index| Folder {
            id: format!("f{index}"),
            name: format!("Level {index}"),
            parent_id: index.checked_sub(1).map(|parent| format!("f{parent}")),
        })
        .collect();
    existing.push(folder("root-extra", "Extra", None));
    let mut source_folders: Vec<_> = (0..folder::MAX_DEPTH)
        .rev()
        .map(|index| Folder {
            id: format!("s{index}"),
            name: format!(" level {index} "),
            parent_id: index.checked_sub(1).map(|parent| format!("s{parent}")),
        })
        .collect();
    source_folders.push(folder(
        "s-extra",
        " extra ",
        Some(&format!("s{}", folder::MAX_DEPTH - 1)),
    ));
    let imported = plan(
        &existing,
        &[],
        &source_folders,
        &[entry("Beyond limit", Some("s-extra"))],
    );

    assert!(imported.folders.is_empty());
    assert_eq!(imported.entries.len(), 1);
    assert_eq!(imported.entries[0].folder_id.as_deref(), Some("root-extra"));
    folder::validate(&existing, &imported.entries).unwrap();
}

#[test]
fn identical_entries_are_skipped_so_a_repeated_import_adds_nothing() {
    let source_folders = vec![folder("s-work", "Work", None)];
    let source = vec![entry("ssh", Some("s-work")), entry("ssh", Some("s-work"))];
    let first = plan(&[], &[], &source_folders, &source);
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.skipped, 1);

    let second = plan(&first.folders, &first.entries, &source_folders, &source);
    assert!(second.folders.is_empty());
    assert!(second.entries.is_empty());
    assert_eq!(second.skipped, 2);
}

#[test]
fn same_title_with_different_contents_is_imported() {
    let existing = vec![entry("ssh", None)];
    let mut changed = entry("ssh", None);
    changed.password = "rotated".into();
    let plan = plan(&[], &existing, &[], &[changed]);
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.skipped, 0);
}

#[test]
fn invalid_source_hierarchy_keeps_every_record_reachable() {
    let source_folders = vec![
        folder("a", "A", Some("b")),
        folder("b", "B", Some("a")),
        folder("blank", "   ", None),
        folder("orphan", "Orphan", Some("missing")),
    ];
    let source = vec![
        entry("in-cycle", Some("a")),
        entry("in-blank", Some("blank")),
        entry("in-orphan", Some("orphan")),
        entry("dangling", Some("nowhere")),
    ];
    let plan = plan(&[], &[], &source_folders, &source);

    assert_eq!(plan.entries.len(), 4);
    folder::validate(&plan.folders, &plan.entries).unwrap();
    let dangling = plan.entries.iter().find(|e| e.title == "dangling").unwrap();
    let blank = plan.entries.iter().find(|e| e.title == "in-blank").unwrap();
    assert_eq!(dangling.folder_id, None);
    assert_eq!(blank.folder_id, None);
}

#[test]
fn deep_source_nesting_is_capped() {
    let source_folders: Vec<_> = (0..folder::MAX_DEPTH + 5)
        .map(|i| {
            let parent = (i > 0).then(|| format!("f{}", i - 1));
            folder(&format!("f{i}"), &format!("Level {i}"), parent.as_deref())
        })
        .collect();
    let plan = plan(&[], &[], &source_folders, &[]);
    folder::validate(&plan.folders, &[]).unwrap();
}

#[test]
fn totp_secrets_are_normalized() {
    let mut source = entry("2fa", None);
    source.totp = Some(Totp {
        secret: "jbsw y3dp ehpk 3pxp".into(),
        algorithm: TotpAlgorithm::Sha1,
        digits: 6,
        period: 30,
    });
    let plan = plan(&[], &[], &[], &[source]);
    assert_eq!(
        plan.entries[0].totp.as_ref().unwrap().secret,
        "JBSWY3DPEHPK3PXP"
    );
}

#[test]
fn duplicate_checks_use_mapped_folders_and_normalized_totp_without_changing_sources() {
    let folders = [folder("work", "Work", None)];
    let source_folders = [folder("source-work", " work ", None)];
    let mut known = entry("2fa", Some("work"));
    known.totp = Some(Totp {
        secret: "JBSWY3DPEHPK3PXP".into(),
        algorithm: TotpAlgorithm::Sha256,
        digits: 8,
        period: 60,
    });
    known
        .add_attachment("key.bin".into(), vec![0, 255, 42])
        .unwrap();
    let mut source = known.clone();
    source.id = crate::records::id::new();
    source.folder_id = Some("source-work".into());
    source.totp.as_mut().unwrap().secret = "jbsw y3dp ehpk 3pxp====".into();
    source.attachments[0].id = crate::records::id::new();
    let source_entries = [source.clone(), source];
    let before = source_entries.clone();

    let repeated = plan(&folders, &[known.clone()], &source_folders, &source_entries);
    assert!(repeated.entries.is_empty());
    assert!(repeated.folders.is_empty());
    assert_eq!(repeated.skipped, 2);

    let first = plan(&folders, &[], &source_folders, &source_entries);
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.skipped, 1);
    assert_eq!(first.entries[0].totp, known.totp);
    assert_eq!(first.entries[0].folder_id.as_deref(), Some("work"));
    assert_ne!(first.entries[0].id, source_entries[0].id);
    assert_eq!(source_entries, before);
}

#[test]
fn attachment_name_bytes_and_order_distinguish_imported_records() {
    let mut known = entry("Files", None);
    known.add_attachment("one.bin".into(), vec![1]).unwrap();
    known.add_attachment("two.bin".into(), vec![2]).unwrap();
    let mut renamed = known.clone();
    renamed.attachments[0].name = "renamed.bin".into();
    let mut changed = known.clone();
    changed.attachments[0].data = vec![3];
    let mut reordered = known.clone();
    reordered.attachments.swap(0, 1);

    for source in [renamed, changed, reordered] {
        let imported = plan(&[], std::slice::from_ref(&known), &[], &[source]);
        assert_eq!(imported.entries.len(), 1);
        assert_eq!(imported.skipped, 0);
    }
}

#[test]
fn deduplication_matches_legacy_content_folder_and_source_order_rules() {
    let folders = [
        folder("work", "Work", None),
        folder("home", "Personal", None),
    ];
    let source_folders = [
        folder("s-work", " work ", None),
        folder("s-home", "PERSONAL", None),
        folder("s-invalid", " ", Some("s-work")),
    ];
    let titles = ["", "Same", "same", "雪🗝", "é", "e\u{301}"];
    let mut original = Vec::new();
    for i in 0..48 {
        let location = [None, Some("work"), Some("home")][i % 3];
        let mut value = entry(titles[i % titles.len()], location);
        value.username = format!("user-{}", i % 7);
        value.password = format!("password-{}", i % 11);
        value.url = format!("https://site{}.example", i % 5);
        value.notes = format!("note-{}", i % 3);
        value.icon = (i % 3 == 0).then(|| "github".into());
        if i % 4 == 0 {
            value.totp = Some(Totp {
                secret: if i == 0 {
                    "jbsw y3dp ehpk 3pxp===="
                } else {
                    "JBSWY3DPEHPK3PXP"
                }
                .into(),
                algorithm: TotpAlgorithm::Sha256,
                digits: 8,
                period: 60,
            });
        }
        if i % 6 == 0 {
            value
                .add_attachment("one.bin".into(), vec![0, 255, i as u8])
                .unwrap();
            value.add_attachment("two.bin".into(), vec![73]).unwrap();
        }
        original.push(value);
    }
    let mut sources = Vec::new();
    for i in 0..96 {
        let mut value = original[i % original.len()].clone();
        value.id = format!("{:032x}", i + 10000);
        value.folder_id = match value.folder_id.as_deref() {
            Some("work") => Some("s-work".into()),
            Some("home") => Some("s-home".into()),
            _ => None,
        };
        if i % 11 == 0 {
            value.folder_id = Some("missing-source".into());
        }
        if let Some(totp) = &mut value.totp {
            totp.secret = "jbsw y3dp ehpk 3pxp====".into();
        }
        if let Some(file) = value.attachments.first_mut() {
            file.id = format!("{:032x}", i + 20000);
        }
        match i % 9 {
            1 => value.username.push('!'),
            2 => value.password.push('!'),
            3 => value.url.push_str("/different"),
            4 => value.notes.push('!'),
            5 => {
                value.icon = if value.icon.is_some() {
                    None
                } else {
                    Some("other".into())
                }
            }
            6 => {
                value.folder_id = Some(
                    if value.folder_id.as_deref() == Some("s-work") {
                        "s-home"
                    } else {
                        "s-work"
                    }
                    .into(),
                )
            }
            7 => {
                value.totp = Some(Totp {
                    secret: "JBSWY3DPEHPK3PXP".into(),
                    algorithm: TotpAlgorithm::Sha1,
                    digits: 6,
                    period: 30,
                })
            }
            8 => value
                .add_attachment("added.bin".into(), vec![i as u8])
                .unwrap(),
            _ => {}
        }
        sources.extend([value.clone(), value]);
    }
    for title in ["New", "new", "New🗝"] {
        let mut value = entry("shared new content", Some("s-invalid"));
        value.title = title.into();
        sources.extend([value.clone(), value]);
    }

    for rotation in [0, 1, 7, 31] {
        let mut known = original.clone();
        known.rotate_left(rotation);
        let mut source_entries = sources.clone();
        source_entries.rotate_left(rotation);
        let known_before = known.clone();
        let sources_before = source_entries.clone();
        for use_known in [false, true] {
            let existing = if use_known { known.as_slice() } else { &[] };
            for length in [0, 1, source_entries.len()] {
                let source = &source_entries[..length];
                let mut expected = legacy_plan(&folders, existing, &source_folders, source);
                let mut actual = plan(&folders, existing, &source_folders, source);
                assert!(expected.folders.is_empty());
                assert_eq!(actual.folders, expected.folders);
                assert_eq!(actual.skipped, expected.skipped);
                if length == source_entries.len() {
                    assert!(!actual.entries.is_empty());
                    assert!(actual.skipped > 0);
                }
                for result in [&mut expected, &mut actual] {
                    for value in &mut result.entries {
                        value.id.clear();
                    }
                }
                assert_eq!(actual.entries, expected.entries);
            }
        }
        assert_eq!(known, known_before);
        assert_eq!(source_entries, sources_before);
    }
}

fn legacy_plan(
    folders: &[Folder],
    entries: &[Entry],
    source_folders: &[Folder],
    source_entries: &[Entry],
) -> Plan {
    let mut plan = Plan::default();
    let mapping = folder_queue::legacy_map_folders(folders, source_folders, &mut plan.folders);

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
