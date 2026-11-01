//! Shared VirtualDom fixture for component tests.

use crate::{
    access::views::locked::LockedView,
    desktop::{
        context::{BiometricProvider, Clock, Session},
        security::{Security, Status},
        session::Screen,
        time::unix_now,
        workspace_view::UnlockedView,
    },
    platform::{
        autolock::{AutoLock, ScreenWatch},
        clipboard::Guard,
        touchid::tests::FakeBiometrics,
    },
    records::entry::{Entry, Totp, TotpAlgorithm},
    settings::{config::Config, state::Settings},
};
use dioxus::prelude::*;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Instant, SystemTime},
};

#[derive(Clone, Default)]
pub(crate) struct RenderCounts(
    pub(crate) Rc<std::cell::Cell<usize>>,
    pub(crate) Rc<std::cell::Cell<usize>>,
);

#[component]
fn SessionProbe() -> Element {
    let count = use_context::<RenderCounts>();
    let state = use_context::<Session>();
    let _screen = state.read();
    count.0.set(count.0.get() + 1);
    rsx! {}
}

#[component]
fn ClockProbe() -> Element {
    let count = use_context::<RenderCounts>();
    let _now = (use_context::<Clock>().0)();
    count.1.set(count.1.get() + 1);
    rsx! {}
}

fn fixture() -> Element {
    let dir = use_hook(|| std::rc::Rc::new(tempfile::tempdir().unwrap()));
    let counts = use_hook(RenderCounts::default);
    use_context_provider(|| counts.clone());
    let state = use_signal(|| {
        let mut unlocked = crate::desktop::test_support::unlocked(&dir);
        let mut entry = Entry::new();
        entry.title = "GitLab".into();
        entry.username = "alice".into();
        entry.password = "private-test-password".into();
        entry.url = "https://gitlab.com".into();
        entry.totp = Some(Totp {
            secret: "JBSWY3DPEHPK3PXP".into(),
            algorithm: TotpAlgorithm::Sha1,
            digits: 6,
            period: 30,
        });
        unlocked.selected = Some(entry.id.clone());
        unlocked.vault.test_entries_mut().push(entry);
        Screen::Unlocked(Box::new(unlocked))
    });
    let settings = use_signal(|| Settings {
        config: Config {
            language: crate::settings::language::Language::Russian,
            ..Config::default()
        },
        path: None,
    });
    let status = use_signal(Status::default);
    let clock = use_signal(unix_now);
    let native = use_hook(|| {
        Rc::new(RefCell::new(Security {
            clipboard: Guard::new(Box::new(
                crate::platform::clipboard::tests::FakeBoard::default(),
            )),
            auto_lock: AutoLock::new(5, Instant::now(), SystemTime::now()),
            screen_watch: ScreenWatch::default(),
        }))
    });
    use_context_provider(|| state);
    use_context_provider(|| settings);
    use_context_provider(|| status);
    use_context_provider(|| Clock(clock));
    use_context_provider(|| native.clone());
    use_context_provider(|| BiometricProvider(Arc::new(FakeBiometrics::default())));
    let body = if matches!(&*state.read(), Screen::Locked(_)) {
        rsx! { LockedView {} }
    } else {
        rsx! { UnlockedView {} }
    };
    rsx! { {body} SessionProbe {} ClockProbe {} }
}

pub(crate) fn mounted() -> VirtualDom {
    let mut dom = VirtualDom::new(fixture);
    dom.rebuild_in_place();
    dom
}

pub(crate) fn update(dom: &mut VirtualDom, edit: impl FnOnce(Session)) {
    dom.in_scope(ScopeId::APP, || edit(consume_context::<Session>()));
    dom.render_immediate_to_vec();
}
