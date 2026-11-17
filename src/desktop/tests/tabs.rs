use super::*;
use crate::desktop::{context::Clock, runtime, security::Status};
use crate::platform::touchid::tests::FakeBiometrics;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Instant, SystemTime},
};

fn fixture() -> Element {
    let dir = use_hook(|| Rc::new(tempfile::tempdir().unwrap()));
    let state =
        use_signal(|| Screen::Unlocked(Box::new(crate::desktop::test_support::unlocked(&dir))));
    let tabs = use_signal(|| Tabs::new(state));
    let settings = use_signal(|| Settings {
        config: Default::default(),
        path: None,
    });
    let status = use_signal(Status::default);
    let clock = use_signal(crate::desktop::time::unix_now);
    use_context_provider(|| state);
    use_context_provider(|| tabs);
    use_context_provider(|| settings);
    use_context_provider(|| status);
    use_context_provider(|| Clock(clock));
    use_context_provider(|| BiometricProvider(Arc::new(FakeBiometrics::default())));
    use_context_provider(|| Rc::new(RefCell::new(crate::desktop::test_support::security())));
    rsx! { TabBar {} }
}

fn mounted() -> VirtualDom {
    let mut dom = VirtualDom::new(fixture);
    dom.rebuild_in_place();
    dom
}

#[test]
fn switching_preserves_each_spaces_draft_search_and_selection() {
    let mut dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        let mut state = consume_context::<Session>();
        if let Screen::Unlocked(u) = &mut *state.write() {
            u.search = "personal search".into();
            u.start_new();
            u.draft.as_mut().unwrap().entry.title = "Unsaved personal record".into();
        }
        new_tab();
        let dir = tempfile::tempdir().unwrap();
        let mut second = crate::desktop::test_support::unlocked(&dir);
        second.search = "team search".into();
        *state.write() = Screen::Unlocked(Box::new(second));
        activate(0);
        {
            let screen = state.peek();
            let Screen::Unlocked(u) = &*screen else {
                panic!("personal tab locked");
            };
            assert_eq!(u.search, "personal search");
            assert_eq!(
                u.draft.as_ref().unwrap().entry.title,
                "Unsaved personal record"
            );
        }
        activate(1);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("team tab locked");
        };
        assert_eq!(u.search, "team search");
    });
    dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert_eq!(html.matches("role=\"tab\"").count(), 2);
    assert_eq!(html.matches("aria-selected=\"true\"").count(), 1);
    assert!(html.contains("New Tab"));
}

#[test]
fn selecting_an_open_profile_focuses_its_tab_and_aliases_do_not_duplicate() {
    let dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let path = match &*state.peek() {
            Screen::Unlocked(u) => u.vault.path().to_path_buf(),
            _ => unreachable!(),
        };
        new_tab();
        runtime::select_profile(path.parent().unwrap().join(".").join("test.lbx"));
        let tabs = consume_context::<Signal<Tabs>>();
        assert_eq!(tabs.peek().active, 0);
        assert_eq!(tabs.peek().sessions.len(), 2);
        assert!(matches!(&*state.peek(), Screen::Unlocked(_)));
    });
}

#[test]
fn closing_tabs_drops_vaults_and_keeps_the_active_session_bound() {
    let dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let path = match &*state.peek() {
            Screen::Unlocked(u) => u.vault.path().to_path_buf(),
            _ => unreachable!(),
        };
        new_tab();
        close(0);
        let tabs = consume_context::<Signal<Tabs>>();
        assert_eq!(tabs.peek().sessions.len(), 1);
        assert_eq!(tabs.peek().active, 0);
        assert!(tabs.peek().sessions[0] == state);
        let reopened =
            crate::vault::Vault::open(path, &secrecy::SecretString::from("master".to_owned()));
        assert!(reopened.is_ok(), "closed tab must release vault lock");
        close(0);
        assert_eq!(tabs.peek().sessions.len(), 1);
        assert!(matches!(&*state.peek(), Screen::Locked(f) if f.path.is_none()));
    });
}

#[test]
fn closing_active_tab_selects_neighbor_and_editing_tab_stays_open() {
    let dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        let mut state = consume_context::<Session>();
        if let Screen::Unlocked(u) = &mut *state.write() {
            u.start_new();
        }
        new_tab();
        close(0);
        let tabs = consume_context::<Signal<Tabs>>();
        assert_eq!(tabs.peek().sessions.len(), 2);
        close(1);
        assert_eq!(tabs.peek().sessions.len(), 1);
        assert!(matches!(&*state.peek(), Screen::Unlocked(u) if u.draft.is_some()));
    });
}

#[test]
fn idle_lock_covers_background_spaces_even_when_active_tab_is_locked() {
    let dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        new_tab();
        let native = consume_context::<SecurityHandle>();
        native.borrow_mut().auto_lock = crate::platform::autolock::AutoLock::new(
            1,
            Instant::now() - std::time::Duration::from_secs(120),
            SystemTime::now() - std::time::Duration::from_secs(120),
        );
        runtime::tick();
        let tabs = consume_context::<Signal<Tabs>>();
        assert!(
            tabs.peek()
                .sessions
                .iter()
                .all(|s| matches!(&*s.peek(), Screen::Locked(_)))
        );
    });
}

#[test]
fn lock_cancels_an_unlock_result_and_locks_other_tabs() {
    let dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        new_tab();
        let mut state = consume_context::<Session>();
        if let Screen::Locked(f) = &mut *state.write() {
            f.password = zeroize::Zeroizing::new("secret".into());
            f.pending = Some(crate::platform::jobs::Worker::ready(Err("unused".into())));
        }
        runtime::lock(Some("screen locked"));
        runtime::tick();
        let tabs = consume_context::<Signal<Tabs>>();
        assert!(tabs.peek().sessions.iter().all(
            |s| matches!(&*s.peek(), Screen::Locked(f) if !f.busy() && f.password.is_empty())
        ));
    });
}

#[test]
fn opening_another_profile_keeps_the_previous_space_unlocked() {
    let dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        runtime::select_profile(std::path::PathBuf::from("another-space.lbx"));
        let tabs = consume_context::<Signal<Tabs>>();
        assert_eq!(tabs.peek().sessions.len(), 2);
        assert!(matches!(&*tabs.peek().sessions[0].peek(), Screen::Unlocked(_)));
        assert!(matches!(&*state.peek(), Screen::Locked(f) if f.path.as_deref() == Some(Path::new("another-space.lbx"))));
    });
}

#[test]
fn background_tabs_poll_their_own_workers() {
    let dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        new_tab();
        activate(0);
        // After activation, the stable view signal holds tab 0. Tab 1's worker
        // must open tab 1 without replacing the visible space.
        let tabs = consume_context::<Signal<Tabs>>();
        let mut background = tabs.peek().sessions[1];
        let dir = tempfile::tempdir().unwrap();
        let opened = crate::access::auth::Opened {
            vault: crate::desktop::test_support::unlocked(&dir).vault,
            with_password: false,
            recovery_notice: None,
        };
        if let Screen::Locked(f) = &mut *background.write() {
            f.pending = Some(crate::platform::jobs::Worker::ready(Ok(opened)));
        }
        runtime::tick();
        assert!(
            tabs.peek()
                .sessions
                .iter()
                .all(|s| matches!(&*s.peek(), Screen::Unlocked(_)))
        );
        assert_eq!(tabs.peek().active, 0);
    });
}
