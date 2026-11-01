use crate::ui::i18n::Text;

use crate::app::{security::Status, session::Screen};
use crate::ui::context::Session;
use dioxus::prelude::*;

#[component]
pub(super) fn StatusBar() -> Element {
    let language = crate::ui::i18n::use_language();
    let state = use_context::<Session>();
    let status = use_context::<Signal<Status>>();
    let status = status.read();

    if status.lock_seconds.is_none() && status.clipboard_seconds.is_none() {
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

            if let Some(seconds) = status.clipboard_seconds {
                span { {language.format(Text::CopiedClipboardClearsInS, &[seconds.to_string()])} }
            }
        }
    }
}
