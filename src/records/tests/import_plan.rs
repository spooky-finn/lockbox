use crate::records::{
    entry::{Totp, TotpAlgorithm},
    import_plan::*,
};

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
