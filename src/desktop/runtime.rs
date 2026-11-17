//! Dioxus lifecycle and the few actions that coordinate independent domains.

use crate::{
    access::auth::{CreationStep, Locked, Mode, StorageKind},
    desktop::{
        context::{BiometricProvider, Clock, SecurityHandle, Session},
        security::{CopyField, Security, Status},
        session::Screen,
        time::unix_now,
    },
    platform::{autolock, touchid},
    settings::state::Settings,
};
use dioxus::prelude::*;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
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
                if let Screen::Unlocked(workspace) = screen {
                    let editing = workspace.is_editing();
                    let was_joining = workspace.vault.pairing_waits_to_join();
                    if let Some(error) = workspace.sharing.links.tick(
                        &mut workspace.vault,
                        &mut workspace.sync,
                        editing,
                        unix_now(),
                    ) {
                        workspace.error = Some(error);
                    }
                    if was_joining
                        && !workspace.vault.pairing_waits_to_join()
                        && workspace
                            .vault
                            .joining_link()
                            .is_some_and(|link| link.joined)
                    {
                        workspace.sharing.joining = false;
                        workspace.sync.saved();
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
    let state = consume_context::<Session>();
    let native = consume_context::<SecurityHandle>();
    let reason = native.borrow().lock_reason();

    let any_unlocked = matches!(&*state.peek(), Screen::Unlocked(_))
        || try_consume_context::<Signal<crate::desktop::tabs::Tabs>>().is_some_and(|tabs| {
            tabs.peek()
                .sessions
                .iter()
                .any(|s| matches!(&*s.peek(), Screen::Unlocked(_)))
        });
    if any_unlocked && let Some(reason) = reason {
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
            if let Some(result) = workspace
                .touch_id
                .poll(&workspace.vault, &mut settings.write())
            {
                workspace.error = result.err();
            }
        }
    }
}

pub(crate) fn begin_create() {
    begin_creation(false);
}

pub(crate) fn select_profile(path: std::path::PathBuf) {
    if crate::desktop::tabs::focus_path(&path) {
        return;
    }
    if try_consume_context::<Signal<crate::desktop::tabs::Tabs>>().is_some()
        && matches!(&*consume_context::<Session>().peek(), Screen::Unlocked(_))
    {
        crate::desktop::tabs::new_tab();
    }
    let mut state = consume_context::<Session>();
    let settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    let native = consume_context::<SecurityHandle>();
    state.write().select_profile(
        path,
        &settings.peek(),
        &*hardware.0,
        &mut native.borrow_mut(),
    );
    publish_status();
}

pub(crate) fn create_from_switcher(joining: bool) {
    if !consume_context::<Session>().peek().can_switch_profile() {
        return;
    }
    let state = consume_context::<Session>();
    let source = match &*state.peek() {
        Screen::Unlocked(u) => u
            .vault
            .profile_summary()
            .map(|_| u.vault.path().to_path_buf()),
        Screen::Locked(f) => f.identity_source.clone(),
    };
    if try_consume_context::<Signal<crate::desktop::tabs::Tabs>>().is_some() {
        crate::desktop::tabs::new_tab();
    } else {
        lock(None);
    }
    begin_creation(joining);
    if let Some(source) = source {
        let mut state = state;
        let settings = consume_context::<Signal<Settings>>();
        if let Screen::Locked(form) = &mut *state.write() {
            let replacement = if joining {
                Locked::begin_join()
            } else {
                Locked::begin_create()
            };
            **form = replacement.with_identity_path(settings.peek().identity_path(), Some(source));
        }
    }
}

pub(crate) fn begin_join() {
    begin_creation(true);
}

pub(crate) fn begin_cloud_restore() {
    let mut state = consume_context::<Session>();
    let settings = consume_context::<Signal<Settings>>();
    *state.write() = Screen::Locked(Box::new(
        Locked::begin_cloud_restore().with_identity_path(settings.peek().identity_path(), None),
    ));
}

