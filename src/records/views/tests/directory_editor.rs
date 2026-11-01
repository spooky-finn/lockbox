use crate::desktop::{
    context::{Session, edit_unlocked},
    session::Screen,
    view_test_support::{mounted, update},
};
use dioxus::prelude::*;

#[test]
fn folder_editor_saves_and_record_editor_uses_the_selected_folder() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_directory())
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("directory-name"));
    assert!(html.contains("directory-parent"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.directories.draft.as_mut().unwrap().name = "Работа".into();
            u.save_directory().unwrap();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("directory-detail"));
    assert!(html.contains("Изменить папку"));
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_new()));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("entry-folder"));
    assert!(html.contains("Работа"));
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked")
        };
        assert_eq!(
            u.draft.as_ref().unwrap().entry.folder_id,
            u.directories.selected
        );
        assert_eq!(
            crate::desktop::test_support::reopen(u).folders[0].name,
            "Работа"
        );
    });
}
