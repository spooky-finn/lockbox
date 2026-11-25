//! A return link can identify its issuing vault only after that vault is unlocked.

use crate::{
    desktop::{
        context::Session,
        i18n::{Text, use_language},
        session::Screen,
        tabs::{self, Tabs},
    },
    sync::protocol::pairing::ConfirmationLink,
};
use dioxus::prelude::*;
use std::sync::LazyLock;
use tokio::sync::watch;

// Keep the latest opened return link for all windows. Claim and dismissal both
// compare the full reference so a stale window cannot clear a later link.
static CONFIRMATION: LazyLock<watch::Sender<Option<ConfirmationLink>>> =
    LazyLock::new(|| watch::channel(None).0);

pub(super) fn queue(reference: ConfirmationLink) {
    CONFIRMATION.send_replace(Some(reference));
}

fn take_confirmation(reference: ConfirmationLink) -> bool {
    CONFIRMATION.send_if_modified(|pending| pending.take_if(|link| *link == reference).is_some())
}

// Reads subscribe the caller to tab switches, unlocks and changes to an open draft.
fn present_confirmation(reference: ConfirmationLink) -> Result<bool, Text> {
    let mut active = consume_context::<Session>();
    let tabs = consume_context::<Signal<Tabs>>();
    let tabs = tabs.read();
    let index = tabs
        .sessions
        .iter()
        .position(|session| {
            let screen = session.read();
            let Screen::Unlocked(workspace) = &*screen else {
                return false;
            };
            workspace.vault.issued_links().any(|issued| {
                issued.ticket.body.chat_role.is_some()
                    && issued.ticket.hash().ok() == Some(reference.ticket)
            })
        })
        .ok_or(Text::ConfirmationUnlockOwner)?;
    let target = tabs.sessions[index];
    if matches!(&*target.read(), Screen::Unlocked(u) if !u.editor_is_clear()) {
        return Err(Text::ConfirmationFinishEditing);
    }
    if matches!(&*active.read(), Screen::Locked(f) if f.busy())
        || matches!(&*active.read(), Screen::Unlocked(u) if u.import.busy())
        || matches!(&*target.read(), Screen::Unlocked(u) if u.import.busy())
        || !take_confirmation(reference)
    {
        return Ok(false);
    }
    let switch = tabs.active != index;
    drop(tabs);
    if switch {
        tabs::activate(index);
    }
    if let Screen::Unlocked(u) = &mut *active.write() {
        u.sharing.links.confirmation = reference.link();
        u.open_sharing();
    }
    Ok(true)
}

#[component]
pub(crate) fn ConfirmationNotice() -> Element {
    let language = use_language();
    let mut pending = use_signal(|| *CONFIRMATION.borrow());
    let mut notice = use_signal(|| None);
    use_future(move || async move {
        let mut updates = CONFIRMATION.subscribe();
        loop {
            let next = *updates.borrow_and_update();
            if *pending.peek() != next {
                pending.set(next);
            }
            if updates.changed().await.is_err() {
                break;
            }
        }
    });
    use_effect(move || {
        let reference = *pending.read();
        let result = reference.map(present_confirmation);
        if result == Some(Ok(true)) {
            dioxus::desktop::window().set_focus();
            pending.set(None);
        }
        notice.set(result.and_then(Result::err));
    });
    let Some(text) = notice() else {
        return rsx! {};
    };
    let Some(reference) = pending() else {
        return rsx! {};
    };
    rsx! {
        div { class: "actions update-notice",
            span { role: "status", {language.text(text)} }
            button { class: "quiet", r#type: "button",
                onclick: move |_| { take_confirmation(reference); pending.set(None); },
                {language.text(Text::Cancel)}
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/confirmation_links.rs"]
mod tests;
