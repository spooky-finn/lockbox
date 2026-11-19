use crate::{
    desktop::{
        context::{Clock, edit_unlocked},
        runtime,
        session::Screen,
        view_test_support::{RenderCounts, mounted, mounted_with_header, update},
    },
    platform::autolock::AutoLock,
};
use dioxus::prelude::*;
use std::time::{Instant, SystemTime};

#[test]
fn autosave_waits_for_idle_then_persists_without_closing_the_editor() {
    use crate::desktop::context::{Session, set_field};
    use crate::records::editor::Field;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_new());
        set_field(state, Field::Title, "Automatic save".into());
        set_field(state, Field::Password, "fictional-password".into());
        runtime::tick();
    });
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.peek();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        assert!(u.draft.is_some());
        assert!(
            !crate::desktop::test_support::reopen(u)
                .entries
                .iter()
                .any(|e| e.title == "Automatic save")
        );
    });
    assert!(dioxus_ssr::render(&dom).contains("id=\"entry-title\""));
    update(&mut dom, |state| {
        // Make the deadline due without a wall-clock sleep.
        edit_unlocked(state, |u| {
            u.draft.as_mut().unwrap().autosave_at = Some(Instant::now())
        });
        runtime::tick();
        crate::desktop::view_test_support::finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(u.draft.is_some());
        assert_eq!(
            crate::desktop::test_support::reopen(u).entries[1].password,
            "fictional-password"
        );
        assert!(u.draft.as_ref().unwrap().autosave_at.is_none());
    });
}

#[test]
fn autosave_coalesces_edits_and_ignores_transient_controls() {
    use crate::desktop::context::{draft_control, edit_draft, set_field};
    use crate::records::editor::{Control, Field};
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_edit());
        draft_control(state, Control::TogglePassword);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(u.draft.as_ref().unwrap().autosave_at.is_none());
    });
    update(&mut dom, |state| {
        set_field(state, Field::Username, "updated-user".into());
        edit_unlocked(state, |u| {
            u.draft.as_mut().unwrap().autosave_at = Some(Instant::now())
        });
        set_field(state, Field::Notes, "latest edit".into());
        runtime::tick();
        crate::desktop::view_test_support::finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(crate::desktop::test_support::reopen(u).entries.is_empty());
    });
    update(&mut dom, |state| {
        edit_draft(state, |d| d.entry.icon = Some("github".into()));
        edit_unlocked(state, |u| {
            u.draft.as_mut().unwrap().autosave_at = Some(Instant::now())
        });
        runtime::tick();
        crate::desktop::view_test_support::finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        let saved = crate::desktop::test_support::reopen(u);
        assert_eq!(saved.entries[0].username, "updated-user");
        assert_eq!(saved.entries[0].notes, "latest edit");
        assert_eq!(saved.entries[0].icon.as_deref(), Some("github"));
        assert!(u.draft.as_ref().unwrap().show_password);
        assert_eq!(u.vault.record_history().unwrap().len(), 1);
    });
}

#[test]
fn failed_timed_save_retains_draft_and_next_edit_retries() {
    use crate::desktop::context::set_field;
    use crate::records::editor::Field;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.start_edit();
            std::fs::create_dir(u.vault.path().with_extension("lbx.tmp")).unwrap();
        });
        set_field(state, Field::Notes, "pending note".into());
        edit_unlocked(state, |u| {
            u.draft.as_mut().unwrap().autosave_at = Some(Instant::now())
        });
        runtime::tick();
        crate::desktop::view_test_support::finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(u.error.is_some());
        assert_eq!(u.draft.as_ref().unwrap().entry.notes, "pending note");
        assert!(u.draft.as_ref().unwrap().autosave_at.is_none());
        assert!(crate::desktop::test_support::reopen(u).entries.is_empty());
        std::fs::remove_dir(u.vault.path().with_extension("lbx.tmp")).unwrap();
    });
    update(&mut dom, |state| {
        set_field(state, Field::Notes, "retry note".into());
        edit_unlocked(state, |u| {
            u.draft.as_mut().unwrap().autosave_at = Some(Instant::now())
        });
        runtime::tick();
        crate::desktop::view_test_support::finish_save(state);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("locked");
        };
        assert!(u.error.is_none());
        assert_eq!(
            crate::desktop::test_support::reopen(u).entries[0].notes,
            "retry note"
        );
    });
}

