use crate::{
    desktop::{
        context::{Session, edit_unlocked, use_unlocked_selector},
        i18n::Text,
        security::Status,
        session::Screen,
        widgets::symbol::{Icon, Symbol},
    },
    sync::scheduler::Status as SyncStatus,
};
use dioxus::prelude::*;

#[component]
pub(crate) fn HeaderStatus() -> Element {
    rsx! {
        div { class: "space-tab-status",
            SyncIndicator {}
            StatusBar {}
        }
    }
}

#[component]
fn SyncIndicator() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let sync = use_unlocked_selector(|u| {
        u.vault
            .pending_uploads()
            .map(|count| (u.sync.status, count))
    });
    let Some(Some((status, pending))) = *sync.read() else {
        return rsx! {};
    };
    let routine = matches!(
        status,
        SyncStatus::Saved | SyncStatus::Syncing | SyncStatus::Synced
    );
    let summary = language.format(
        Text::PendingUpload,
        &[language.sync_status(status).to_owned(), pending.to_string()],
    );

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

#[component]
fn StatusBar() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let status = use_context::<Signal<Status>>();
    let status = status.read();

    if status.lock_seconds.is_none() && !status.clipboard_active {
        return rsx! {};
    }

    let editing = matches!(&*state.read(), Screen::Unlocked(workspace) if workspace.is_editing());
    rsx! {
        div { class: "status", role: "status", aria_live: "polite",
            if let Some(seconds) = status.lock_seconds {
                span { class: "notice", {language.format(Text::AutoLockInS, &[seconds.to_string()])}
                    if editing { {language.text(Text::UnsavedChangesWillBeDiscarded)} }
                }
            }

            if status.clipboard_active {
                span { {language.text(Text::Copied)} }
            }
        }
    }
}
