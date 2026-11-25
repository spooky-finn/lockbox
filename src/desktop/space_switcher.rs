//! In-app profile selection. Only local names and paths are shown before unlock.

use crate::desktop::profiles;
use crate::{
    desktop::{
        context::Session,
        i18n::Text,
        session::Screen,
        widgets::symbol::{Icon, Symbol},
    },
    settings::{config::Config, state::Settings},
};
use dioxus::prelude::*;
use std::path::Path;

#[derive(Clone, Copy, PartialEq)]
enum Rename {
    File,
    Shared,
    Alias,
}

fn close(mut open: Signal<bool>) {
    open.set(false);
    let _ = document::eval("document.getElementById('space-switch-trigger')?.focus()");
}

#[component]
pub(crate) fn SpaceSwitcher() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let mut settings = use_context::<Signal<Settings>>();
    let mut open = use_signal(|| false);
    let mut rename = use_signal(|| None::<(Rename, String)>);
    let mut rename_error = use_signal(|| None::<String>);
    let shared_name = use_memo(move || {
        if !open() {
            return None;
        }
        let screen = state.read();
        let Screen::Unlocked(workspace) = &*screen else {
            return None;
        };
        if !screen.can_switch_profile() || !workspace.vault.can_rename_shared() {
            return None;
        }
        workspace
            .vault
            .profile_summary()
            .map(|profile| profile.space_name.to_owned())
    })();
    let screen = state.read();
    let preferences = settings.read();
    let current = match &*screen {
        Screen::Unlocked(workspace) => Some(workspace.vault.path().to_path_buf()),
        Screen::Locked(form) => form.path.clone(),
    };
    let name = screen
        .vault_name(&preferences.config)
        .unwrap_or_else(|| language.text(Text::ChooseSpace).to_owned());
    let cloud = match &*screen {
        Screen::Unlocked(workspace) => workspace.vault.profile_summary().is_some(),
        Screen::Locked(form) => {
            form.space_name.is_some()
                || current
                    .as_ref()
                    .is_some_and(|path| preferences.config.space_names.contains_key(path))
        }
    };
    let allowed = screen.can_switch_profile();
    let busy = matches!(&*screen, Screen::Locked(form) if form.busy());
    let can_rename = allowed && matches!(&*screen, Screen::Unlocked(_));
    let alias = current
        .as_ref()
        .and_then(|path| preferences.config.vault_aliases.get(path))
        .cloned()
        .unwrap_or_default();
    let filename = current
        .as_deref()
        .and_then(Path::file_name)
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut paths = preferences.known_profiles();
    if let Some(path) = &current
        && !paths.contains(path)
    {
        paths.insert(0, path.clone());
    }
    let rename_form = if open()
        && let Some((kind, value)) = rename()
    {
        let disabled = !can_rename
            || match kind {
                Rename::File => cloud,
                Rename::Shared => shared_name.is_none(),
                Rename::Alias => false,
            };
        let label = match kind {
            Rename::File => Text::VaultFilename,
            Rename::Shared => Text::SharedVaultName,
            Rename::Alias => Text::PersonalVaultName,
        };
        rsx! {
            form { class: "space-rename-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    if !disabled && let Some((kind, value)) = rename() {
                        let result = match kind {
                            Rename::File => profiles::rename_local_vault(&value),
                            Rename::Shared => profiles::rename_shared_vault(&value),
                            Rename::Alias => profiles::rename_vault_for_me(&value),
                        };
                        match result {
                            Ok(()) => close(open),
                            Err(error) => rename_error.set(Some(error)),
                        }
                    }
                },
                label { r#for: "vault-name", {language.text(label)} }
                input { id: "vault-name", r#type: "text", value: "{value}", disabled,
                    autocomplete: "off", autofocus: true,
                    onmounted: move |event: MountedEvent| async move {
                        let _ = event.set_focus(true).await;
                    },
                    oninput: move |event| {
                        rename.set(Some((kind, event.value())));
                        rename_error.set(None);
                    },
                }
                if kind == Rename::Alias {
                    p { class: "muted", {language.text(Text::PersonalVaultNameHelp)} }
                }
                if let Some(error) = rename_error() {
                    p { class: "error", role: "alert", "{error}" }
                }
                div { class: "space-rename-actions",
                    button { r#type: "button", class: "quiet",
                        onclick: move |_| { rename.set(None); rename_error.set(None); },
                        {language.text(Text::Cancel)}
                    }
                    button { r#type: "submit", disabled: disabled || (kind != Rename::Alias && value.trim().is_empty()), {language.text(Text::Save)} }
                }
            }
        }
    } else {
        rsx! {}
    };
    rsx! {
        div { class: "space-switcher",
            onkeydown: move |event: KeyboardEvent| {
                if !open() {
                    return;
                }
                if event.key() == Key::Escape {
                    event.stop_propagation();
                    close(open);
                } else if matches!(event.key(), Key::ArrowLeft | Key::ArrowRight) {
                    event.stop_propagation();
                }
            },
            button {
                id: "space-switch-trigger", class: "space-switch-trigger", r#type: "button",
                aria_label: language.text(Text::SwitchSpace), aria_expanded: open().to_string(),
                aria_controls: "space-switch-panel", disabled: busy,
                onclick: move |_| {
                    if open() { close(open); }
                    else {
                        // Other tabs may have opened or renamed a profile since
                        // this window last wrote its preferences.
                        let path = settings.peek().path.clone();
                        if let Some(path) = path { settings.write().config = Config::load(&path); }
                        rename.set(None);
                        rename_error.set(None);
                        open.set(true);
                    }
                },
                span { class: "brand-icon", Symbol { icon: Icon::Key } }
                span { class: "space-trigger-label",
                    strong { "{name}" }
                    small { if current.is_some() { {language.text(if cloud { Text::TabCloud } else { Text::TabLocal })} } else { "lockbox" } }
                }
                span { class: "space-chevron", Symbol { icon: Icon::ChevronRight } }
            }
            if open() {
                button { class: "space-switch-backdrop", r#type: "button", tabindex: "-1",
                    aria_label: language.text(Text::CloseSpaceSwitcher), onclick: move |_| close(open),
                }
                section { id: "space-switch-panel", class: "space-switch-panel", aria_label: language.text(Text::YourSpaces),
                    div { class: "space-switch-heading",
                        strong { {language.text(Text::YourSpaces)} }
                        button { class: "quiet icon-button", r#type: "button", aria_label: language.text(Text::CloseSpaceSwitcher),
                            onclick: move |_| close(open), Symbol { icon: Icon::Close }
                        }
                    }
                    p { class: "space-switch-hint", role: "status",
                        {language.text(if allowed { Text::SpaceSwitchHint } else { Text::SpaceSwitchEditing })}
                    }
                    div { class: "space-switch-list",
                        if paths.is_empty() { p { class: "space-switch-hint", {language.text(Text::NoKnownSpaces)} } }
                        for path in paths {
                            {
                                let selected = current.as_ref() == Some(&path);
                                let is_cloud = preferences.config.space_names.contains_key(&path) || (selected && cloud);
                                let label = if selected { name.clone() } else { preferences.config.vault_name(&path, None) };
                                let missing = !path.is_file();
                                let display_path = path.display().to_string();
                                rsx! {
                                    button { key: "{display_path}", class: if selected { "space-option selected" } else { "space-option" },
                                        r#type: "button", disabled: !allowed || missing,
                                        aria_current: selected.then_some("true"), title: "{display_path}",
                                        onclick: move |_| {
                                            if !selected { profiles::select_profile(path.clone()); }
                                            close(open);
                                        },
                                        span { class: "space-option-icon", Symbol { icon: if is_cloud { Icon::Cloud } else { Icon::Folder } } }
                                        span { class: "space-option-label",
                                            strong { "{label}" }
                                            small { {language.text(if is_cloud { Text::TabCloud } else { Text::TabLocal })} }
                                            span { class: "space-option-path", "{display_path}" }
                                            if missing { small { {language.text(Text::SpaceFileMissing)} } }
                                        }
                                        if selected { span { title: language.text(Text::CurrentSpace), Symbol { icon: Icon::Check } } }
                                    }
                                }
                            }
                        }
                    }
                    {rename_form}
                    div { class: "space-switch-actions",
                        if can_rename && rename().is_none() {
                            if !cloud {
                                button { class: "quiet", r#type: "button",
                                    onclick: move |_| rename.set(Some((Rename::File, filename.clone()))),
                                    Symbol { icon: Icon::Edit } {language.text(Text::RenameVaultFile)}
                                }
                            }
                            if let Some(shared_name) = shared_name {
                                button { class: "quiet", r#type: "button",
                                    onclick: move |_| rename.set(Some((Rename::Shared, shared_name.clone()))),
                                    Symbol { icon: Icon::Edit } {language.text(Text::RenameSharedVault)}
                                }
                            }
                            button { class: "quiet", r#type: "button",
                                onclick: move |_| rename.set(Some((Rename::Alias, alias.clone()))),
                                Symbol { icon: Icon::Edit } {language.text(Text::RenameVaultForMe)}
                            }
                        }
                        button { class: "quiet", r#type: "button", disabled: !allowed,
                            onclick: move |_| { close(open); profiles::pick_vault(false); },
                            Symbol { icon: Icon::FolderOpen } {language.text(Text::OpenAnotherSpace)}
                        }
                        button { class: "quiet", r#type: "button", disabled: !allowed,
                            onclick: move |_| { close(open); profiles::create_from_switcher(false); },
                            Symbol { icon: Icon::Plus } {language.text(Text::CreateAVault)}
                        }
                        button { class: "quiet", r#type: "button", disabled: !allowed,
                            onclick: move |_| { close(open); profiles::create_from_switcher(true); },
                            Symbol { icon: Icon::Cloud } {language.text(Text::SharingConnect)}
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/space_switcher.rs"]
mod tests;
