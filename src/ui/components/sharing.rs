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
    let mut selection = use_signal(|| None::<(bool, usize)>);
    let clock = use_context::<Clock>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let blocked = u.is_editing()
        || u.sharing.busy()
        || u.vault.signing_error().is_some()
        || u.vault.has_pending_exchange();
    // Imported and re-exported artifacts always open the matching workflow step.
    let artifact_step = u
        .sharing
        .review
        .as_ref()
        .map(|artifact| match artifact {
            Artifact::Certificate(_) => (true, 0),
            Artifact::Invitation(_) => (false, 1),
            Artifact::Response(_) => (true, 2),
            Artifact::Receipt(_) => (false, 2),
        })
        .or_else(|| {
            u.sharing.output.as_ref().map(|output| match output.kind {
                Kind::Certificate => (false, 0),
                Kind::Invitation => (true, 1),
                Kind::Response => (false, 1),
                Kind::Receipt => (true, 2),
            })
        });
    let awaiting_owner = u.vault.accepted_invitations().iter().any(|i| !i.joined);
    let awaiting_consent = u
        .vault
        .issued_invitations(*clock.0.read())
        .iter()
        .any(|i| i.status == InvitationStatus::Pending);
    let connected = u.sharing.notice == Some("This device joined the space");
    let (inviting, step) = artifact_step
        .or(if connected {
            Some((false, 2))
        } else {
            *selection.read()
        })
        .unwrap_or_else(|| {
            if connected || awaiting_owner {
                (false, 2)
            } else if awaiting_consent {
                (true, 2)
            } else {
                (u.is_controller(), 0)
            }
        });
    let steps = if inviting {
        [
            Text::SharingStepDevice,
            Text::SharingStepInvite,
            Text::SharingStepAdmit,
        ]
    } else {
        [
            Text::SharingStepPrepare,
            Text::SharingStepAccept,
            Text::SharingStepConnect,
        ]
    };
    let hint = if inviting {
        [
            Text::SharingHintDevice,
            Text::SharingHintInvite,
            Text::SharingHintAdmit,
        ][step]
    } else {
        [
            Text::SharingHintPrepare,
            Text::SharingHintAccept,
            Text::SharingHintConnect,
        ][step]
    };
    let reviewing = artifact_step.is_some() || u.sharing.busy();
    rsx! {
        article { class: "settings-page sharing-page stack", aria_label: language.text(Text::SharingParticipants),
            h1 { {language.text(Text::SharingParticipants)} }
            p { class: "muted", {language.text(Text::SharingDescription)} }
            if u.is_editing() { p { class: "notice", {language.text(Text::SharingDraft)} } }
            if u.vault.signing_error().is_some() { p { class: "notice", {language.text(Text::SharingReadOnly)} } }
            if u.sharing.busy() { p { role: "status", {language.text(Text::SharingBusy)} } }
            if let Some(notice) = u.sharing.notice { p { class: "notice", role: "status", {language.message(notice)} } }
            div { class: "sharing-paths", role: "group", aria_label: language.text(Text::SharingChoosePath),
                if u.is_controller() {
                    button { class: if inviting { "sharing-path selected" } else { "sharing-path" },
                        aria_pressed: inviting, disabled: reviewing,
                        onclick: move |_| selection.set(Some((true, 0))),
                        strong { {language.text(Text::SharingInvite)} }
                        span { {language.text(Text::SharingInvitePathHint)} }
                    }
                }
                button { class: if !inviting { "sharing-path selected" } else { "sharing-path" },
                    aria_pressed: !inviting, disabled: reviewing,
                    onclick: move |_| selection.set(Some((false, 0))),
                    strong { {language.text(Text::SharingConnect)} }
                    span { {language.text(Text::SharingJoinPathHint)} }
                }
            }
            section { class: "settings-section sharing-wizard stack",
                nav { aria_label: language.text(Text::SharingProgress),
                    ol { class: "sharing-steps",
                        for (index, title) in steps.into_iter().enumerate() {
                            li {
                                button { class: if index == step { "sharing-step current" } else { "sharing-step" },
                                    aria_current: if index == step { "step" } else { "false" },
                                    disabled: reviewing,
                                    onclick: move |_| selection.set(Some((inviting, index))),
                                    span { class: "sharing-step-number", "{index + 1}" }
                                    span { {language.text(title)} }
                                }
                            }
                        }
                    }
                }
                header { class: "sharing-step-heading stack",
                    h2 { {language.text(if connected { Text::SharingConnected } else { steps[step] })} }
                    p { class: "muted", {language.text(if connected { Text::SharingConnectedHint } else { hint })} }
                }
                if !connected && u.sharing.review.is_none() && u.sharing.output.is_none() {
                    div { class: "actions sharing-actions",
                        if inviting {
                            if step == 0 {
                                button { class: "primary", disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Certificate), {language.text(Text::SharingImportDevice)} }
                            } else if step == 1 {
                                p { class: "muted", {language.text(Text::SharingResumeInvite)} }
                            } else {
                                button { class: "primary", disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Response), {language.text(Text::SharingImportResponse)} }
                            }
                        } else if step == 0 {
                            button { class: "primary", disabled: blocked, onclick: move |_| act(Action::Certificate), {language.text(Text::SharingExportDevice)} }
                        } else if step == 1 {
                            button { class: "primary", disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Invitation), {language.text(Text::SharingImportInvitation)} }
                        } else {
                            button { class: "primary", disabled: blocked, onclick: move |_| runtime::import_sharing_file(Kind::Receipt), {language.text(Text::SharingImportConfirmation)} }
                        }
                    }
                }
                Review { blocked }
                if let Some(output) = &u.sharing.output {
                    section { class: "sharing-output stack", aria_label: language.text(Text::SharingOutput),
                        h3 { {language.text(Text::SharingOutput)} }
                        p { {language.text(match output.kind {
                            Kind::Certificate => Text::SharingSendDevice, Kind::Invitation => Text::SharingSendInvitation,
                            Kind::Response => Text::SharingSendResponse, Kind::Receipt => Text::SharingSendConfirmation,
                        })} }
                        if let Some(code) = output.code {
                            p { {language.text(Text::SharingVerificationCode)} }
                            code { class: "sharing-code", "{code}" }
                        }
                        button { class: "primary", disabled: u.sharing.busy(), onclick: move |_| {
                            selection.set(Some((inviting, step)));
                            runtime::export_sharing_file();
                        }, {language.text(Text::SharingSaveFile)} }
                        button { class: "quiet", disabled: u.sharing.busy(), onclick: move |_| {
                            selection.set(Some((inviting, step)));
                            edit_unlocked(state, |u| { u.sharing.output = None; u.sharing.notice = None; });
                        }, {language.text(Text::Cancel)} }
                    }
                }
                if !connected {
                    footer { class: "sharing-navigation",
                        button { class: "quiet", disabled: reviewing || step == 0,
                            onclick: move |_| selection.set(Some((inviting, step.saturating_sub(1)))),
                            {language.text(Text::SharingPreviousStep)} }
                        if step < 2 {
                            button { class: "quiet", disabled: reviewing,
                                onclick: move |_| selection.set(Some((inviting, step + 1))),
                                {language.text(Text::SharingNextStep)} }
                        }
                    }
                }
            }
            details { class: "settings-section sharing-identity",
                summary { {language.text(Text::SharingThisDevice)} }
                div { class: "stack",
                    if let Some(certificate) = u.vault.device_certificate() {
                        p { class: "muted", {language.text(Text::UserFingerprint)} }
                        code { class: "sharing-code", "{certificate.body.user}" }
                        p { class: "muted", {language.text(Text::DeviceFingerprint)} }
                        code { class: "sharing-code", "{certificate.body.device}" }
                    }
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
        section { class: "sharing-review stack", aria_label: language.text(Text::SharingReview),
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
                    p { class: "muted", {language.text(Text::SharingExpiryHint)} }
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
                    input { id: "invitation-code", autocomplete: "off", spellcheck: false, maxlength: 64, value: "{u.sharing.code}",
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
                    p { {language.text(Text::SharingSpace)} }
                    code { class: "sharing-code", "{receipt.descriptor().space}" }
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
