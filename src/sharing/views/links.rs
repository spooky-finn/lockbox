//! Link-first invitation screens. Every approval captures one immutable signed request.

use crate::{
    desktop::{
        context::{Clock, Session, edit_unlocked},
        i18n::{Text, use_language},
        session::Screen,
    },
    sharing::views::{
        role_text,
        verification::{self, CopyTarget, DeviceVerification},
    },
    sync::protocol::{Hash, Role, pairing::ClosedReason},
};
use dioxus::prelude::*;

#[derive(Clone, Copy)]
enum Command {
    Create,
    Request,
    Approve(Hash, Hash, Role, bool),
    Reject(Hash, Hash),
    Cancel(Hash),
    Consent(Hash),
    Retry,
    Dismiss(Hash),
    Forget(Hash),
}

fn act(command: Command) {
    let state = consume_context::<Session>();
    let now = *(consume_context::<Clock>().0).peek();
    let mut settings = consume_context::<Signal<crate::settings::state::Settings>>();
    edit_unlocked(state, |u| {
        if u.is_editing() || u.sharing.busy() {
            return;
        }
        u.sharing.links.suspend();
        u.error = (|| -> crate::vault::Result<()> {
            match command {
                Command::Create => {
                    u.vault.create_link(now, u.sharing.links.days)?;
                }
                Command::Request => {
                    u.vault
                        .request_link(&u.sharing.links.link, &u.sharing.links.code, now)?;
                    u.sharing.joining = true;
                    u.sharing.links.link.clear();
                    u.sharing.links.code.clear();
                    u.sync.suspend();
                }
                Command::Approve(session, request, role, verified) => {
                    u.sync.suspend();
                    u.vault
                        .approve_link_request(session, request, role, verified, now)?;
                }
                Command::Reject(session, request) => {
                    u.vault.reject_link_request(session, request)?
                }
                Command::Cancel(session) => u.vault.cancel_link(session)?,
                Command::Consent(session) => {
                    u.sync.suspend();
                    u.vault.consent_link(session, now)?;
                }
                Command::Retry => u.vault.retry_pairing()?,
                Command::Dismiss(session) => {
                    u.vault.dismiss_join_link(session)?;
                    u.sharing.links.link.clear();
                    u.sharing.links.code.clear();
                }
                Command::Forget(session) => u.vault.forget_link(session)?,
            }
            Ok(())
        })()
        .err()
        .map(|error| error.to_string());
        if u.error.is_none() {
            u.sharing.links.wake();
            if u.vault.joining_link().is_some_and(|link| link.joined) {
                u.error = settings
                    .write()
                    .remember_profile(
                        u.vault.path().to_path_buf(),
                        u.vault
                            .profile_summary()
                            .map(|profile| profile.space_name.to_owned()),
                    )
                    .err();
            }
        }
    });
}

#[component]
pub(super) fn LinkInvitations(blocked: bool) -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    if u.is_controller() && !u.sharing.joining {
        rsx! { OwnerLinks { blocked } }
    } else if !u.sharing.joining && u.vault.joining_link().is_none() {
        rsx! {}
    } else {
        rsx! { JoinLink { blocked } }
    }
}

