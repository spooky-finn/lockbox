use crate::ui::components::test_support::{mounted, update};
use crate::ui::context::edit_unlocked;

#[test]
fn tree_records_show_only_titles_and_search_reveals_collapsed_ancestors() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let parent = crate::core::folder::Folder::new("Работа".into(), None);
            let child = crate::core::folder::Folder::new("Сервисы".into(), Some(parent.id.clone()));
            u.vault.test_entries_mut()[0].folder_id = Some(child.id.clone());
            u.directories.collapsed.insert(parent.id.clone());
            u.directories.collapsed.insert(child.id.clone());
            *u.vault.test_folders_mut() = vec![parent, child];
            u.search = "alice".into();
        })
    });
    let html = dioxus_ssr::render(&dom);
    let tree = html
        .split("aria-label=\"Записи и папки\"")
        .nth(1)
        .unwrap()
        .split("</nav>")
        .next()
        .unwrap();
    for title in ["Работа", "Сервисы", "GitLab"] {
        assert!(tree.contains(title), "missing {title}");
    }
    assert!(
        !tree.contains("alice"),
        "usernames should not appear in record rows"
    );
    assert!(
        !tree.contains("entry-icon"),
        "record rows should not have icons"
    );
    assert!(!tree.contains("<small"));
    update(&mut dom, |state| edit_unlocked(state, |u| u.search.clear()));
    let html = dioxus_ssr::render(&dom);
    let tree = html
        .split("aria-label=\"Записи и папки\"")
        .nth(1)
        .unwrap()
        .split("</nav>")
        .next()
        .unwrap();
    assert!(
        !tree.contains("GitLab"),
        "clearing search should preserve collapsed state"
    );
}
