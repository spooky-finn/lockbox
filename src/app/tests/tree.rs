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
