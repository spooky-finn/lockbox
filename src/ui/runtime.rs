//! Dioxus lifecycle and the few actions that coordinate independent domains.

use super::context::{BiometricProvider, Clock, SecurityHandle, Session};
use crate::app::{
    auth::{Locked, Mode, StorageKind, with_vault_extension},
    security::{CopyField, Security, Status},
    session::Screen,
    settings::Settings,
    time::unix_now,
};
use crate::{infra::autolock, infra::touchid};
use dioxus::prelude::*;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

mod import;
mod sharing;
pub(super) use import::choose_import_file;
pub(super) use sharing::{export_sharing_file, import_sharing_file};

pub(super) fn use_runtime() -> Session {
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
    use_context_provider(|| Clock(clock));
    use_context_provider(|| status);

    dioxus::desktop::use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};

        if matches!(
            event,
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } | Event::LoopDestroyed
        ) {
            lock(None);
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

pub(super) fn tick() {
    let mut clock = consume_context::<Clock>().0;
    check_lock();
    consume_context::<SecurityHandle>()
        .borrow_mut()
        .clipboard
        .tick(Instant::now());
    let pending = match &*consume_context::<Session>().peek() {
        Screen::Locked(form) => form.pending.is_some() || form.auto_touch_id,
        Screen::Unlocked(workspace) => {
            workspace.touch_id.pending.is_some()
                || workspace.sharing.needs_poll()
                || (workspace.import.needs_poll() && !workspace.is_editing())
        }
    };

    if pending {
        poll_workers();
    }
    {
        let mut state = consume_context::<Session>();
        // Avoid notifying the whole UI at 10 Hz while the network worker waits.
        let should_tick = matches!(&*state.peek(), Screen::Unlocked(u) if u.sync.needs_tick_for(&u.vault, u.is_editing()));

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
    publish_status();
    let now = unix_now();

    if *clock.peek() != now {
        clock.set(now);
    }
}

pub(super) fn lock(reason: Option<&str>) {
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
    publish_status();
}

fn check_lock() {
    let state = consume_context::<Session>();
    let native = consume_context::<SecurityHandle>();
    let reason = native.borrow().lock_reason();

    if matches!(&*state.peek(), Screen::Unlocked(_))
        && let Some(reason) = reason
    {
        lock(Some(reason));
    }
}

pub(super) fn record_activity() {
    // Expired sessions must lock before the first input after sleep resets activity.
    check_lock();
    consume_context::<SecurityHandle>().borrow_mut().activity();
}

pub(super) fn publish_status() {
    let native = consume_context::<SecurityHandle>();
    let mut status = consume_context::<Signal<Status>>();
    let next = native.borrow().status();

    if *status.peek() != next {
        status.set(next);
    }
}

pub(super) fn poll_workers() {
    let mut state = consume_context::<Session>();
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

pub(super) fn begin_create() {
    let mut state = consume_context::<Session>();
    let mut form = Locked::new(None);
    form.mode = Mode::Create;
    *state.write() = Screen::Locked(Box::new(form));
}

pub(super) fn create_cloud_profile() {
    let mut state = consume_context::<Session>();
    {
        let mut screen = state.write();
        let Screen::Locked(form) = &mut *screen else {
            return;
        };

        if form.pending.is_some() || !form.validate_creation() {
            return;
        }
    }

    let path = dirs::home_dir()
        .ok_or_else(|| std::io::Error::other("Cannot locate home directory"))
        .and_then(|home| new_profile_path(&home));
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

fn new_profile_path(home: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let directory = home.join(".lockbox");
    std::fs::create_dir_all(&directory)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(std::io::Error::other)?;
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(directory.join(format!("profile-{name}.lbx")))
}

pub(super) fn pick_vault(create: bool) {
    let mut state = consume_context::<Session>();
    let settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    spawn(async move {
        let dialog = rfd::AsyncFileDialog::new().add_filter("lockbox", &["lbx"]);
        let path = if create {
            dialog.set_file_name("vault.lbx").save_file().await
        } else {
            dialog.pick_file().await
        };

        if let Some(path) = path {
            let path = path.path().to_path_buf();
            let mut form = if create {
                Locked::new(Some(with_vault_extension(path)))
            } else {
                Locked::new(Some(path)).offer_touch_id(&settings.peek().config, &*hardware.0, false)
            };

            if create {
                form.mode = Mode::Create;
                form.storage_kind = Some(StorageKind::Local);
                form.error = None;
            }
            *state.write() = Screen::Locked(Box::new(form));
        }
    });
}

pub(super) fn forget_path() {
    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    let mut form = Locked::new(None);
    form.error = settings.write().remember(None).err();
    *state.write() = Screen::Locked(Box::new(form));
}

pub(super) fn set_touch_id(wanted: bool) {
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

pub(super) fn set_auto_lock(minutes: u32) {
    if !autolock::CHOICES.contains(&minutes) {
        return;
    }

    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    let native = consume_context::<SecurityHandle>();
    settings.write().config.auto_lock_minutes = minutes;
    native
        .borrow_mut()
        .auto_lock
        .set_minutes(minutes, Instant::now(), SystemTime::now());

    if let Screen::Unlocked(workspace) = &mut *state.write() {
        workspace.error = settings.peek().save().err();
    }
    publish_status();
}

pub(super) fn set_language(language: crate::core::language::Language) {
    let mut settings = consume_context::<Signal<Settings>>();
    let mut state = consume_context::<Session>();
    settings.write().config.language = language;

    if let Err(error) = settings.peek().save() {
        match &mut *state.write() {
            Screen::Locked(form) => form.error = Some(error),
            Screen::Unlocked(workspace) => workspace.error = Some(error),
        }
    }
}

pub(super) fn copy(field: CopyField) {
    let mut state = consume_context::<Session>();
    let native = consume_context::<SecurityHandle>();

    if let Screen::Unlocked(workspace) = &mut *state.write()
        && let Some(entry) = workspace.selected_entry()
    {
        workspace.error = native.borrow_mut().copy(entry, field).err();
    }
    publish_status();
}

pub(super) fn add_attachment() {
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
        let (name, mut data) = crate::infra::attachments::read(path)?;
        draft
            .entry
            .add_attachment(name, std::mem::take(&mut *data))
            .map_err(str::to_owned)
    })()
    .err();
}

pub(super) fn export_attachment(id: &str) {
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
    u.error = crate::infra::attachments::export(path, &file.data).err();
}

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;
