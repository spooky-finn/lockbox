use super::*;

fn folder(id: &str, name: &str, parent: Option<&str>) -> Folder {
    Folder {
        id: id.into(),
        name: name.into(),
        parent_id: parent.map(str::to_owned),
    }
}

fn ids(folders: Vec<&Folder>) -> Vec<&str> {
    folders
        .into_iter()
        .map(|folder| folder.id.as_str())
        .collect()
}

#[test]
fn breadcrumbs_membership_and_options_share_the_same_hierarchy() {
    let folders = [
        folder("child", "Dev", Some("root")),
        folder("archive", "archive", None),
        folder("leaf", "SSH", Some("child")),
        folder("root", "Work", None),
    ];
    let hierarchy = Hierarchy::new(&folders);

    assert_eq!(ids(hierarchy.ancestors("leaf")), ["root", "child", "leaf"]);
    assert_eq!(hierarchy.path("leaf"), "Work / Dev / SSH");
    assert!(hierarchy.contains("root", "leaf"));
    assert!(hierarchy.contains("leaf", "leaf"));
    assert!(!hierarchy.contains("leaf", "root"));
    assert!(!hierarchy.contains("missing", "leaf"));
    assert!(!hierarchy.contains("root", "missing"));
    assert!(hierarchy.ancestors("missing").is_empty());
    assert_eq!(hierarchy.path("missing"), "");
    assert_eq!(
        hierarchy
            .options()
            .iter()
            .map(|(folder, path)| (folder.id.as_str(), path.as_str()))
            .collect::<Vec<_>>(),
        [
            ("archive", "archive"),
            ("root", "Work"),
            ("child", "Work / Dev"),
            ("leaf", "Work / Dev / SSH"),
        ]
    );
}

#[test]
fn cycles_and_missing_parents_leave_bounded_breadcrumbs() {
    let folders = [
        folder("self", "Self", Some("self")),
        folder("a", "A", Some("b")),
        folder("b", "B", Some("a")),
        folder("orphan", "Orphan", Some("missing")),
    ];
    let hierarchy = Hierarchy::new(&folders);

    assert_eq!(ids(hierarchy.ancestors("self")), ["self"]);
    assert_eq!(ids(hierarchy.ancestors("a")), ["b", "a"]);
    assert_eq!(ids(hierarchy.ancestors("b")), ["a", "b"]);
    assert_eq!(ids(hierarchy.ancestors("orphan")), ["orphan"]);
    assert!(hierarchy.contains("self", "self"));
    assert!(hierarchy.contains("b", "a"));
    assert!(hierarchy.contains("a", "b"));
    assert!(!hierarchy.contains("missing", "orphan"));
    assert_eq!(hierarchy.path("orphan"), "Orphan");
}

#[test]
fn duplicate_ids_use_first_occurrence_and_keep_stable_options() {
    let folders = [
        folder("duplicate", "First", Some("root")),
        folder("duplicate", "Second", None),
        folder("root", "Root", None),
        folder("leaf", "Leaf", Some("duplicate")),
    ];
    let hierarchy = Hierarchy::new(&folders);

    assert_eq!(
        ids(hierarchy.ancestors("leaf")),
        ["root", "duplicate", "leaf"]
    );
    assert_eq!(hierarchy.path("duplicate"), "Root / First");
    assert!(hierarchy.contains("root", "duplicate"));
    assert_eq!(
        hierarchy
            .options()
            .iter()
            .map(|(folder, path)| (folder.name.as_str(), path.as_str()))
            .collect::<Vec<_>>(),
        [
            ("Root", "Root"),
            ("First", "Root / First"),
            ("Second", "Root / First"),
            ("Leaf", "Root / First / Leaf"),
        ]
    );
}

#[test]
fn ancestry_and_membership_stop_at_the_same_depth_limit() {
    let folders: Vec<_> = (0..MAX_DEPTH + 2)
        .map(|index| Folder {
            id: format!("f{index}"),
            name: format!("Folder {index}"),
            parent_id: index.checked_sub(1).map(|parent| format!("f{parent}")),
        })
        .collect();
    let leaf = format!("f{}", MAX_DEPTH + 1);
    let hierarchy = Hierarchy::new(&folders);
    let ancestors = hierarchy.ancestors(&leaf);
    assert_eq!(ancestors.len(), MAX_DEPTH);
    assert_eq!(ancestors[0].id, "f2");
    assert_eq!(ancestors.last().unwrap().id, leaf);
    assert!(hierarchy.contains("f2", &leaf));
    assert!(!hierarchy.contains("f1", &leaf));
    assert!(!hierarchy.contains("f0", &leaf));
}
