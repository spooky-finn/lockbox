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

    let filename = unlocked
        .vault
        .path()
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    rsx! {
        header { class: "toolbar",
            div { class: "brand",
                div { class: "brand-icon", Symbol { icon: Icon::Key } }
                div { strong { "lockbox" } span { class: "vault-name", "{filename}" } }
            }
            div { class: "toolbar-status",
                if let Some(sync) = unlocked.vault.sync_state() {
                    div { class: if matches!(unlocked.sync.status, crate::sync::scheduler::Status::Saved | crate::sync::scheduler::Status::Syncing | crate::sync::scheduler::Status::Synced) { "sync-status" } else { "sync-status notice" },
                        p { aria_live: "polite", {language.format(Text::PendingUpload, &[language.sync_status(unlocked.sync.status).to_owned(), sync.outbox.len().to_string()])} }
                        button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| {
                            u.retry_sync();
                        }), {language.text(Text::RetrySync)} }
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
