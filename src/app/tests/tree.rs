use super::{Node, project};
use crate::app::test_support::{add, unlocked};
use crate::app::workspace::Unlocked;

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
    use crate::core::folder::Folder;
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    add(&mut workspace, "Find me", "username", "https://example.com");
    workspace.vault.folders = vec![
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
    workspace.vault.entries[0].folder_id = Some("f".into());
    assert_eq!(project(&workspace.vault, "").len(), 3);
    assert!(matches!(
        project(&workspace.vault, "find").as_slice(),
        [Node::Record(_)]
    ));
    workspace.vault.entries[0].folder_id = Some("missing".into());
    assert!(matches!(
        project(&workspace.vault, "find").as_slice(),
        [Node::Record(_)]
    ));
}
