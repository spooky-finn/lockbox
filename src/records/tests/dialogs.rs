use crate::desktop::{
    context::edit_unlocked,
    runtime,
    session::Screen,
    view_test_support::{mounted, update},
};
use dioxus::prelude::*;

#[test]
fn attachment_dialog_results_recheck_selection_and_vault_before_io() {
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.bin");
    let destination = dir.path().join("output.bin");
    std::fs::write(&source, [0, 255, 42]).unwrap();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_edit());
        let (vault_path, entry_id) = {
            let screen = state.peek();
            let Screen::Unlocked(u) = &*screen else {
                panic!("locked")
            };
            (
                u.vault.path().to_path_buf(),
                u.draft.as_ref().unwrap().entry.id.clone(),
            )
        };
        // A different profile or draft must not receive a delayed picker result.
        super::finish_add_attachment(state, &source, &entry_id, &source);
        super::finish_add_attachment(state, &vault_path, "stale draft", &source);
        edit_unlocked(state, |u| {
            assert!(u.draft.as_ref().unwrap().entry.attachments.is_empty())
        });
        super::finish_add_attachment(state, &vault_path, &entry_id, &source);
        let file_id = {
            let screen = state.peek();
            let Screen::Unlocked(u) = &*screen else {
                panic!("locked")
            };
            let files = &u.draft.as_ref().unwrap().entry.attachments;
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].data.as_slice(), &[0, 255, 42]);
            files[0].id.clone()
        };
        edit_unlocked(state, |u| u.save_draft().unwrap());
        super::finish_export_attachment(state, &source, &entry_id, &file_id, &destination);
        super::finish_export_attachment(state, &vault_path, "stale entry", &file_id, &destination);
        assert!(!destination.exists());
        super::finish_export_attachment(state, &vault_path, &entry_id, &file_id, &destination);
        assert_eq!(std::fs::read(&destination).unwrap(), [0, 255, 42]);
        runtime::lock(None);
        let locked_destination = dir.path().join("locked.bin");
        super::finish_add_attachment(state, &vault_path, &entry_id, &source);
        super::finish_export_attachment(
            state,
            &vault_path,
            &entry_id,
            &file_id,
            &locked_destination,
        );
        assert!(!locked_destination.exists());
        assert!(matches!(&*state.peek(), Screen::Locked(_)));
    });
}
