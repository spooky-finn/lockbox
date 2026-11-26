//! Review signed invitations and the exact confirmation returned through the private chat.

use super::{Command, act};
use crate::{
    desktop::{
        context::{Clock, Session, edit_unlocked},
        i18n::{Text, use_language},
        session::Screen,
    },
    sharing::views::{
        role_text,
        verification::{self, CopyTarget},
    },
    sync::protocol::{
        Error, Role,
        pairing::{ConfirmationLink, LinkReference, ManagedLink, Ticket},
    },
    vault::VaultError,
};
use dioxus::prelude::*;

#[component]
pub(super) fn ConfirmationReview(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let clock = use_context::<Clock>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let input = u.sharing.links.confirmation.trim();
    let review = if input.is_empty() {
        None
    } else {
        Some(
            ConfirmationLink::parse(input)
                .map_err(VaultError::from)
                .and_then(|reference| {
                    let (link, contact) = u.vault.review_chat_link(&reference, *clock.0.peek())?;
                    Ok((reference, link, contact))
                }),
        )
    };
    rsx! {
        label { r#for: "chat-confirmation", {language.text(Text::ChatPasteConfirmation)} }
        textarea { id: "chat-confirmation", rows: 2, maxlength: crate::sync::protocol::pairing::MAX_LINK_BYTES as i64,
            autocomplete: "off", spellcheck: "false", value: u.sharing.links.confirmation.as_str(), disabled: blocked,
            oninput: move |event| edit_unlocked(state, |u| u.sharing.links.confirmation = event.value()),
        }
        match review {
            Some(Ok((reference, link, contact))) => rsx! {
                section { class: "sharing-request stack",
                    h3 { {crate::sync::protocol::pairing::display_name(&contact.body.username)} }
                    p { {crate::sync::protocol::pairing::display_name(&link.ticket.body.space_name)} }
                    if let Some(role) = link.ticket.body.chat_role {
                        p { {language.text(role_text(role))} }
                    }
                    p { class: "muted", {language.text(Text::SharingDeviceId)} }
                    code { class: "sharing-code", "{contact.body.certificate.body.device}" }
                    if link.approval.is_none() {
                        button { class: "primary", disabled: blocked, onclick: move |_| act(Command::ApproveChat(reference)), {language.text(Text::LinkApprove)} }
                    } else {
                        p { role: "status", {language.text(Text::ChatApproved)} }
                    }
                }
            },
            Some(Err(VaultError::Sync(Error::MissingDependency))) => rsx! {
                p { role: "status", {language.text(Text::ChatMissingRequest)} }
                button { class: "quiet", disabled: blocked, onclick: move |_| edit_unlocked(state, |u| u.sharing.links.wake()), {language.text(Text::LinkRetry)} }
            },
            Some(Err(VaultError::InvitationRoleMismatch)) => rsx! { p { role: "status", {language.text(Text::ChatRoleMismatch)} } },
            Some(Err(VaultError::Sync(Error::InvitationExpired))) => rsx! { p { role: "status", {language.text(Text::LinkExpired)} } },
            Some(Err(_)) => rsx! { p { role: "status", {language.text(Text::ChatInvalidConfirmation)} } },
            None => rsx! {},
        }
    }
}

#[component]
pub(super) fn ReturnConfirmation() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let mut copied = use_signal(|| false);
    rsx! {
        p { role: "status", {language.text(Text::ChatReturnHint)} }
        div { class: "actions sharing-actions",
            button { class: "primary", onclick: move |_| copied.set(verification::copy(state, language, CopyTarget::Confirmation)),
                {language.text(Text::ChatCopyConfirmation)}
            }
            if copied() { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } }
        }
    }
}

#[derive(PartialEq)]
enum Preview {
    Empty,
    Invalid,
    Legacy,
    Chat {
        space: String,
        role: Role,
        expires: u64,
    },
}

impl Preview {
    fn parse(input: &str) -> Self {
        let input = input.trim();
        if input.is_empty() {
            return Self::Empty;
        }
        if input.starts_with("lockbox://pair/") {
            return match ManagedLink::parse(input) {
                Ok(managed) => {
                    let ticket = &managed.body.ticket.body;
                    match ticket.chat_role {
                        Some(role) => Self::Chat {
                            space: ticket.space_name.clone(),
                            role,
                            expires: ticket.expires_at.0,
                        },
                        None => Self::Legacy,
                    }
                }
                Err(_) => Self::Invalid,
            };
        }
        if LinkReference::parse(input).is_ok()
            || Ticket::parse_link(input).is_ok_and(|ticket| ticket.body.chat_role.is_none())
        {
            Self::Legacy
        } else {
            Self::Invalid
        }
    }
}

#[component]
pub(super) fn JoinDraft(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let clock = use_context::<Clock>();
    // Verify the signed mailbox only when the pasted link changes.
    let draft = use_memo(move || match &*state.read() {
        Screen::Unlocked(u) => u.sharing.links.link.clone(),
        Screen::Locked(_) => String::new(),
    });
    let preview = use_memo(move || Preview::parse(&draft.read()));
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    rsx! {
        label { r#for: "pairing-link", {language.text(Text::LinkPaste)} }
        textarea { id: "pairing-link", rows: 3, maxlength: crate::sync::protocol::pairing::MAX_LINK_BYTES as i64,
            autocomplete: "off", spellcheck: "false", value: u.sharing.links.link.as_str(), disabled: blocked,
            oninput: move |event| edit_unlocked(state, |u| u.sharing.links.link = event.value()),
        }
        match &*preview.read() {
            Preview::Empty => rsx! { p { class: "muted", {language.text(Text::ChatChooseLink)} } },
            Preview::Invalid => rsx! { p { role: "status", {language.text(Text::ChatInvalidInvitation)} } },
            Preview::Chat { space, role, expires } => {
                let reviewed = draft();
                let expired = *clock.0.read() >= *expires;
                rsx! {
                    h3 { "{space}" }
                    p { {language.text(role_text(*role))} }
                    p { {language.format(Text::ChatExpires, &[crate::desktop::time::utc_timestamp(*expires)])} }
                    if expired {
                        p { role: "status", {language.text(Text::LinkExpired)} }
                    } else {
                        p { class: "muted", {language.text(Text::ChatJoinHint)} }
                        button { class: "primary", disabled: blocked, onclick: move |_| act(Command::Join(reviewed.clone())), {language.text(Text::VaultActionJoin)} }
                    }
                }
            }
            Preview::Legacy => rsx! {
                p { class: "muted", {language.text(Text::LinkJoinHint)} }
                label { r#for: "pairing-code", {language.text(Text::SharingVerificationCode)} }
                input { id: "pairing-code", maxlength: 64, autocomplete: "off", spellcheck: "false", value: u.sharing.links.code.as_str(), disabled: blocked,
                    oninput: move |event| edit_unlocked(state, |u| u.sharing.links.code = event.value()),
                }
                p { class: "muted", {language.text(Text::LinkCodeHint)} }
                button { class: "primary", disabled: blocked || u.sharing.links.code.trim().is_empty(), onclick: move |_| act(Command::Request), {language.text(Text::LinkSendRequest)} }
            }
        }
    }
}
