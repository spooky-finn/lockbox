//! Participant identities, reviewed artifacts and explicit access changes.

use crate::{
    desktop::{
        context::{Clock, Session, edit_unlocked},
        i18n::{Text, use_language},
        session::Screen,
    },
    sharing::actions::{Action, Artifact, Kind},
    sync::protocol::Role,
    vault::InvitationStatus,
};
use dioxus::prelude::*;

mod flow;

use flow::{Path, Step, Workflow};

fn act(action: Action) {
    let state = consume_context::<Session>();
    let now = *(consume_context::<Clock>().0).peek();
    let mut save = false;
    edit_unlocked(state, |u| {
        u.sharing_action(action, now);
        save = u.error.is_none() && u.sharing.output.is_some();
    });
    if save {
        crate::sharing::dialogs::export_sharing_file();
    }
}

#[cfg(test)]
#[path = "../tests/views.rs"]
mod tests;

pub(crate) fn role_text(role: Role) -> Text {
    match role {
        Role::Owner => Text::SharingOwner,
        Role::Editor => Text::SharingEditor,
        Role::Reader => Text::SharingReader,
    }
}

fn select_step(state: Session, mut selection: Signal<Option<Step>>, step: Step) {
    edit_unlocked(state, |u| u.sharing.cancel_output());
    selection.set(Some(step));
}

#[component]
pub(crate) fn SharingView() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let blocked = u.sharing_blocked();
    rsx! {
        article { class: "settings-page sharing-page stack", aria_label: language.text(Text::SharingParticipants),
            h1 { {language.text(Text::SharingParticipants)} }
            p { class: "muted", {language.text(Text::SharingDescription)} }
            if u.is_editing() { p { class: "notice", {language.text(Text::SharingDraft)} } }
            if u.vault.signing_error().is_some() { p { class: "notice", {language.text(Text::SharingReadOnly)} } }
            if u.sharing.busy() { p { role: "status", {language.text(Text::SharingBusy)} } }
            if let Some(notice) = u.sharing.notice { p { class: "notice", role: "status", {language.message(notice)} } }
            button { class: "primary", disabled: blocked,
                onclick: move |_| crate::sharing::dialogs::import_sharing_file(),
                {language.text(Text::SharingOpenFile)}
            }
            Wizard { blocked }
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
            if !u.sharing.joining {
                Members { blocked }
            }
            Invitations { blocked }
        }
    }
}

#[component]
fn Wizard(blocked: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let clock = use_context::<Clock>();
    let mut selection = use_signal(|| None::<Step>);
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let workflow = Workflow::new(u, *selection.read(), *clock.0.read());
    let step = workflow.step();
    let path = step.path();
    rsx! {
        div { class: "sharing-paths", role: "group", aria_label: language.text(Text::SharingChoosePath),
            if u.is_controller() && !u.sharing.joining {
                button { class: if path == Path::Invite { "sharing-path selected" } else { "sharing-path" },
                    aria_pressed: path == Path::Invite, disabled: workflow.navigation_locked(),
                    onclick: move |_| select_step(state, selection, Path::Invite.first()),
                    strong { {language.text(Text::SharingInvite)} }
                    span { {language.text(Text::SharingInvitePathHint)} }
                }
            }
            button { class: if path == Path::Join { "sharing-path selected" } else { "sharing-path" },
                aria_pressed: path == Path::Join, disabled: workflow.navigation_locked(),
                onclick: move |_| select_step(state, selection, Path::Join.first()),
                strong { {language.text(Text::SharingConnect)} }
                span { {language.text(Text::SharingJoinPathHint)} }
            }
        }
        section { class: "settings-section sharing-wizard stack",
            nav { aria_label: language.text(Text::SharingProgress),
                ol { class: "sharing-steps",
                    for (index, item) in path.steps().into_iter().enumerate() {
                        li {
                            button { class: if item == step { "sharing-step current" } else { "sharing-step" },
                                aria_current: if item == step { "step" } else { "false" },
                                disabled: workflow.navigation_locked(),
                                onclick: move |_| select_step(state, selection, item),
                                span { class: "sharing-step-number", "{index + 1}" }
                                span { {language.text(item.title())} }
                            }
                        }
                    }
                }
            }
            header { class: "sharing-step-heading stack",
                h2 { {language.text(workflow.title())} }
                p { class: "muted", {language.text(workflow.hint())} }
            }
            if workflow.show_step_action() {
                StepAction { step, blocked }
            }
            Review { blocked }
            SharingOutput { oninteract: move |_| selection.set(None) }
            if !workflow.is_complete() {
                footer { class: "sharing-navigation",
                    button { class: "quiet", disabled: !workflow.can_go_back(),
                        onclick: move |_| select_step(state, selection, step.previous().unwrap_or(step)),
                        {language.text(Text::SharingPreviousStep)} }
                    if let Some(next) = step.next() {
                        button { class: "quiet", disabled: workflow.navigation_locked(),
                            onclick: move |_| select_step(state, selection, next),
                            {language.text(Text::SharingNextStep)} }
                    }
                }
            }
        }
    }
}

