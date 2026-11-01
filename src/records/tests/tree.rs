use crate::{
    desktop::{
        test_support::{add, unlocked},
        workspace::Unlocked,
    },
    records::tree::{Node, project},
};

fn outline(nodes: &[Node<'_>]) -> Vec<String> {
    fn visit(nodes: &[Node<'_>], depth: usize, lines: &mut Vec<String>) {
        for node in nodes {
            match node {
                Node::Folder(folder, children) => {
                    lines.push(format!("{depth}:folder:{}", folder.name));
                    visit(children, depth + 1, lines);
                }
                Node::Record(entry) => lines.push(format!("{depth}:entry:{}", entry.title)),
            }
        }
    }

    let mut lines = Vec::new();
    visit(nodes, 0, &mut lines);
    lines
}

#[test]
fn search_matches_title_username_and_url_ignoring_case() {
    let dir = tempfile::tempdir().unwrap();
    let mut unlocked = unlocked(&dir);
    add(&mut unlocked, "GitLab", "a8ee", "https://gitlab.example");
    add(&mut unlocked, "Почта", "inbox", "https://mail.example");
    add(&mut unlocked, "банк", "client", "https://bank.example");
    let titles = |unlocked: &Unlocked| {
        project(&unlocked.vault, &unlocked.search)
            .into_iter()
            .filter_map(|node| match node {
                Node::Record(entry) => Some(entry.title.clone()),
                Node::Folder(_, _) => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(&unlocked), ["GitLab", "банк", "Почта"]);
    unlocked.search = "GITLAB".into();
    assert_eq!(titles(&unlocked), ["GitLab"]);
    unlocked.search = "INBOX".into();
    assert_eq!(titles(&unlocked), ["Почта"]);
    unlocked.search = "bank.example".into();
    assert_eq!(titles(&unlocked), ["банк"]);
    unlocked.search = "ПОЧ".into();
    assert_eq!(titles(&unlocked), ["Почта"]);
}

#[test]
fn concurrent_folder_cycles_and_dangling_parents_do_not_hide_records() {
    use crate::records::folder::Folder;
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    add(&mut workspace, "Find me", "username", "https://example.com");
    *workspace.vault.test_folders_mut() = vec![
        Folder {
            id: "f".into(),
            name: "F".into(),
            parent_id: Some("g".into()),
        },
        Folder {
            id: "g".into(),
            name: "G".into(),
            parent_id: Some("f".into()),
        },
    ];
    workspace.vault.test_entries_mut()[0].folder_id = Some("f".into());
    assert_eq!(project(&workspace.vault, "").len(), 3);
    assert!(matches!(
        project(&workspace.vault, "find").as_slice(),
        [Node::Record(_)]
    ));
    workspace.vault.test_entries_mut()[0].folder_id = Some("missing".into());
    assert!(matches!(
        project(&workspace.vault, "find").as_slice(),
        [Node::Record(_)]
    ));
}

#[test]
fn branches_keep_sorted_siblings_and_inherit_matching_folder_names() {
    use crate::records::{entry::Entry, folder::Folder};
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    *workspace.vault.test_folders_mut() = [
        ("beta", "beta", None),
        ("nested", "Nested", Some("alpha")),
        ("alpha", "ALPHA", None),
    ]
    .into_iter()
    .map(|(id, name, parent)| Folder {
        id: id.into(),
        name: name.into(),
        parent_id: parent.map(str::to_owned),
    })
    .collect();
    *workspace.vault.test_entries_mut() = [
        ("z root", None),
        ("z child", Some("alpha")),
        ("Hidden Match", Some("nested")),
        ("Alpha root", None),
        ("alpha child", Some("alpha")),
    ]
    .into_iter()
    .map(|(title, parent)| {
        let mut entry = Entry::new();
        entry.title = title.into();
        entry.folder_id = parent.map(str::to_owned);
        entry
    })
    .collect();

    assert_eq!(
        outline(&project(&workspace.vault, "")),
        [
            "0:folder:ALPHA",
            "1:folder:Nested",
            "2:entry:Hidden Match",
            "1:entry:alpha child",
            "1:entry:z child",
            "0:folder:beta",
            "0:entry:Alpha root",
            "0:entry:z root",
        ]
    );
    assert_eq!(
        outline(&project(&workspace.vault, "alpha")),
        [
            "0:folder:ALPHA",
            "1:folder:Nested",
            "2:entry:Hidden Match",
            "1:entry:alpha child",
            "1:entry:z child",
            "0:entry:Alpha root",
        ]
    );
    for query in [" HIDDEN ", " nested "] {
        assert_eq!(
            outline(&project(&workspace.vault, query)),
            ["0:folder:ALPHA", "1:folder:Nested", "2:entry:Hidden Match",]
        );
    }
}

#[test]
fn excessive_depth_keeps_unreachable_items_at_root_and_searchable() {
    use crate::records::{
        entry::Entry,
        folder::{Folder, MAX_DEPTH},
    };
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    *workspace.vault.test_folders_mut() = (0..MAX_DEPTH + 3)
        .map(|index| Folder {
            id: format!("f{index}"),
            name: format!("Level {index}"),
            parent_id: index.checked_sub(1).map(|parent| format!("f{parent}")),
        })
        .collect();
    *workspace.vault.test_entries_mut() = [
        ("Deep target", format!("f{MAX_DEPTH}")),
        ("Orphan target", "missing".into()),
    ]
    .into_iter()
    .map(|(title, parent)| {
        let mut entry = Entry::new();
        entry.title = title.into();
        entry.folder_id = Some(parent);
        entry
    })
    .collect();

    let nodes = project(&workspace.vault, "");
    assert_eq!(nodes.len(), 5);
    let lines = outline(&nodes);
    assert_eq!(lines.len(), MAX_DEPTH + 5);
    assert_eq!(
        lines[MAX_DEPTH],
        format!("{MAX_DEPTH}:folder:Level {MAX_DEPTH}")
    );
    assert_eq!(
        &lines[MAX_DEPTH + 1..],
        [
            format!("0:folder:Level {}", MAX_DEPTH + 1),
            format!("0:folder:Level {}", MAX_DEPTH + 2),
            "0:entry:Deep target".into(),
            "0:entry:Orphan target".into(),
        ]
    );
    assert_eq!(
        outline(&project(&workspace.vault, "target")),
        ["0:entry:Deep target", "0:entry:Orphan target"]
    );
}

#[test]
fn normalized_sort_ties_keep_input_order_for_siblings_and_folder_options() {
    use crate::records::{
        entry::Entry,
        folder::{Folder, Hierarchy},
    };
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let names = ["Банк", "ALPHA", "банк", "alpha", "İ", "i\u{307}"];
    *workspace.vault.test_folders_mut() = names
        .iter()
        .map(|name| Folder::new((*name).into(), None))
        .collect();
    *workspace.vault.test_entries_mut() = names
        .iter()
        .map(|name| {
            let mut entry = Entry::new();
            entry.title = (*name).into();
            entry
        })
        .collect();
    let expected = ["ALPHA", "alpha", "İ", "i\u{307}", "Банк", "банк"];
    let expected_outline = ["folder", "entry"]
        .into_iter()
        .flat_map(|kind| expected.iter().map(move |name| format!("0:{kind}:{name}")))
        .collect::<Vec<_>>();
    assert_eq!(outline(&project(&workspace.vault, "")), expected_outline);
    assert_eq!(
        Hierarchy::new(workspace.vault.folders())
            .options()
            .into_iter()
            .map(|(folder, _)| folder.name.as_str())
            .collect::<Vec<_>>(),
        expected
    );
}