fn begin_creation(joining: bool) {
    let mut state = consume_context::<Session>();
    let settings = consume_context::<Signal<Settings>>();
    let mut screen = state.write();
    if let Screen::Locked(form) = &mut *screen
        && form.mode == Mode::Create
    {
        form.switch_creation(joining);
        return;
    }
    let form = if joining {
        Locked::begin_join()
    } else {
        Locked::begin_create()
    };
    let source = match &*screen {
        Screen::Locked(form) => form
            .path
            .as_ref()
            .filter(|path| settings.peek().config.space_names.contains_key(*path))
            .cloned(),
        Screen::Unlocked(workspace) => workspace
            .vault
            .profile_summary()
            .map(|_| workspace.vault.path().to_path_buf()),
    };
    *screen = Screen::Locked(Box::new(
        form.with_identity_path(settings.peek().identity_path(), source),
    ));
}

pub(crate) fn create_cloud_profile() {
    let mut state = consume_context::<Session>();
    {
        let mut screen = state.write();
        let Screen::Locked(form) = &mut *screen else {
            return;
        };

        if form.busy() || !form.validate_creation() {
            return;
        }
        // A failed recovery may already have durably reserved root authority
        // for this exact target. Resume it rather than allocating a competitor.
        if form.cloud_recovery.restoring && form.path.as_ref().is_some_and(|path| path.exists()) {
            form.submit();
            return;
        }
    }

    let path = dirs::home_dir()
        .ok_or_else(|| std::io::Error::other("Cannot locate home directory"))
        .and_then(|home| crate::vault::paths::new_profile_path(&home));
    if let Screen::Locked(form) = &mut *state.write() {
        match path {
            Ok(path) => {
                form.path = Some(path);
                form.submit();
            }
            Err(error) => form.error = Some(format!("Cannot create profile directory: {error}")),
        }
    }
}

pub(crate) fn pick_vault(create: bool) {
    let mut state = consume_context::<Session>();
    if !state.peek().can_switch_profile() {
        return;
    }
    if create {
        let mut screen = state.write();
        let Screen::Locked(form) = &mut *screen else {
            return;
        };
        if form.mode != Mode::Create
            || form.creation_step != CreationStep::Sync
            || form.joining
            || form.busy()
        {
            return;
        }
        form.storage_kind = Some(StorageKind::Local);
    }
    spawn(async move {
        let dialog = rfd::AsyncFileDialog::new().add_filter("lockbox", &["lbx"]);
        let path = if create {
            dialog.set_file_name("vault.lbx").save_file().await
        } else {
            dialog.pick_file().await
        };

        if let Some(path) = path {
            let path = path.path().to_path_buf();
            if create {
                if let Screen::Locked(form) = &mut *state.write()
                    && form.storage_kind == Some(StorageKind::Local)
                {
                    form.choose_local_path(path);
                }
            } else {
                select_profile(path);
            }
        }
    });
}

pub(crate) fn forget_path() {
    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    let mut form = Locked::new(None);
    form.error = settings.write().remember(None).err();
    *state.write() = Screen::Locked(Box::new(form));
}

pub(crate) fn set_touch_id(wanted: bool) {
    let mut state = consume_context::<Session>();
    let mut settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    let biometrics = hardware.for_language(settings.peek().config.language);

    if let Screen::Unlocked(workspace) = &mut *state.write() {
        workspace.error =
            workspace
                .touch_id
                .set(&workspace.vault, &mut settings.write(), biometrics, wanted);
    }
}

pub(crate) fn set_auto_lock(minutes: u32) {
    if !autolock::CHOICES.contains(&minutes) {
        return;
    }

    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    let native = consume_context::<SecurityHandle>();
    let saved = settings
        .write()
        .update(|config| config.auto_lock_minutes = minutes);
    native
        .borrow_mut()
        .auto_lock
        .set_minutes(minutes, Instant::now(), SystemTime::now());

    if let Screen::Unlocked(workspace) = &mut *state.write() {
        workspace.error = saved.err();
    }
    publish_status();
}

pub(crate) fn set_language(language: crate::settings::language::Language) {
    update_preferences(|config| config.language = language);
}

pub(crate) fn set_website_icons(enabled: bool) {
    update_preferences(|config| config.website_icons = enabled);
    if !enabled {
        let mut state = consume_context::<Session>();
        if let Screen::Unlocked(workspace) = &mut *state.write() {
            workspace.favicon_cache = Default::default();
        }
    }
}

