//! Dioxus lifecycle and the few actions that coordinate independent domains.

use crate::{
    desktop::{
        context::{BiometricProvider, Clock, SecurityHandle, Session},
        security::{CopyField, Security, Status},
        session::Screen,
        time::unix_now,
    },
    platform::touchid,
    settings::state::Settings,
};
use dioxus::prelude::*;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

pub(crate) fn use_runtime() -> Session {
    let settings = use_signal(Settings::load);
    let hardware = use_hook(|| BiometricProvider(touchid::system()));
    let native = use_hook(|| {
        Rc::new(RefCell::new(Security::new(
            settings.peek().config.auto_lock_minutes,
        )))
    });
    let state = use_signal(|| Screen::locked(&settings.peek(), &*hardware.0));
    let clock = use_signal(unix_now);
    let status = use_signal(Status::default);
    use_context_provider(|| settings);
    use_context_provider(|| hardware.clone());
    use_context_provider(|| native.clone());
    use_context_provider(|| state);
    let tabs = use_signal(|| crate::desktop::tabs::Tabs::new(state));
    use_context_provider(|| tabs);
    use_context_provider(|| Clock(clock));
    use_context_provider(|| status);

    // Wry delivers every window's events to every tab: act only on our own.
    let own_window = dioxus::desktop::window().id();
    dioxus::desktop::use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};

        match event {
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } if *window_id == own_window => lock(None),
            Event::LoopDestroyed => lock(None),
            _ => {}
        }
    });
    // The menu bar is shared too; the focused tab handles its commands.
    dioxus::desktop::use_muda_event_handler(move |event| {
        if !dioxus::desktop::window().is_focused() {
            return;
        }

        match event.id().as_ref() {
            crate::desktop::window::NEW_TAB => crate::desktop::tabs::new_tab(),
            crate::desktop::window::CLOSE_TAB => {
                let tabs = consume_context::<Signal<crate::desktop::tabs::Tabs>>();
                let active = tabs.peek().active;
                crate::desktop::tabs::close(active);
            }
            crate::desktop::window::NEW_WINDOW => crate::desktop::window::open(false),
            _ => {}
        }
    });
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            tick();
        }
    });
    state
}

pub(crate) fn tick() {
    let mut clock = consume_context::<Clock>().0;
    check_lock();
    consume_context::<SecurityHandle>()
        .borrow_mut()
        .clipboard
        .tick(Instant::now());
    tick_session(consume_context::<Session>());
    if let Some(tabs) = try_consume_context::<Signal<crate::desktop::tabs::Tabs>>() {
        let sessions: Vec<_> = tabs
            .peek()
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (i != tabs.peek().active).then_some(*s))
            .collect();
        for session in sessions {
            tick_session(session);
        }
    }
    publish_status();
    let now = unix_now();
    if *clock.peek() != now {
        clock.set(now);
    }
}

fn tick_session(mut state: Session) {
    let autosave_due = matches!(&*state.peek(), Screen::Unlocked(u) if
        u.draft.as_ref().and_then(|d| d.autosave_at).is_some_and(|at| Instant::now() >= at));
    if autosave_due {
        state.with_mut(|screen| {
            if let Screen::Unlocked(workspace) = screen {
                // One attempt per edit. Navigation or another edit retries failures.
                if let Some(draft) = &mut workspace.draft {
                    draft.autosave_at = None;
                }
                workspace.error = workspace.autosave_draft().err();
            }
        });
    }

    let pending = match &*state.peek() {
        Screen::Locked(form) => form.busy() || form.auto_touch_id,
        Screen::Unlocked(workspace) => {
            workspace.touch_id.pending.is_some()
                || workspace.sharing.needs_poll()
                || (workspace.import.needs_poll() && !workspace.is_editing())
        }
    };

    if pending {
        poll_session(state);
    }
    {
        let pairing_tick = matches!(&*state.peek(), Screen::Unlocked(u) if u.sharing.links.needs_tick(&u.vault, u.is_editing()));
        if pairing_tick {
            state.with_mut(|screen| {
                if let Screen::Unlocked(workspace) = screen
                    && workspace.tick_pairing(unix_now())
                {
                    let mut settings = consume_context::<Signal<Settings>>();
                    workspace.error = settings
                        .write()
                        .remember_profile(
                            workspace.vault.path().to_path_buf(),
                            workspace
                                .vault
                                .profile_summary()
                                .map(|profile| profile.space_name.to_owned()),
                        )
                        .err();
                }
            });
        }
        // Avoid notifying the whole UI at 10 Hz while the network worker waits.
        let should_tick = matches!(&*state.peek(), Screen::Unlocked(u) if !u.sharing.joining && !u.vault.pairing_waits_to_join() && u.sync.needs_tick_for(&u.vault, u.is_editing()));

        if should_tick {
            state.with_mut(|screen| {
                if let Screen::Unlocked(workspace) = screen {
                    let editing = workspace.is_editing();

                    if let Some(error) = workspace.sync.tick(&mut workspace.vault, editing) {
                        workspace.error = Some(error);
                    }
                }
            });
        }
    }
}

