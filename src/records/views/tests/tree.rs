use crate::desktop::{
    context::edit_unlocked,
    view_test_support::{mounted, update},
};

#[test]
fn draft_typing_does_not_reproject_the_tree() {
    let mut dom = mounted();
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_edit()));
    let before = crate::records::tree::PROJECTIONS.get();
    update(&mut dom, |state| {
        crate::desktop::context::set_field(
            state,
            crate::records::editor::Field::Password,
            "fictional-update".into(),
        )
    });
    assert_eq!(crate::records::tree::PROJECTIONS.get(), before);
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.search = "missing".into())
    });
    assert!(crate::records::tree::PROJECTIONS.get() > before);
}

#[test]
fn tree_records_show_only_titles_and_search_reveals_collapsed_ancestors() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let parent = crate::records::folder::Folder::new("Работа".into(), None);
            let child =
                crate::records::folder::Folder::new("Сервисы".into(), Some(parent.id.clone()));
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

#[test]
fn large_vault_typing_and_navigation_reuse_metadata_and_hierarchy() {
    use crate::desktop::context::set_field;
    use crate::records::views::folder_select::OPTION_BUILDS;
    use crate::records::{editor::Field, entry::Entry, folder::Folder, tree};
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.test_entries_mut().extend((0..1_000).map(|i| {
                let mut entry = Entry::new();
                entry.title = format!("Fictional entry {i}");
                entry
            }));
            u.vault
                .test_folders_mut()
                .extend((0..100).map(|i| Folder::new(format!("Folder {i}"), None)));
            u.start_edit();
        })
    });
    let before = (
        tree::METADATA_SCANS.get(),
        tree::PROJECTIONS.get(),
        OPTION_BUILDS.get(),
    );
    for index in 0..20 {
        update(&mut dom, |state| {
            set_field(state, Field::Password, format!("fictional-{index}"))
        });
    }
    assert_eq!(
        (
            tree::METADATA_SCANS.get(),
            tree::PROJECTIONS.get(),
            OPTION_BUILDS.get()
        ),
        before,
        "typing must not scan records, project the tree or rebuild folder options"
    );
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.draft = None;
            u.select_directory(None);
        })
    });
    assert_eq!(tree::METADATA_SCANS.get(), before.0);
    assert_eq!(
        tree::PROJECTIONS.get(),
        before.1,
        "selection must reuse sorted hierarchy"
    );
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let id = u.vault.folders()[0].id.clone();
            u.directories.toggle(id);
        })
    });
    assert_eq!(
        tree::PROJECTIONS.get(),
        before.1,
        "expansion must reuse sorted hierarchy"
    );
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let mut entry = u.vault.entries()[0].clone();
            entry.title = "Renamed saved entry".into();
            u.vault.upsert_entry(entry).unwrap();
        })
    });
    assert!(tree::METADATA_SCANS.get() > before.0);
    assert!(tree::PROJECTIONS.get() > before.1);
    assert!(dioxus_ssr::render(&dom).contains("Renamed saved entry"));
}
