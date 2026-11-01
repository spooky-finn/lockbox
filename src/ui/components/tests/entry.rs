use crate::ui::components::test_support::{mounted, update};
use crate::ui::context::edit_unlocked;
use crate::ui::runtime;

#[test]
fn selected_entry_hides_secrets_and_escapes_user_text() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.entries[0].title = "<script>bad()</script>".into();
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
    use crate::app::workspace::Page;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.save_changes().unwrap();
            u.vault.entries[0].password = "new-history-secret".into();
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
            assert_eq!(u.vault.entries.len(), 1);
        })
    });
    assert!(!dioxus_ssr::render(&dom).contains("new-history-secret"));
}
