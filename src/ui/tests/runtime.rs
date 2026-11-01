use crate::app::session::Screen;
use crate::infra::autolock::AutoLock;
use crate::ui::components::test_support::{RenderCounts, mounted, update};
use crate::ui::context::{Clock, edit_unlocked};
use crate::ui::runtime;
use dioxus::prelude::*;
use std::time::{Instant, SystemTime};

#[test]
fn worker_results_open_the_vault_and_report_failure_or_disconnect() {
    use crate::app::auth::Opened;
    use std::sync::mpsc;
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    update(&mut dom, |_| runtime::lock(None));
    update(&mut dom, |state| {
        let (sender, receiver) = mpsc::channel();
        crate::ui::context::edit_locked(state, |form| form.pending = Some(receiver));
        runtime::poll_workers();
        assert!(matches!(&*state.peek(), Screen::Locked(_)));
        sender
            .send(Ok(Opened {
                vault: crate::app::test_support::unlocked(&dir).vault,
                with_password: true,
            }))
            .unwrap_or_else(|_| panic!("receiver dropped"));
        runtime::poll_workers();
        assert!(matches!(&*state.peek(), Screen::Unlocked(_)));
    });
    update(&mut dom, |_| runtime::lock(None));
    update(&mut dom, |state| {
        let (sender, receiver) = mpsc::channel();
        crate::ui::context::edit_locked(state, |form| form.pending = Some(receiver));
        sender
            .send(Err("wrong password".into()))
            .unwrap_or_else(|_| panic!("receiver dropped"));
        runtime::poll_workers();
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("open after failure");
        };
        assert_eq!(form.error.as_deref(), Some("wrong password"));
        assert!(form.pending.is_none());
    });
    update(&mut dom, |state| {
        let (sender, receiver) = mpsc::channel();
        crate::ui::context::edit_locked(state, |form| form.pending = Some(receiver));
        drop(sender);
        runtime::poll_workers();
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("open after disconnect");
        };
        assert_eq!(form.error.as_deref(), Some("Unlock was interrupted"));
        assert!(form.pending.is_none());
    });
}

#[test]
fn background_tick_clears_clipboard_and_locks_without_input() {
    use crate::ui::context::SecurityHandle;
    use std::time::Duration;
    let mut dom = mounted();
    update(&mut dom, |_| {
        let native = consume_context::<SecurityHandle>();
        native
            .borrow_mut()
            .clipboard
            .copy("secret", Instant::now() - Duration::from_secs(21))
            .unwrap();
        runtime::tick();
        assert!(
            native
                .borrow()
                .clipboard
                .remaining(Instant::now())
                .is_none()
        );
        native.borrow_mut().auto_lock = AutoLock::new(
            1,
            Instant::now() - Duration::from_secs(61),
            SystemTime::now() - Duration::from_secs(61),
        );
        runtime::tick();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Мастер-пароль"));
    assert!(html.contains("долго не было действий"));
}

#[test]
fn first_input_after_sleep_locks_before_resetting_activity() {
    use crate::ui::context::SecurityHandle;
    use std::time::Duration;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |workspace| workspace.start_new());
        let native = consume_context::<SecurityHandle>();
        native.borrow_mut().auto_lock = AutoLock::new(
            1,
            Instant::now() - Duration::from_secs(61),
            SystemTime::now() - Duration::from_secs(61),
        );
        runtime::record_activity();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Мастер-пароль"));
    assert!(!html.contains("id=\"entry-title\""));
}

#[test]
fn clock_status_and_activity_do_not_rerender_session_subscribers() {
    use crate::ui::context::SecurityHandle;
    let mut dom = mounted();
    let counts = dom.in_scope(ScopeId::APP, consume_context::<RenderCounts>);
    let before = (counts.0.get(), counts.1.get());
    dom.in_scope(ScopeId::APP, || {
        let mut clock = consume_context::<Clock>().0;
        *clock.write() += 1;
        let native = consume_context::<SecurityHandle>();
        native
            .borrow_mut()
            .clipboard
            .copy("secret", Instant::now())
            .unwrap();
        runtime::publish_status();
        runtime::record_activity();
    });
    dom.render_immediate_to_vec();
    assert_eq!(
        counts.0.get(),
        before.0,
        "session subscriber rerendered for an unrelated signal"
    );
    assert!(counts.1.get() > before.1, "clock subscriber did not update");
    assert!(dioxus_ssr::render(&dom).contains("буфер очистится через"));
}

#[test]
fn profile_paths_use_private_directory_and_unique_names() {
    let home = tempfile::tempdir().unwrap();
    let first = runtime::new_profile_path(home.path()).unwrap();
    let second = runtime::new_profile_path(home.path()).unwrap();
    assert_eq!(first.parent().unwrap(), home.path().join(".lockbox"));
    assert!(first.parent().unwrap().is_dir());
    assert_eq!(first.extension().unwrap(), "lbx");
    assert_ne!(first, second);
    assert!(!first.exists());
}

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
        runtime::finish_add_attachment(state, &source, &entry_id, &source);
        runtime::finish_add_attachment(state, &vault_path, "stale draft", &source);
        edit_unlocked(state, |u| {
            assert!(u.draft.as_ref().unwrap().entry.attachments.is_empty())
        });
        runtime::finish_add_attachment(state, &vault_path, &entry_id, &source);
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
        runtime::finish_export_attachment(state, &source, &entry_id, &file_id, &destination);
        runtime::finish_export_attachment(
            state,
            &vault_path,
            "stale entry",
            &file_id,
            &destination,
        );
        assert!(!destination.exists());
        runtime::finish_export_attachment(state, &vault_path, &entry_id, &file_id, &destination);
        assert_eq!(std::fs::read(&destination).unwrap(), [0, 255, 42]);
        runtime::lock(None);
        let locked_destination = dir.path().join("locked.bin");
        runtime::finish_add_attachment(state, &vault_path, &entry_id, &source);
        runtime::finish_export_attachment(
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
