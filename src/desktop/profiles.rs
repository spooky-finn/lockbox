//! Desktop profile selection, creation and rename. Owns tab coordination and native pickers.
//! Authentication and persistence remain in access workflows and the vault.

use crate::{
    access::auth::{CreationStep, Locked, Mode, StorageKind},
    desktop::{
        context::{BiometricProvider, SecurityHandle, Session},
        i18n::Text,
        runtime::{lock, publish_status},
        session::Screen,
    },
    settings::state::Settings,
};
use dioxus::prelude::*;
use std::path::{Path, PathBuf};

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

/// One stop of the lock-screen carousel.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CarouselStop {
    Vault(PathBuf),
    /// The open-or-create page, where new vaults join the carousel.
    Add,
}

/// Known vaults this tab can open, then the open-or-create page. Vaults open
/// in other tabs are skipped: selecting one would only switch to that tab.
pub(crate) fn adjacent_profiles(
    settings: &Settings,
    current: Option<&Path>,
    open_elsewhere: &[PathBuf],
) -> (Option<CarouselStop>, Option<CarouselStop>) {
    let mut paths = settings.known_profiles();
    if let Some(path) = current
        && !paths.iter().any(|known| known == path)
    {
        paths.insert(0, path.to_owned());
    }
    paths.retain(|path| {
        path.is_file()
            && (Some(path.as_path()) == current
                || !open_elsewhere
                    .iter()
                    .any(|open| crate::desktop::tabs::same_path(open, path)))
    });
    let mut stops: Vec<_> = paths.into_iter().map(CarouselStop::Vault).collect();
    stops.push(CarouselStop::Add);
    let here = current.map_or(CarouselStop::Add, |path| {
        CarouselStop::Vault(path.to_owned())
    });
    let (previous, next) = match stops.iter().position(|stop| *stop == here) {
        Some(index) if stops.len() > 1 => (
            stops.get((index + stops.len() - 1) % stops.len()),
            stops.get((index + 1) % stops.len()),
        ),
        Some(_) => (None, None),
        None => (stops.last(), stops.first()),
    };
    (previous.cloned(), next.cloned())
}

/// Paths held by the other tabs of this window.
pub(crate) fn paths_in_other_tabs() -> Vec<PathBuf> {
    let Some(tabs) = try_consume_context::<Signal<crate::desktop::tabs::Tabs>>() else {
        return Vec::new();
    };
    let tabs = tabs.peek();
    tabs.sessions
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != tabs.active)
        .filter_map(|(_, session)| match &*session.peek() {
            Screen::Unlocked(workspace) => Some(workspace.vault.path().to_path_buf()),
            Screen::Locked(form) => form.path.clone(),
        })
        .collect()
}

pub(crate) fn select_stop(stop: CarouselStop) {
    match stop {
        CarouselStop::Vault(path) => select_profile(path),
        CarouselStop::Add => show_open_or_create(),
    }
}

/// Leave the selected vault for the open-or-create page in this tab.
fn show_open_or_create() {
    let mut state = consume_context::<Session>();
    if !state.peek().can_switch_profile()
        || matches!(&*state.peek(), Screen::Locked(form) if form.path.is_none())
    {
        return;
    }
    let settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    consume_context::<SecurityHandle>()
        .borrow_mut()
        .clipboard
        .clear_now();
    let form = Locked::new(None)
        .with_identity_path(settings.peek().identity_path(), None)
        .offer_touch_id(&settings.peek().config, &*hardware.0, false);
    *state.write() = Screen::Locked(Box::new(form));
    publish_status();
}

#[cfg(test)]
#[path = "tests/profiles.rs"]
mod tests;

pub(crate) fn rename_local_vault(name: &str) -> Result<(), String> {
    let mut state = consume_context::<Session>();
    let mut settings = consume_context::<Signal<Settings>>();
    let language = settings.peek().config.language;
    let mut screen = state.write();
    if !screen.can_switch_profile() {
        return Err(language.text(Text::SpaceSwitchEditing).to_owned());
    }
    let Screen::Unlocked(workspace) = &mut *screen else {
        return Err(language.text(Text::UnlockToRenameVault).to_owned());
    };

    let source = workspace.vault.path().to_path_buf();
    let sync_error = workspace.vault.rename_local(name).map_err(|error| {
        language.format(
            Text::CouldNotRenameVault,
            &[language.message(&error.to_string())],
        )
    })?;
    let destination = workspace.vault.path();
    if destination != source {
        let settings_error = settings
            .write()
            .rename_local_vault(&source, destination)
            .err();
        let errors: Vec<_> = sync_error
            .map(|error| language.format(Text::VaultRenameSyncFailed, &[error.to_string()]))
            .into_iter()
            .chain(settings_error)
            .collect();
        if !errors.is_empty() {
            workspace.error = Some(errors.join("\n"));
        }
    }
    Ok(())
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