#[test]
fn lock_flushes_inflight_save_and_newer_input_before_releasing_the_vault() {
    use crate::desktop::context::set_field;
    use crate::records::editor::Field;
    use crate::vault::{Vault, VaultError};
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_edit());
        set_field(state, Field::Title, "Snapshot before lock".into());
        let path = {
            let mut state = state;
            let Screen::Unlocked(u) = &mut *state.write() else {
                panic!("locked");
            };
            u.request_autosave();
            assert!(u.saving_draft());
            u.vault.path().to_owned()
        };
        let password = secrecy::SecretString::from("master".to_owned());
        assert!(matches!(
            Vault::open(&path, &password),
            Err(VaultError::ProfileInUse)
        ));
        set_field(state, Field::Title, "Final edit before lock".into());
        runtime::lock(None);
        assert!(matches!(&*state.peek(), Screen::Locked(_)));
        let reopened = Vault::open(path, &password).unwrap();
        assert_eq!(reopened.entries()[0].title, "Final edit before lock");
        runtime::tick();
        assert!(matches!(&*state.peek(), Screen::Locked(_)));
    });
}

#[test]
fn worker_results_open_the_vault_and_report_task_or_worker_failure() {
    use crate::access::auth::Opened;
    use crate::platform::jobs::Worker;
    use std::sync::mpsc;
    use std::time::Duration;
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    update(&mut dom, |_| runtime::lock(None));
    update(&mut dom, |state| {
        let (sender, receiver) = mpsc::channel();
        crate::desktop::context::edit_locked(state, |form| {
            form.pending = Some(Worker::spawn(move |_| {
                receiver.recv_timeout(Duration::from_secs(5)).unwrap()
            }))
        });
        runtime::poll_workers();
        assert!(matches!(&*state.peek(), Screen::Locked(_)));
        sender
            .send(Ok(Opened {
                vault: crate::desktop::test_support::unlocked(&dir).vault,
                with_password: true,
                recovery_notice: None,
            }))
            .unwrap_or_else(|_| panic!("receiver dropped"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(&*state.peek(), Screen::Locked(_)) && Instant::now() < deadline {
            runtime::poll_workers();
            std::thread::yield_now();
        }
        assert!(matches!(&*state.peek(), Screen::Unlocked(_)));
    });
    update(&mut dom, |_| runtime::lock(None));
    update(&mut dom, |state| {
        crate::desktop::context::edit_locked(state, |form| {
            form.pending = Some(Worker::ready(Err("wrong password".into())))
        });
        runtime::poll_workers();
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("open after failure");
        };
        assert_eq!(form.error.as_deref(), Some("wrong password"));
        assert!(form.pending.is_none());
    });
    update(&mut dom, |state| {
        let mut failed = Worker::ready(Err::<Opened, String>("fixture".into()));
        let _ = failed.take();
        crate::desktop::context::edit_locked(state, |form| form.pending = Some(failed));
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
    use crate::desktop::context::SecurityHandle;
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
    use crate::desktop::context::SecurityHandle;
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
    use crate::desktop::context::SecurityHandle;
    let mut dom = mounted_with_header();
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
    let first = crate::vault::paths::new_profile_path(home.path()).unwrap();
    let second = crate::vault::paths::new_profile_path(home.path()).unwrap();
    assert_eq!(first.parent().unwrap(), home.path().join(".lockbox"));
    assert!(first.parent().unwrap().is_dir());
    assert_eq!(first.extension().unwrap(), "lbx");
    assert_ne!(first, second);
    assert!(!first.exists());
}

#[test]
fn record_fields_copy_actual_values_and_report_success() {
    use crate::desktop::context::SecurityHandle;
    use crate::desktop::security::CopyField;
    use crate::platform::clipboard::Guard;
    use crate::platform::clipboard::tests::FakeBoard;

    let mut dom = mounted();
    update(&mut dom, |state| {
        let board = FakeBoard::default();
        consume_context::<SecurityHandle>().borrow_mut().clipboard =
            Guard::new(Box::new(board.clone()));
        for (field, expected) in [
            (CopyField::Username, "alice"),
            (CopyField::Password, "private-test-password"),
            (CopyField::Url, "https://gitlab.com"),
        ] {
            assert!(runtime::copy(field));
            assert_eq!(board.0.borrow().as_deref(), Some(expected));
        }
        edit_unlocked(state, |u| u.selected = None);
        assert!(!runtime::copy(CopyField::Password));
    });
    assert!(!dioxus_ssr::render(&dom).contains("private-test-password"));
}

#[test]
fn copying_a_record_does_not_rerender_session_subscribers() {
    let mut dom = mounted_with_header();
    let counts = dom.in_scope(ScopeId::APP, consume_context::<RenderCounts>);
    let before = counts.0.get();
    update(&mut dom, |_| {
        assert!(runtime::copy(crate::desktop::security::CopyField::Password));
    });
    assert_eq!(counts.0.get(), before);
    assert!(dioxus_ssr::render(&dom).contains("буфер очистится через"));
}
