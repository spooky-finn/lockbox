use crate::desktop::i18n::Text;

use crate::desktop::{
    context::{Session, edit_unlocked},
    runtime,
    session::Screen,
    widgets::{
        button::IconButton,
        status::StatusBar,
        symbol::{Icon, Symbol},
    },
};
use dioxus::prelude::*;

#[component]
pub(crate) fn Toolbar() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };

    rsx! {
        header { class: "toolbar",
            crate::desktop::space_switcher::SpaceSwitcher {}
            div { class: "toolbar-status",
                if let Some(sync) = unlocked.vault.sync_state() {
                    {
                        let routine = matches!(unlocked.sync.status, crate::sync::scheduler::Status::Saved | crate::sync::scheduler::Status::Syncing | crate::sync::scheduler::Status::Synced);
                        let summary = language.format(Text::PendingUpload, &[language.sync_status(unlocked.sync.status).to_owned(), sync.outbox.len().to_string()]);
                        rsx! {
                            div { class: if routine { "sync-status sync-compact" } else { "sync-status notice" },
                                p { class: if routine { "sr-only" } else { "sync-message" }, aria_live: "polite", "{summary}" }
                                button { class: "quiet sync-control", r#type: "button", title: "{summary}", aria_label: format!("{}: {summary}", language.text(Text::RetrySync)),
                                    onclick: move |_| edit_unlocked(state, |u| u.retry_sync()),
                                    if routine {
                                        Symbol { icon: if unlocked.sync.status == crate::sync::scheduler::Status::Synced { Icon::Check } else { Icon::Cloud } }
                                        if !sync.outbox.is_empty() { span { class: "sync-count", "{sync.outbox.len()}" } }
                                    } else { {language.text(Text::RetrySync)} }
                                }
                            }
                        }
                    }
                }
                StatusBar {}
            }
            div { class: "actions",
                IconButton { icon: Icon::Lock, label: language.text(Text::LockVault),
                    onclick: move |_| runtime::lock(None),
                }
            }
        }
    }
}
