//! Dispatch OS URL events without performing any invitation action on receipt.

use crate::sync::protocol::pairing::{
    ConfirmationLink, LinkReference, MAX_LINK_BYTES, ManagedLink, Ticket,
};
use std::sync::Mutex;

mod confirmation;
pub(super) use confirmation::ConfirmationNotice;

// The first window to claim the invitation presents it for review. Bound pending
// input and preserve the user's first link if another arrives before review.
static INVITATION: Mutex<Option<String>> = Mutex::new(None);

pub fn receive(link: &str) {
    if link.len() > MAX_LINK_BYTES {
        return;
    }
    if link.starts_with("lockbox://browser/") {
        super::browser::receive(link);
    } else if link == link.trim()
        && let Ok(reference) = ConfirmationLink::parse(link)
    {
        confirmation::queue(reference);
    } else if valid_invitation(link) {
        INVITATION
            .lock()
            .unwrap()
            .get_or_insert_with(|| link.to_owned());
    }
}

fn valid_invitation(link: &str) -> bool {
    link == link.trim()
        && (LinkReference::parse(link).is_ok()
            || ManagedLink::parse(link).is_ok()
            || Ticket::parse_link(link).is_ok())
}

fn take_invitation() -> Option<String> {
    INVITATION.lock().unwrap().take()
}

pub(super) fn present_invitation() {
    use crate::access::auth::{Locked, Mode};
    use crate::desktop::{context::Session, session::Screen, tabs};
    use crate::settings::state::Settings;
    use dioxus::prelude::*;

    if INVITATION.lock().unwrap().is_none() {
        return;
    }
    let mut state = consume_context::<Session>();
    let settings = consume_context::<Signal<Settings>>();
    let (reuse, source) = match &*state.peek() {
        Screen::Locked(form) if form.busy() => return,
        Screen::Unlocked(workspace) if workspace.import.busy() => return,
        Screen::Locked(form) => (
            form.path.is_none() && form.mode == Mode::Open && !form.recovering,
            form.path
                .as_ref()
                .filter(|path| settings.peek().config.space_names.contains_key(*path))
                .cloned(),
        ),
        Screen::Unlocked(workspace) => (
            false,
            workspace
                .vault
                .profile_summary()
                .map(|_| workspace.vault.path().to_owned()),
        ),
    };
    let Some(link) = take_invitation() else {
        return;
    };
    if let Screen::Locked(form) = &mut *state.write()
        && form.joining
        && form
            .invitation_link
            .as_deref()
            .is_none_or(|existing| existing == link)
    {
        form.invitation_link = Some(link);
        dioxus::desktop::window().set_focus();
        return;
    }
    if let Screen::Unlocked(workspace) = &mut *state.write()
        && workspace.sharing.joining
        && workspace.sharing.links.link.is_empty()
        && workspace.vault.resolving_link().is_none()
        && workspace.vault.joining_link().is_none()
        && !workspace.is_editing()
    {
        workspace.sharing.links.code.clear();
        workspace.sharing.links.link = link;
        workspace.open_sharing();
        dioxus::desktop::window().set_focus();
        return;
    }
    if !reuse {
        tabs::new_tab();
    }
    let mut form = Locked::begin_join().with_identity_path(settings.peek().identity_path(), source);
    form.invitation_link = Some(link);
    state.set(Screen::Locked(Box::new(form)));
    dioxus::desktop::window().set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_links_only_queue_canonical_invitations() {
        let reference = LinkReference {
            space_name: "Family".into(),
            ticket: crate::sync::protocol::Hash([7; 32]),
        };
        let link = reference.link();
        for invalid in [
            format!("{link}?callback=https://example.com"),
            format!(" {link}"),
            link.replace("lockbox://", "https://"),
            "lockbox://invite/invalid".into(),
            "lockbox://pair/invalid".into(),
        ] {
            receive(&invalid);
        }
        assert!(take_invitation().is_none());
        receive(&link);
        receive(
            &LinkReference {
                space_name: "Other".into(),
                ..reference
            }
            .link(),
        );
        assert_eq!(take_invitation().as_deref(), Some(link.as_str()));
        assert!(take_invitation().is_none());
    }
}
