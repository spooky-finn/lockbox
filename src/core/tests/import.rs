use super::*;
use crate::core::entry::{Totp, TotpAlgorithm};

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