#[component]
fn StepAction(step: Step, blocked: bool) -> Element {
    let language = use_language();
    if step != Step::PrepareDevice {
        return rsx! {};
    }
    rsx! {
        div { class: "actions sharing-actions",
            button { class: "primary", disabled: blocked,
                onclick: move |_| act(Action::Certificate),
                {language.text(Text::SharingExportDevice)}
            }
        }
    }
}

#[component]
fn SharingOutput(oninteract: EventHandler) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(u) = &*screen else {
        return rsx! {};
    };
    let Some(output) = u
        .sharing
        .output
        .as_ref()
        .or(u.sharing.saved_output.as_ref())
    else {
        return rsx! {};
    };
    rsx! {
        section { class: "sharing-output stack", aria_label: language.text(Text::SharingOutput),
            h3 { {language.text(Text::SharingOutput)} }
            if u.sharing.output.is_none() {
                p { role: "status", {language.text(match output.kind {
                    Kind::Certificate => Text::SharingWaitingInvitation,
                    Kind::Invitation => Text::SharingPending,
                    Kind::Response => Text::SharingAwaitingOwner,
                    Kind::Receipt => Text::SharingSendConfirmation,
                })} }
            }
            p { {language.text(match output.kind {
                Kind::Certificate => Text::SharingSendDevice, Kind::Invitation => Text::SharingSendInvitation,
                Kind::Response => Text::SharingSendResponse, Kind::Receipt => Text::SharingSendConfirmation,
            })} }
            if let Some(code) = output.code {
                p { {language.text(Text::SharingVerificationCode)} }
                code { class: "sharing-code", "{code}" }
            }
            button { class: "primary", disabled: u.sharing.busy(), onclick: move |_| {
                oninteract.call(());
                crate::sharing::dialogs::export_sharing_file();
            }, {language.text(if u.sharing.output.is_some() { Text::SharingSaveFile } else { Text::SharingSaveAgain })} }
            button { class: "quiet", disabled: u.sharing.busy(), onclick: move |_| {
                oninteract.call(());
                edit_unlocked(state, |u| u.sharing.cancel_output());
            }, {language.text(Text::Cancel)} }
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
            button { class: "quiet", onclick: move |_| edit_unlocked(state, |u| u.sharing.cancel_review()), {language.text(Text::Cancel)} }
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
    let mut users = std::collections::BTreeMap::new();
    for (certificate, role) in members {
        users
            .entry(certificate.body.user)
            .or_insert_with(|| (role, Vec::new()))
            .1
            .push(certificate);
    }
    rsx! {
        section { class: "settings-section stack",
            h2 { {language.text(Text::SharingMembers)} }
            p { class: "muted", {language.text(Text::SharingRolesApplyToUser)} }
            for (user, (role, devices)) in users {
                div { class: "sharing-member stack", key: "{user}",
                    div { class: "sharing-member-heading",
                        h3 { {u.vault.participant_name(user).unwrap_or_else(|| language.text(Text::SharingUnnamedUser)).to_owned()} }
                        span { class: "sharing-role", {language.text(role_text(role))} }
                    }
                    if devices.iter().any(|c| Some(c.body.device) == controller) {
                        p { class: "muted", {language.text(Text::SharingController)} }
                    }
                    if can_manage {
                        details {
                            summary { {language.text(Text::SharingEditName)} }
                            label { r#for: "participant-name-{user}", {language.text(Text::SharingLocalName)} }
                            input { id: "participant-name-{user}", maxlength: 256,
                                value: u.vault.participant_alias(user).unwrap_or_default().to_owned(),
                                disabled: blocked,
                                onchange: move |event| edit_unlocked(state, |u| u.set_participant_name(user, &event.value())),
                            }
                            p { class: "muted", {language.text(Text::SharingLocalNameHint)} }
                        }
                        if role != Role::Owner {
                            button { disabled: blocked, onclick: move |_| act(Action::Role(user, if role == Role::Reader { Role::Editor } else { Role::Reader })),
                                {language.text(if role == Role::Reader { Text::SharingAllowEditing } else { Text::SharingMakeReader })} }
                        }
                    }
                    details { class: "sharing-devices",
                        summary { {language.text(Text::SharingDevices)} " ({devices.len()})" }
                        p { class: "muted", {language.text(Text::UserFingerprint)} }
                        code { class: "sharing-code", "{user}" }
                        for certificate in devices {
                            div { class: "sharing-device stack", key: "{certificate.body.device}",
                                p { class: "muted", {language.text(Text::DeviceFingerprint)} }
                                code { class: "sharing-code", "{certificate.body.device}" }
                                if can_manage && Some(certificate.body.device) != controller {
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
                    if let Some(name) = u.vault.participant_name(invitation.user) {
                        h3 { "{name}" }
                    }
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
