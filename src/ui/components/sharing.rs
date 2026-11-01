//! Participant identities, reviewed artifacts and explicit access changes.

use crate::{
    app::{
        session::Screen,
        sharing::{Action, Artifact, Kind},
    },
    core::sync::Role,
    infra::vault::InvitationStatus,
    ui::{
        context::{Clock, Session, edit_unlocked},
        i18n::{Text, use_language},
        runtime,
    },
};
use dioxus::prelude::*;

fn act(action: Action) {
    let state = consume_context::<Session>();
    let now = *(consume_context::<Clock>().0).peek();
    edit_unlocked(state, |u| u.sharing_action(action, now));
}

#[cfg(test)]
#[path = "tests/sharing.rs"]
mod tests;

pub(super) fn role_text(role: Role) -> Text {
    match role {
        Role::Owner => Text::SharingOwner,
        Role::Editor => Text::SharingEditor,
        Role::Reader => Text::SharingReader,
    }
}

#[component]
pub(super) fn SharingView() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let blocked = u.is_editing()
        || u.sharing.busy()
        || u.vault.signing_error().is_some()
        || u.vault.has_pending_exchange();
    rsx! {
        article { class: "settings-page sharing-page stack", aria_label: language.text(Text::SharingParticipants),
            h1 { {language.text(Text::SharingParticipants)} }
            p { class: "muted", {language.text(Text::SharingDescription)} }
            if u.is_editing() { p { class: "notice", {language.text(Text::SharingDraft)} } }
            if u.vault.signing_error().is_some() { p { class: "notice", {language.text(Text::SharingReadOnly)} } }
            if u.sharing.busy() { p { role: "status", {language.text(Text::SharingBusy)} } }
            if let Some(notice) = u.sharing.notice { p { class: "notice", role: "status", {language.message(notice)} } }
            section { class: "settings-section stack",
                h2 { {language.text(Text::SharingThisDevice)} }
                if let Some(certificate) = u.vault.device_certificate() {
                    p { class: "muted", {language.text(Text::UserFingerprint)} }
                    code { class: "sharing-code", "{certificate.body.user}" }
                    p { class: "muted", {language.text(Text::DeviceFingerprint)} }
                    code { class: "sharing-code", "{certificate.body.device}" }
                }
                button { disabled: u.sharing.busy() || u.is_editing(), onclick: move |_| act(Action::Certificate), {language.text(Text::SharingExportDevice)} }
            }
            section { class: "settings-section stack",
                h2 { {language.text(Text::SharingConnect)} }
                p { class: "muted", {language.text(Text::SharingConnectionSteps)} }
                div { class: "actions sharing-actions",
                    button { disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Invitation), {language.text(Text::SharingImportInvitation)} }
                    button { disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Receipt), {language.text(Text::SharingImportConfirmation)} }
                }
            }
            if u.is_controller() {
                section { class: "settings-section stack",
                    h2 { {language.text(Text::SharingInvite)} }
                    div { class: "actions sharing-actions",
                        button { disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Certificate), {language.text(Text::SharingImportDevice)} }
                        button { disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Response), {language.text(Text::SharingImportResponse)} }
                    }
                }
            }
            Review { blocked }
            if let Some(output) = &u.sharing.output {
                section { class: "settings-section stack", aria_label: language.text(Text::SharingOutput),
                    h2 { {language.text(Text::SharingOutput)} }
                    p { {language.text(match output.kind {
                        Kind::Certificate => Text::SharingSendDevice, Kind::Invitation => Text::SharingSendInvitation,
                        Kind::Response => Text::SharingSendResponse, Kind::Receipt => Text::SharingSendConfirmation,
                    })} }
                    if let Some(code) = output.code {
                        p { {language.text(Text::SharingVerificationCode)} }
                        code { class: "sharing-code", "{code}" }
                    }
                    button { disabled: u.sharing.busy(), onclick: move |_| runtime::export_sharing_file(), {language.text(Text::SharingSaveFile)} }
                }
            }
            Members { blocked }
            Invitations { blocked }
        }
    }
}