pub(crate) fn lock(reason: Option<&str>) {
    let mut state = consume_context::<Session>();
    let settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    let native = consume_context::<SecurityHandle>();
    state.write().lock(
        &settings.peek(),
        &*hardware.0,
        &mut native.borrow_mut(),
        reason,
    );
    if let Some(tabs) = try_consume_context::<Signal<crate::desktop::tabs::Tabs>>() {
        for (index, session) in tabs.peek().sessions.iter().enumerate() {
            if index != tabs.peek().active {
                let mut session = *session;
                session.write().lock(
                    &settings.peek(),
                    &*hardware.0,
                    &mut native.borrow_mut(),
                    reason,
                );
            }
        }
    }
    publish_status();
}

fn check_lock() {
    let native = consume_context::<SecurityHandle>();
    let Some(reason) = native.borrow().lock_reason() else {
        return;
    };

    let state = consume_context::<Session>();
    let any_unlocked = matches!(&*state.peek(), Screen::Unlocked(_))
        || try_consume_context::<Signal<crate::desktop::tabs::Tabs>>().is_some_and(|tabs| {
            tabs.peek()
                .sessions
                .iter()
                .any(|s| matches!(&*s.peek(), Screen::Unlocked(_)))
        });
    if any_unlocked {
        lock(Some(reason));
    }
}

pub(crate) fn record_activity() {
    // Expired sessions must lock before the first input after sleep resets activity.
    check_lock();
    consume_context::<SecurityHandle>().borrow_mut().activity();
}

pub(crate) fn publish_status() {
    let native = consume_context::<SecurityHandle>();
    let mut status = consume_context::<Signal<Status>>();
    let next = native.borrow().status();

    if *status.peek() != next {
        status.set(next);
    }
}

#[cfg(test)]
pub(crate) fn poll_workers() {
    poll_session(consume_context::<Session>());
}

fn poll_session(mut state: Session) {
    let mut settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    let native = consume_context::<SecurityHandle>();
    let mut screen = state.write();

    match &mut *screen {
        Screen::Locked(form) => {
            let language = settings.peek().config.language;

            if let Some(opened) = form.poll(hardware.for_language(language)) {
                screen.open(
                    opened,
                    &mut settings.write(),
                    &*hardware.0,
                    &mut native.borrow_mut(),
                );
            }
        }
        Screen::Unlocked(workspace) => {
            workspace.poll_sharing(unix_now());
            workspace.poll_import();
            if workspace.touch_id.pending.is_some()
                && let Some(result) = workspace
                    .touch_id
                    .poll(&workspace.vault, &mut settings.write())
            {
                workspace.error = result.err();
            }
        }
    }
}

pub(crate) fn copy(field: CopyField) -> bool {
    let mut state = consume_context::<Session>();
    let native = consume_context::<SecurityHandle>();
    let error = {
        let screen = state.peek();
        let Screen::Unlocked(workspace) = &*screen else {
            return false;
        };
        let Some(entry) = workspace.selected_entry() else {
            return false;
        };
        native.borrow_mut().copy(entry, field).err()
    };
    let copied = error.is_none();
    // Successful copies normally change only clipboard status, not the session.
    let changed = matches!(&*state.peek(), Screen::Unlocked(u) if u.error != error);
    if changed && let Screen::Unlocked(u) = &mut *state.write() {
        u.error = error;
    }
    publish_status();
    copied
}

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;