#[component]
fn OwnerLinks(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let has_active = u.vault.issued_links().any(|link| link.closed.is_none());
    rsx! {
        section { class: "settings-section stack", aria_label: language.text(Text::LinkInvitations),
            h2 { {language.text(Text::LinkInvitations)} }
            p { class: "muted", {language.text(Text::LinkOwnerHint)} }
            for link in u.vault.issued_links() {
                LinkCard { key: "{link.ticket.body.id}", session: link.ticket.body.id, blocked }
            }
            details { class: "sharing-file-options",
                summary { {language.text(Text::SharingExpiry)} ": "
                    {language.text(match u.sharing.links.days { 1 => Text::SharingOneDay, 30 => Text::SharingThirtyDays, _ => Text::SharingSevenDays })}
                }
                label { r#for: "link-expiry", {language.text(Text::SharingExpiry)} }
                select { id: "link-expiry", value: "{u.sharing.links.days}", disabled: blocked,
                    onchange: move |event| edit_unlocked(state, |u| { if let Ok(days @ (1 | 7 | 30)) = event.value().parse() { u.sharing.links.days = days; } }),
                    option { value: "1", {language.text(Text::SharingOneDay)} }
                    option { value: "7", {language.text(Text::SharingSevenDays)} }
                    option { value: "30", {language.text(Text::SharingThirtyDays)} }
                }
            }
            button { class: if has_active { "quiet" } else { "primary" }, disabled: blocked, onclick: move |_| act(Command::Create), {language.text(Text::LinkCreate)} }
        }
    }
}

#[component]
fn LinkCard(session: Hash, blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let mut copied = use_signal(|| false);
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let Some(link) = u
        .vault
        .issued_links()
        .find(|link| link.ticket.body.id == session)
    else {
        return rsx! {};
    };
    rsx! {
        div { class: "sharing-link-card stack",
            if let Some(closed) = &link.closed {
                p { role: "status", {language.text(closed_text(closed.body.reason))} }
                button { class: "quiet", disabled: blocked || !u.vault.can_forget_link(session), onclick: move |_| act(Command::Forget(session)), {language.text(Text::LinkForget)} }
            } else {
                if link.approval.is_none() {
                    p { {language.text(Text::LinkReady)} }
                    div { class: "actions sharing-actions",
                        button { class: "primary", disabled: blocked, onclick: move |_| copied.set(verification::copy(state, language, CopyTarget::Link(session))), {language.text(Text::LinkCopy)} }
                        if copied() { span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} } }
                    }
                    p { class: "muted", {language.text(Text::LinkCodeHint)} }
                    div { class: "actions sharing-actions",
                        button { class: "quiet", onclick: move |_| copied.set(verification::copy(state, language, CopyTarget::LinkCode(session))), {language.text(Text::SharingCopyCode)} }
                    }
                    details { class: "sharing-message-preview",
                        summary { {language.text(Text::LinkShow)} }
                        pre { class: "sharing-message", "{link.ticket.link().unwrap_or_default()}" }
                        p { {language.text(Text::SharingVerificationCode)} }
                        code { class: "sharing-code", "{link.ticket.verification_code().map(|code| code.to_string()).unwrap_or_default()}" }
                    }
                    if link.requests.is_empty() { p { role: "status", {language.text(Text::LinkWaitingRequests)} } }
                } else {
                    p { role: "status", {language.text(Text::LinkApprovedWaiting)} }
                }
                for (request, _) in link.requests.iter().filter(|(request, _)| !link.rejected.contains(request)) {
                    RequestCard { key: "{session}:{request}", session, request: *request, blocked }
                }
                button { class: "quiet", disabled: blocked, onclick: move |_| act(Command::Cancel(session)), {language.text(Text::LinkCancel)} }
            }
        }
    }
}

