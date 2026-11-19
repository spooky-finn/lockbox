//! Open spaces retain independent sessions; the active session feeds existing views.
use crate::{
    access::auth::Locked,
    desktop::{
        context::{BiometricProvider, SecurityHandle, Session},
        i18n::Text,
        session::Screen,
        widgets::{
            button::IconButton,
            status::HeaderStatus,
            symbol::{Icon, Symbol},
        },
    },
    settings::state::Settings,
};
use dioxus::prelude::*;
use std::path::Path;

pub(crate) struct Tabs {
    pub(crate) sessions: Vec<Session>,
    pub(crate) active: usize,
}

impl Tabs {
    pub(crate) fn new(state: Session) -> Self {
        Self {
            sessions: vec![state],
            active: 0,
        }
    }

    fn activate(&mut self, index: usize) {
        if index == self.active || index >= self.sessions.len() {
            return;
        }
        let mut current = self.sessions[self.active];
        let mut target = self.sessions[index];
        std::mem::swap(&mut *current.write(), &mut *target.write());
        self.sessions.swap(self.active, index);
        self.active = index;
    }
}

pub(crate) fn activate(index: usize) {
    let state = consume_context::<Session>();
    // Unlock and native import operations must finish before moving their session.
    if matches!(&*state.peek(), Screen::Locked(f) if f.busy())
        || matches!(&*state.peek(), Screen::Unlocked(u) if u.import.busy())
    {
        return;
    }
    let mut tabs = consume_context::<Signal<Tabs>>();
    consume_context::<SecurityHandle>()
        .borrow_mut()
        .clipboard
        .clear_now();
    tabs.write().activate(index);
    let _ = document::eval(
        "requestAnimationFrame(() => document.querySelector('.space-tab.active')?.scrollIntoView({block: 'nearest', inline: 'nearest'}))",
    );
    crate::desktop::runtime::publish_status();
}

pub(crate) fn new_tab() {
    let state = consume_context::<Session>();
    if matches!(&*state.peek(), Screen::Locked(f) if f.busy())
        || matches!(&*state.peek(), Screen::Unlocked(u) if u.import.busy())
    {
        return;
    }
    let settings = consume_context::<Signal<Settings>>();
    let hardware = consume_context::<BiometricProvider>();
    let form = Locked::new(None)
        .with_identity_path(settings.peek().identity_path(), None)
        .offer_touch_id(&settings.peek().config, &*hardware.0, false);
    let session = Signal::new_in_scope(Screen::Locked(Box::new(form)), ScopeId::APP);
    let mut tabs = consume_context::<Signal<Tabs>>();
    let index = {
        let mut tabs = tabs.write();
        tabs.sessions.push(session);
        tabs.sessions.len() - 1
    };
    activate(index);
}