pub(crate) fn set_theme(theme: crate::settings::appearance::ThemeMode) {
    update_preferences(|config| config.appearance.theme = theme);
}

pub(crate) fn set_theme_preset(preset: crate::settings::appearance::ThemePreset) {
    update_preferences(|config| {
        config.appearance.preset = preset;
        config.appearance.accent = None;
        config.appearance.background = None;
    });
}

pub(crate) fn set_gradient(enabled: bool) {
    update_preferences(|config| config.appearance.gradient = enabled);
}

#[cfg(test)]
pub(crate) fn set_accent_color(value: Option<&str>) {
    set_color(value, |config, color| config.appearance.accent = color);
}

#[cfg(test)]
pub(crate) fn set_background_color(value: Option<&str>) {
    set_color(value, |config, color| config.appearance.background = color);
}

#[cfg(test)]
fn set_color(
    value: Option<&str>,
    change: impl FnOnce(
        &mut crate::settings::config::Config,
        Option<crate::settings::appearance::RgbColor>,
    ),
) {
    let color = match value {
        Some(value) => {
            let Some(color) = crate::settings::appearance::RgbColor::parse(value) else {
                return;
            };
            Some(color)
        }
        None => None,
    };
    update_preferences(|config| change(config, color));
}

fn update_preferences(change: impl FnOnce(&mut crate::settings::config::Config)) {
    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    let saved = settings.write().update(change);

    if let Err(error) = saved {
        match &mut *state.write() {
            Screen::Locked(form) => form.error = Some(error),
            Screen::Unlocked(workspace) => workspace.error = Some(error),
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

pub(crate) fn add_attachment() {
    let state = consume_context::<Session>();
    let selected = match &*state.peek() {
        Screen::Unlocked(u) => u
            .draft
            .as_ref()
            .map(|d| (u.vault.path().to_path_buf(), d.entry.id.clone())),
        _ => None,
    };
    let Some((vault_path, draft_id)) = selected else {
        return;
    };

    // A blocking native dialog can re-enter Dioxus with its callback borrowed.
    spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new().pick_file().await {
            finish_add_attachment(state, &vault_path, &draft_id, path.path());
        }
    });
}

fn finish_add_attachment(
    mut state: Session,
    vault_path: &std::path::Path,
    draft_id: &str,
    path: &std::path::Path,
) {
    let mut screen = state.write();
    let Screen::Unlocked(u) = &mut *screen else {
        return;
    };
    if u.vault.path() != vault_path {
        return;
    }
    let Some(draft) = &mut u.draft else {
        return;
    };
    if draft.entry.id != draft_id {
        return;
    }
    u.error = (|| {
        let (name, mut data) = crate::records::attachments::read(path)?;
        draft
            .entry
            .add_attachment(name, std::mem::take(&mut *data))
            .map_err(str::to_owned)
    })()
    .err();
}

pub(crate) fn export_attachment(id: &str) {
    let state = consume_context::<Session>();
    let selected = match &*state.peek() {
        Screen::Unlocked(u) => u.selected_entry().and_then(|e| {
            e.attachments
                .iter()
                .find(|f| f.id == id)
                .map(|f| (u.vault.path().to_path_buf(), e.id.clone(), f.name.clone()))
        }),
        _ => None,
    };
    let Some((vault_path, entry_id, name)) = selected else {
        return;
    };
    let id = id.to_owned();

    spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_file_name(&name)
            .save_file()
            .await
        {
            finish_export_attachment(state, &vault_path, &entry_id, &id, path.path());
        }
    });
}

fn finish_export_attachment(
    mut state: Session,
    vault_path: &std::path::Path,
    entry_id: &str,
    id: &str,
    path: &std::path::Path,
) {
    let mut screen = state.write();
    let Screen::Unlocked(u) = &mut *screen else {
        return;
    };
    if u.vault.path() != vault_path {
        return;
    }
    let Some(entry) = u.selected_entry().filter(|e| e.id == entry_id) else {
        return;
    };
    let Some(file) = entry.attachments.iter().find(|f| f.id == id) else {
        return;
    };
    u.error = crate::records::attachments::export(path, &file.data).err();
}

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;