#[component]
fn RequestCard(session: Hash, request: Hash, blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let mut verified = use_signal(|| false);
    let mut selected = use_signal(|| Role::Reader);
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let Some(link) = u
        .vault
        .issued_links()
        .find(|link| link.ticket.body.id == session)
    else {
        return rsx! {};
    };
    let Some(contact) = link.requests.get(&request) else {
        return rsx! {};
    };
    let certificate = &contact.body.certificate.body;
    let existing_role = u.vault.space_members().ok().and_then(|members| {
        members
            .into_iter()
            .find(|(certificate, _)| certificate.body.user == contact.body.certificate.body.user)
            .map(|(_, role)| role)
    });
    let role = existing_role.unwrap_or(*selected.read());
    rsx! {
        section { class: "sharing-request stack",
            h3 { "{contact.body.username}" }
            p { class: "muted", {language.text(Text::SharingUserId)} }
            code { class: "sharing-code", "{certificate.user}" }
            p { class: "muted", {language.text(Text::SharingDeviceId)} }
            code { class: "sharing-code", "{certificate.device}" }
            if link.approval.is_none() {
                p { class: "muted", {language.text(Text::SharingCompareDeviceHint)} }
                label { class: "sharing-check",
                    input { r#type: "checkbox", checked: verified(), disabled: blocked, onchange: move |event| verified.set(event.checked()) }
                    {language.text(Text::SharingVerifyRecipient)}
                }
                if existing_role.is_none() {
                    label { {language.text(Text::SharingAccessRole)}
                        select { value: if role == Role::Editor { "editor" } else { "reader" }, disabled: blocked,
                            onchange: move |event| selected.set(if event.value() == "editor" { Role::Editor } else { Role::Reader }),
                            option { value: "reader", {language.text(Text::SharingReader)} }
                            option { value: "editor", {language.text(Text::SharingEditor)} }
                        }
                    }
                } else { p { {language.text(role_text(role))} } }
                div { class: "actions sharing-actions",
                    button { class: "primary", disabled: blocked || !verified(), onclick: move |_| act(Command::Approve(session, request, role, verified())), {language.text(Text::LinkApprove)} }
                    button { class: "quiet", disabled: blocked, onclick: move |_| act(Command::Reject(session, request)), {language.text(Text::LinkReject)} }
                }
            } else if link.approval.as_ref().is_some_and(|approval| approval.request == request) {
                p { {language.text(role_text(link.approval.as_ref().unwrap().role))} }
            } else {
                p { class: "muted", {language.text(Text::LinkOtherRequest)} }
            }
        }
    }
}

#[component]
fn JoinLink(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let session = u.vault.joining_link().map(|link| link.ticket.body.id);
    rsx! {
        section { class: "settings-section stack", aria_label: language.text(Text::LinkJoin),
            h2 { {language.text(Text::LinkJoin)} }
            if let Some(link) = u.vault.joining_link() {
                p { "{link.ticket.body.space_name}" }
                if link.joined {
                    p { role: "status", {language.text(Text::SharingConnectedHint)} }
                } else if let Some(reason) = link.closed {
                    p { role: "status", {language.text(closed_text(reason))} }
                    button { class: "primary", disabled: blocked, onclick: move |_| { if let Some(session) = session { act(Command::Dismiss(session)); } }, {language.text(Text::LinkTryAnother)} }
                } else {
                    if !link.proof_verified {
                        p { role: "status", {language.text(Text::LinkChecking)} }
                    } else if let Some(offer) = &link.offer {
                        if link.consented {
                            p { role: "status", {language.text(Text::LinkConnecting)} }
                        } else {
                            p { {language.text(Text::LinkAccessOffered)} }
                            p { {language.text(role_text(offer.role()))} }
                            button { class: "primary", disabled: blocked, onclick: move |_| { if let Some(session) = session { act(Command::Consent(session)); } }, {language.text(Text::LinkConsent)} }
                        }
                    } else {
                        p { role: "status", {language.text(Text::LinkWaitingApproval)} }
                    }
                    DeviceVerification {}
                    button { class: "quiet", disabled: blocked, onclick: move |_| act(Command::Retry), {language.text(Text::LinkRetry)} }
                    if !link.consented {
                        button { class: "quiet", disabled: blocked, onclick: move |_| { if let Some(session) = session { act(Command::Dismiss(session)); } }, {language.text(Text::LinkWithdraw)} }
                    }
                }
            } else {
                p { class: "muted", {language.text(Text::LinkJoinHint)} }
                label { r#for: "pairing-link", {language.text(Text::LinkPaste)} }
                textarea { id: "pairing-link", rows: 3, maxlength: crate::sync::protocol::pairing::MAX_LINK_BYTES as i64,
                    autocomplete: "off", spellcheck: "false", value: u.sharing.links.link.as_str(), disabled: blocked,
                    oninput: move |event| edit_unlocked(state, |u| u.sharing.links.link = event.value()),
                }
                label { r#for: "pairing-code", {language.text(Text::SharingVerificationCode)} }
                input { id: "pairing-code", maxlength: 64, autocomplete: "off", spellcheck: "false", value: u.sharing.links.code.as_str(), disabled: blocked,
                    oninput: move |event| edit_unlocked(state, |u| u.sharing.links.code = event.value()),
                }
                p { class: "muted", {language.text(Text::LinkCodeHint)} }
                button { class: "primary", disabled: blocked || u.sharing.links.link.trim().is_empty() || u.sharing.links.code.trim().is_empty(), onclick: move |_| act(Command::Request), {language.text(Text::LinkSendRequest)} }
            }
        }
    }
}

fn closed_text(reason: ClosedReason) -> Text {
    match reason {
        ClosedReason::Used => Text::LinkUsed,
        ClosedReason::Cancelled => Text::LinkCancelled,
        ClosedReason::Expired => Text::LinkExpired,
        ClosedReason::Rejected => Text::LinkRejected,
    }
}
