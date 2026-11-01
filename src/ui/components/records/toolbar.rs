use crate::ui::i18n::Text;

use crate::app::session::Screen;
use crate::ui::components::{
    button::IconButton,
    status::StatusBar,
    symbol::{Icon, Symbol},
};
use crate::ui::context::{Session, edit_unlocked};
use crate::ui::runtime;
use dioxus::prelude::*;

#[component]
pub(in crate::ui) fn Toolbar() -> Element {
    let language = crate::ui::i18n::use_language();
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
                    div { class: if matches!(unlocked.sync.status, crate::app::space_sync::Status::Saved | crate::app::space_sync::Status::Syncing | crate::app::space_sync::Status::Synced) { "sync-status" } else { "sync-status notice" },
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
