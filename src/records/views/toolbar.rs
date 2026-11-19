use crate::desktop::i18n::Text;
use crate::sync::scheduler::Status as SyncStatus;

use crate::desktop::{
    context::{Session, edit_unlocked, use_unlocked_selector},
    runtime,
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
    let sync = use_unlocked_selector(|u| {
        u.vault
            .pending_uploads()
            .map(|count| (u.sync.status, count))
    });
    let sync = sync.read();
    let Some(sync) = *sync else {
        return rsx! {};
    };

    rsx! {
        header { class: "toolbar",
            crate::desktop::space_switcher::SpaceSwitcher {}
            div { class: "toolbar-status",
                if let Some((status, pending)) = sync {
                    {
                        let routine = matches!(status, SyncStatus::Saved | SyncStatus::Syncing | SyncStatus::Synced);
                        let summary = language.format(Text::PendingUpload, &[language.sync_status(status).to_owned(), pending.to_string()]);
                        rsx! {
                            div { class: if routine { "sync-status sync-compact" } else { "sync-status notice" },
                                p { class: if routine { "sr-only" } else { "sync-message" }, aria_live: "polite", "{summary}" }
                                button {
                                    class: if status == SyncStatus::Syncing { "quiet sync-control is-syncing" } else { "quiet sync-control" },
                                    r#type: "button", title: "{summary}", aria_label: format!("{}: {summary}", language.text(Text::RetrySync)),
                                    onclick: move |_| edit_unlocked(state, |u| u.retry_sync()),
                                    if routine {
                                        Symbol { icon: if status == SyncStatus::Synced { Icon::Check } else { Icon::Cloud } }
                                        if pending > 0 { span { class: "sync-count", "{pending}" } }
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
