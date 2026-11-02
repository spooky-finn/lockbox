//! Public identity messages for independent device verification.

use crate::{
    desktop::{
        context::{SecurityHandle, Session, edit_unlocked},
        i18n::{Text, use_language},
        runtime,
        session::Screen,
        workspace::Unlocked,
    },
    settings::language::Language,
};
use dioxus::prelude::*;

#[derive(Clone, Copy)]
pub(super) enum CopyTarget {
    Identity,
    InvitationCode,
    Link(crate::sync::protocol::Hash),
    LinkCode(crate::sync::protocol::Hash),
}

pub(super) fn message(unlocked: &Unlocked, language: Language) -> Option<String> {
    let profile = unlocked.vault.profile_summary()?;
    let certificate = unlocked.vault.device_certificate()?;
    Some(language.format(
        Text::SharingVerificationMessage,
        &[
            profile.username.to_owned(),
            certificate.body.user.to_string(),
            certificate.body.device.to_string(),
        ],
    ))
}

/// Resolve public values at click time, so profile changes cannot copy stale IDs.
pub(super) fn copy(state: Session, language: Language, target: CopyTarget) -> bool {
    let native = consume_context::<SecurityHandle>();
    let mut copied = false;
    edit_unlocked(state, |unlocked| {
        let text = match target {
            CopyTarget::Identity => message(unlocked, language),
            CopyTarget::InvitationCode => unlocked
                .sharing
                .output
                .as_ref()
                .or(unlocked.sharing.saved_output.as_ref())
                .and_then(|output| output.code)
                .map(|code| code.to_string()),
            CopyTarget::Link(id) | CopyTarget::LinkCode(id) => match unlocked
                .vault
                .export_link(id, matches!(target, CopyTarget::LinkCode(_)))
            {
                Ok(value) => Some(value),
                Err(error) => {
                    unlocked.error = Some(error.to_string());
                    None
                }
            },
        };
        if let Some(text) = text {
            unlocked.error = native
                .borrow_mut()
                .clipboard
                .copy(&text, std::time::Instant::now())
                .err()
                .map(|error| format!("Could not copy: {error}"));
            copied = unlocked.error.is_none();
        }
    });
    runtime::publish_status();
    copied
}

#[component]
pub(super) fn DeviceVerification() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let mut copied = use_signal(|| None);
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let Some(message) = message(unlocked, language) else {
        return rsx! {};
    };
    let device = unlocked.vault.device_certificate().unwrap().body.device;
    let is_copied = copied
        .read()
        .is_some_and(|(id, at): (_, std::time::Instant)| {
            id == device && at.elapsed().as_secs() < 3
        });
    rsx! {
        section { class: "sharing-verification stack", aria_label: language.text(Text::SharingVerifyDevice),
            h3 { {language.text(Text::SharingVerifyDevice)} }
            p { class: "muted", {language.text(Text::SharingSecondChannelHint)} }
            details { class: "sharing-message-preview",
                summary { {language.text(Text::SharingMessagePreview)} }
                pre { class: "sharing-message", "{message}" }
            }
            div { class: "actions sharing-actions",
                button { class: "quiet", onclick: move |_| {
                    if copy(state, language, CopyTarget::Identity) {
                        copied.set(Some((device, std::time::Instant::now())));
                    } else {
                        copied.set(None);
                    }
                }, {language.text(Text::SharingCopyVerification)} }
                if is_copied {
                    span { class: "copy-indicator", role: "status", {language.text(Text::Copied)} }
                }
            }
        }
    }
}
