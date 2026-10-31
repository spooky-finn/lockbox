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
        assert_eq!(form.error.as_deref(), Some("Разблокировка прервалась"));
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