#[component]
fn Review(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let Some(review) = &u.sharing.review else {
        return rsx! {};
    };
    rsx! {
        section { class: "settings-section stack", aria_label: language.text(Text::SharingReview),
            h2 { {language.text(Text::SharingReview)} }
            match review {
                Artifact::Certificate(certificate) => rsx! {
                    p { {language.text(Text::UserFingerprint)} }
                    code { class: "sharing-code", "{certificate.body.user}" }
                    p { {language.text(Text::DeviceFingerprint)} }
                    code { class: "sharing-code", "{certificate.body.device}" }
                    label { class: "sharing-check",
                        input { r#type: "checkbox", checked: u.sharing.recipient_verified,
                            onchange: move |event| edit_unlocked(state, |u| u.sharing.recipient_verified = event.checked()), }
                        {language.text(Text::SharingVerifyRecipient)}
                    }
                    label { r#for: "invitation-expiry", {language.text(Text::SharingExpiry)} }
                    select { id: "invitation-expiry", value: "{u.sharing.expiry_days}",
                        onchange: move |event| edit_unlocked(state, |u| {
                            if let Ok(days @ (1 | 7 | 30)) = event.value().parse() { u.sharing.expiry_days = days; }
                        }),
                        option { value: "1", {language.text(Text::SharingOneDay)} }
                        option { value: "7", {language.text(Text::SharingSevenDays)} }
                        option { value: "30", {language.text(Text::SharingThirtyDays)} }
                    }
                    div { class: "actions sharing-actions",
                        if u.vault.device_certificate().is_some_and(|own| own.body.user == certificate.body.user) {
                            button { disabled: blocked || !u.sharing.recipient_verified, onclick: move |_| act(Action::Issue(Role::Owner)), {language.text(Text::SharingInviteOwnDevice)} }
                        } else {
                            button { disabled: blocked || !u.sharing.recipient_verified, onclick: move |_| act(Action::Issue(Role::Reader)), {language.text(Text::SharingInviteReader)} }
                            button { disabled: blocked || !u.sharing.recipient_verified, onclick: move |_| act(Action::Issue(Role::Editor)), {language.text(Text::SharingInviteEditor)} }
                        }
                    }
                },
                Artifact::Invitation(invitation) => rsx! {
                    p { {language.text(Text::SharingSpace)} }
                    code { class: "sharing-code", "{invitation.descriptor().space}" }
                    p { {language.text(role_text(invitation.role()))} }
                    p { {language.text(Text::SharingCodeHint)} }
                    label { r#for: "invitation-code", {language.text(Text::SharingVerificationCode)} }
                    input { id: "invitation-code", autocomplete: "off", spellcheck: false, value: "{u.sharing.code}",
                        oninput: move |event| edit_unlocked(state, |u| u.sharing.code = event.value()), }
                    button { disabled: blocked, onclick: move |_| act(Action::Accept), {language.text(Text::SharingAccept)} }
                },
                Artifact::Response(response) => rsx! {
                    p { {language.text(Text::SharingResponseReview)} }
                    code { class: "sharing-code", "{response.recipient().body.user}" }
                    code { class: "sharing-code", "{response.recipient().body.device}" }
                    p { {language.text(role_text(response.role()))} }
                    button { disabled: blocked, onclick: move |_| act(Action::Admit), {language.text(Text::SharingAdmit)} }
                },
                Artifact::Receipt(receipt) => rsx! {
                    p { {language.text(Text::SharingConfirmationReview)} }
                    code { class: "sharing-code", "{receipt.id()}" }
                    button { disabled: blocked, onclick: move |_| act(Action::Join), {language.text(Text::SharingJoin)} }
                },
            }
            button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| {
                u.sharing.review = None; u.sharing.recipient_verified = false; u.sharing.code.clear();
            }), {language.text(Text::Cancel)} }
        }
    }
}

