use crate::desktop::{
    context::edit_unlocked,
    runtime,
    view_test_support::{mounted, update},
};

#[test]
fn selected_entry_hides_secrets_and_escapes_user_text() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.test_entries_mut()[0].title = "<script>bad()</script>".into();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("alice"));
    assert!(html.contains("bad()"));
    assert!(!html.contains("<script>bad()"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("JBSWY3DPEHPK3PXP"));
    assert!(html.contains("Код TOTP"));
}

#[test]
fn showing_then_locking_removes_the_entry_and_password() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.show_password = true)
    });
    assert!(dioxus_ssr::render(&dom).contains("private-test-password"));
    update(&mut dom, |_| runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Мастер-пароль"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("alice"));
    assert!(!html.contains("entry-detail"));
}

#[test]
fn history_hides_secrets_and_trash_restores_record() {
    use crate::desktop::workspace::Page;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.save_changes().unwrap();
            u.vault.test_entries_mut()[0].password = "new-history-secret".into();
            u.vault.save_changes().unwrap();
            u.open_records_page(Page::History);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("История записи"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("new-history-secret"));
    assert!(!html.contains("JBSWY3DPEHPK3PXP"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.delete_selected().unwrap();
            u.open_records_page(Page::Trash);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("GitLab"));
    assert!(html.contains("Восстановить"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let id = u.vault.trash().unwrap()[0].id.clone();
            u.restore_record(&id);
            assert_eq!(u.vault.entries().len(), 1);
        })
    });
    assert!(!dioxus_ssr::render(&dom).contains("new-history-secret"));
}

#[test]
fn history_follows_selection_and_keeps_other_record_contents_hidden() {
    use crate::{desktop::workspace::Page, records::entry::Entry};
    let mut dom = mounted();
    let mut other_id = String::new();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.save_changes().unwrap();
            let mut other = Entry::new();
            other.title = "Other history record".into();
            other.username = "other-history-login".into();
            other.password = "other-history-secret".into();
            other_id = other.id.clone();
            u.vault.upsert_entry(other).unwrap();
            u.open_records_page(Page::History);
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("alice"));
    assert!(!html.contains("other-history-login"));
    assert!(!html.contains("private-test-password"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.selected = Some(other_id))
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("other-history-login"));
    assert!(!html.contains("other-history-secret"));
    for selected in [Some("missing".to_owned()), None] {
        update(&mut dom, |state| {
            edit_unlocked(state, |u| u.selected = selected)
        });
        let html = dioxus_ssr::render(&dom);
        assert!(!html.contains("class=\"history-change\""));
        assert!(!html.contains("other-history-login"));
        assert!(
            html.contains(
                crate::settings::language::Language::Russian
                    .text(crate::desktop::i18n::Text::NoRecordHistory)
            )
        );
    }
}
