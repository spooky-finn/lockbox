//! In-app profile selection. Only local names and paths are shown before unlock.

use crate::{
    desktop::{
        context::Session,
        i18n::Text,
        runtime,
        session::Screen,
        widgets::symbol::{Icon, Symbol},
    },
    settings::{config::Config, state::Settings},
};
use dioxus::prelude::*;
use std::path::Path;

fn profile_name(path: &Path, config: &Config) -> String {
    config.space_names.get(path).cloned().unwrap_or_else(|| {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    })
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
    let screen = state.read();
    let preferences = settings.read();
    let current = match &*screen {
        Screen::Unlocked(workspace) => Some(workspace.vault.path().to_path_buf()),
        Screen::Locked(form) => form.path.clone(),
    };
    let name = match &*screen {
        Screen::Unlocked(workspace) => workspace
            .vault
            .profile_summary()
            .map(|profile| profile.space_name.to_owned())
            .unwrap_or_else(|| profile_name(workspace.vault.path(), &preferences.config)),
        Screen::Locked(form) => form
            .space_name
            .clone()
            .or_else(|| {
                current
                    .as_deref()
                    .map(|path| profile_name(path, &preferences.config))
            })
            .unwrap_or_else(|| language.text(Text::ChooseSpace).to_owned()),
    };
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
    let mut paths = preferences.known_profiles();
    if let Some(path) = &current
        && !paths.contains(path)
    {
        paths.insert(0, path.clone());
    }
    rsx! {
        div { class: "space-switcher",
            onkeydown: move |event: KeyboardEvent| {
                if event.key() == Key::Escape && open() {
                    event.stop_propagation();
                    close(open);
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
                                let label = if selected { name.clone() } else { profile_name(&path, &preferences.config) };
                                let missing = !path.is_file();
                                let display_path = path.display().to_string();
                                rsx! {
                                    button { key: "{display_path}", class: if selected { "space-option selected" } else { "space-option" },
                                        r#type: "button", disabled: !allowed || missing,
                                        aria_current: selected.then_some("true"), title: "{display_path}",
                                        onclick: move |_| {
                                            if !selected { runtime::select_profile(path.clone()); }
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
                    div { class: "space-switch-actions",
                        button { class: "quiet", r#type: "button", disabled: !allowed,
                            onclick: move |_| { close(open); runtime::pick_vault(false); },
                            Symbol { icon: Icon::FolderOpen } {language.text(Text::OpenAnotherSpace)}
                        }
                        button { class: "quiet", r#type: "button", disabled: !allowed,
                            onclick: move |_| { close(open); runtime::create_from_switcher(false); },
                            Symbol { icon: Icon::Plus } {language.text(Text::CreateAVault)}
                        }
                        button { class: "quiet", r#type: "button", disabled: !allowed,
                            onclick: move |_| { close(open); runtime::create_from_switcher(true); },
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