pub(crate) fn focus_path(path: &Path) -> bool {
    let Some(tabs) = try_consume_context::<Signal<Tabs>>() else {
        return false;
    };
    let index = tabs
        .peek()
        .sessions
        .iter()
        .position(|state| match &*state.peek() {
            Screen::Unlocked(u) => same_path(u.vault.path(), path),
            Screen::Locked(f) => f.path.as_deref().is_some_and(|p| same_path(p, path)),
        });
    if let Some(index) = index {
        activate(index);
        true
    } else {
        false
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || left
            .canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}

pub(crate) fn close(index: usize) {
    let mut tabs = consume_context::<Signal<Tabs>>();
    let session = tabs.peek().sessions[index];
    if !session.peek().can_switch_profile() {
        return;
    }
    if tabs.peek().sessions.len() == 1 {
        consume_context::<SecurityHandle>()
            .borrow_mut()
            .clipboard
            .clear_now();
        let mut session = session;
        session.set(Screen::Locked(Box::new(Locked::new(None))));
        crate::desktop::runtime::publish_status();
        return;
    }
    if index == tabs.peek().active {
        activate(if index == 0 { 1 } else { index - 1 });
        if tabs.peek().active == index {
            return;
        }
    }
    let mut tabs = tabs.write();
    let mut removed = tabs.sessions.remove(index);
    // Dropping a Signal handle alone retains its value until its owner unmounts.
    removed.set(Screen::Locked(Box::new(Locked::new(None))));
    if index < tabs.active {
        tabs.active -= 1;
    }
}

#[component]
pub(crate) fn TabBar() -> Element {
    let language = crate::desktop::i18n::use_language();
    let tabs = use_context::<Signal<Tabs>>();
    let settings = use_context::<Signal<Settings>>();
    let tabs = tabs.read();
    let active_index = tabs.active;
    let active = tabs.sessions[active_index].read();
    let busy = matches!(&*active, Screen::Locked(f) if f.busy())
        || matches!(&*active, Screen::Unlocked(u) if u.import.busy());
    rsx! {
        nav { class: "space-tabs", aria_label: language.text(Text::YourSpaces),
            if cfg!(target_os = "macos") {
                div { class: "space-tabs-window-controls", aria_hidden: "true",
                    onmousedown: move |_| dioxus::desktop::window().drag(),
                }
            }
            div { class: "space-tab-list", role: "tablist",
                onkeydown: move |event: KeyboardEvent| {
                    let tabs = consume_context::<Signal<Tabs>>();
                    let count = tabs.peek().sessions.len();
                    let current = tabs.peek().active;
                    let next = match event.key() {
                        Key::ArrowRight => Some((current + 1) % count),
                        Key::ArrowLeft => Some((current + count - 1) % count),
                        Key::Home => Some(0), Key::End => Some(count - 1), _ => None,
                    };
                    if let Some(next) = next {
                        event.prevent_default(); event.stop_propagation(); activate(next);
                        let _ = document::eval("requestAnimationFrame(() => document.querySelector('[role=tab][aria-selected=true]')?.focus())");
                    }
                },
                for (index, session) in tabs.sessions.iter().enumerate() {
                    { let screen = session.read();
                      let label = match &*screen {
                        Screen::Unlocked(u) => u.vault.profile_summary().map(|p| p.space_name.to_owned())
                            .unwrap_or_else(|| u.vault.path().file_stem().unwrap_or_default().to_string_lossy().into_owned()),
                        Screen::Locked(f) => f.space_name.clone().or_else(|| f.path.as_ref().map(|p| settings.read().config.space_names.get(p).cloned().unwrap_or_else(|| p.file_stem().unwrap_or_default().to_string_lossy().into_owned()))).unwrap_or_else(|| language.text(Text::ChooseSpace).to_owned()),
                      };
                      let (icon, tooltip) = match &*screen {
                        Screen::Unlocked(u) => (if u.vault.profile_summary().is_some() { Icon::Cloud } else { Icon::Folder },
                            format!("{}\n{}", crate::desktop::window::title(&screen, settings.read().config.language), u.vault.path().display())),
                        Screen::Locked(f) => (Icon::Lock, f.path.as_ref().map(|p| format!("{label}\n{}", p.display())).unwrap_or_else(|| label.clone())),
                      };
                      rsx! { div { key: "{index}", class: if index == tabs.active { "space-tab active" } else { "space-tab" },
                        button { id: "space-tab-{index}", class: "space-tab-select", r#type: "button", role: "tab", aria_label: "{label}", aria_controls: "space-tab-content", tabindex: if index == tabs.active { "0" } else { "-1" }, disabled: busy, aria_selected: (index == tabs.active).to_string(), title: "{tooltip}", onclick: move |_| activate(index),
                            span { class: "space-tab-icon", aria_hidden: "true", Symbol { icon } }
                            span { class: "space-tab-label", "{label}" }
                        }
                        button { class: "space-tab-close quiet", r#type: "button", aria_label: format!("{}: {label}", language.text(Text::MenuCloseTab)), title: format!("{}: {label}", language.text(Text::MenuCloseTab)), disabled: !screen.can_switch_profile(), onclick: move |_| close(index), Symbol { icon: Icon::Close } }
                      } }
                    }
                }
            }
            button { class: "space-tab-add quiet icon-button", r#type: "button", disabled: busy, aria_label: language.text(Text::MenuNewTab), title: language.text(Text::MenuNewTab), onclick: move |_| new_tab(), Symbol { icon: Icon::Plus } }
            div { class: "space-tabs-drag", aria_hidden: "true",
                onmousedown: move |_| { if cfg!(target_os = "macos") { dioxus::desktop::window().drag(); } },
            }
            if matches!(&*active, Screen::Unlocked(_)) {
                div { class: "space-tab-actions",
                    HeaderStatus {}
                    crate::desktop::space_switcher::SpaceSwitcher { key: "{active_index}" }
                    IconButton { class: "quiet", icon: Icon::Lock, label: language.text(Text::LockVault),
                        onclick: move |_| crate::desktop::runtime::lock(None),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/tabs.rs"]
mod tests;
