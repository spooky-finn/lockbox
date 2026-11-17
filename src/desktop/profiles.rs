//! Desktop profile selection and creation. Owns tab coordination and native pickers.
//! Authentication and persistence remain in access workflows and the vault.

use crate::{
    access::auth::{CreationStep, Locked, Mode, StorageKind},
    desktop::{
        context::{BiometricProvider, SecurityHandle, Session},
        runtime::{lock, publish_status},
        session::Screen,
    },
    settings::state::Settings,
};
use dioxus::prelude::*;

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