#[component]
fn Members(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let Ok(members) = u.vault.space_members() else {
        return rsx! {};
    };
    let controller = u.vault.space_controller().ok();
    let can_manage = u.is_controller();
    rsx! {
        section { class: "settings-section stack",
            h2 { {language.text(Text::SharingMembers)} }
            p { class: "muted", {language.text(Text::SharingRolesApplyToUser)} }
            for (certificate, role) in members {
                div { class: "sharing-member stack", key: "{certificate.body.device}",
                    code { class: "sharing-code", "{certificate.body.user}" }
                    code { class: "sharing-code muted", "{certificate.body.device}" }
                    p { {language.text(role_text(role))} }
                    if Some(certificate.body.device) == controller { p { class: "muted", {language.text(Text::SharingController)} } }
                    if can_manage {
                        div { class: "actions sharing-actions",
                            if role != Role::Owner {
                                button { disabled: blocked, onclick: { let user = certificate.body.user; move |_| act(Action::Role(user, if role == Role::Reader { Role::Editor } else { Role::Reader })) },
                                    {language.text(if role == Role::Reader { Text::SharingAllowEditing } else { Text::SharingMakeReader })} }
                            }
                            if Some(certificate.body.device) != controller {
                                if u.sharing.confirm_remove == Some(certificate.body.device) {
                                    p { class: "notice", {language.text(Text::SharingRemovalHint)} }
                                    button { class: "danger", disabled: blocked, onclick: { let device = certificate.body.device; move |_| act(Action::Remove(device)) }, {language.text(Text::SharingConfirmRemoval)} }
                                    button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| u.sharing.confirm_remove = None), {language.text(Text::Cancel)} }
                                } else {
                                    button { class: "quiet", disabled: blocked, onclick: { let device = certificate.body.device; move |_| edit_unlocked(state, |u| u.sharing.confirm_remove = Some(device)) }, {language.text(Text::SharingRemoveDevice)} }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Invitations(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let now = *(use_context::<Clock>().0).read();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    rsx! {
        section { class: "settings-section stack",
            h2 { {language.text(Text::SharingInvitations)} }
            for invitation in u.vault.issued_invitations(now) {
                div { class: "sharing-member stack", key: "{invitation.id}",
                    code { class: "sharing-code", "{invitation.id}" }
                    code { class: "sharing-code muted", "{invitation.device}" }
                    p { {language.text(match invitation.status {
                        InvitationStatus::Pending => Text::SharingPending, InvitationStatus::Admitting => Text::SharingAdmitting,
                        InvitationStatus::Admitted => Text::SharingAdmitted, InvitationStatus::Cancelled => Text::SharingCancelled,
                        InvitationStatus::Expired => Text::SharingExpired,
                    })} }
                    div { class: "actions sharing-actions",
                        if invitation.status == InvitationStatus::Pending {
                            button { disabled: blocked, onclick: move |_| act(Action::Reexport(invitation.id)), {language.text(Text::SharingReexport)} }
                            button { disabled: blocked, onclick: move |_| act(Action::Cancel(invitation.id)), {language.text(Text::SharingCancelInvitation)} }
                        }
                        if matches!(invitation.status, InvitationStatus::Admitting | InvitationStatus::Admitted) {
                            button { disabled: blocked, onclick: move |_| act(Action::Refresh(invitation.id)), {language.text(Text::SharingRefresh)} }
                        }
                        if matches!(invitation.status, InvitationStatus::Admitted | InvitationStatus::Cancelled | InvitationStatus::Expired) {
                            button { class: "quiet", disabled: blocked, onclick: move |_| act(Action::Forget(invitation.id)), {language.text(Text::SharingForget)} }
                        }
                    }
                }
            }
            for invitation in u.vault.accepted_invitations().into_iter().filter(|invitation| !invitation.joined) {
                div { class: "sharing-member stack", key: "accepted-{invitation.id}",
                    code { class: "sharing-code", "{invitation.id}" }
                    p { {language.text(Text::SharingAwaitingOwner)} }
                    button { disabled: blocked, onclick: move |_| act(Action::Response(invitation.id)), {language.text(Text::SharingReexportResponse)} }
                }
            }
        }
    }
}
