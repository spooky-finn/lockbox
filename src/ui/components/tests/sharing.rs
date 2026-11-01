use super::*;
use crate::{
    app::workspace::Page,
    core::entry::Entry,
    infra::{
        vault::{KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, Vault},
        witness::MemoryWitness,
    },
    ui::components::test_support::{mounted, update},
};
use secrecy::SecretString;
use std::sync::Arc;

fn vault(directory: &tempfile::TempDir, name: &str) -> Vault {
    ProfileStore::new(Arc::new(MemoryWitness::default()))
        .create_shared(
            directory.path().join(format!("{name}.lbx")),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: name.into(),
                space_name: "Shared fixture".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "fixture".into(),
                    access_key: "private-access-marker".into(),
                    secret_key: "private-provider-marker".into(),
                },
            },
        )
        .unwrap()
}

#[test]
fn sharing_page_renders_member_controls_without_record_or_provider_secrets() {
    let directory = tempfile::tempdir().unwrap();
    let mut shared = vault(&directory, "owner");
    let mut entry = Entry::new();
    entry.title = "Private record title".into();
    entry.password = "private-record-marker".into();
    shared.entries.push(entry);
    shared.save_changes().unwrap();
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::core::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = shared;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Participant devices"));
    assert!(html.contains("Open recipient device file"));
    assert!(html.contains("Controls membership"));
    for secret in [
        "private-record-marker",
        "private-provider-marker",
        "private-access-marker",
    ] {
        assert!(!html.contains(secret), "rendered {secret}");
    }
    let panel = html.split("sharing-page").nth(1).unwrap();
    assert!(!panel.contains("Private record title"));
}

#[test]
fn recipient_enters_an_independent_code_and_drafts_remain_hidden_and_intact() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "owner");
    let guest = vault(&directory, "guest");
    let offer = owner
        .issue_invitation(guest.device_certificate().unwrap(), Role::Editor, None)
        .unwrap();
    let code = offer.verification_code().unwrap().to_string();
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::core::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.start_new();
            u.draft.as_mut().unwrap().entry.title = "Preserved draft".into();
            u.draft.as_mut().unwrap().entry.password = "private-draft-marker".into();
            u.open_sharing();
            u.sharing.review = Some(Artifact::Invitation(offer));
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("id=\"invitation-code\""));
    assert!(!html.contains(&code));
    assert!(!html.contains("private-draft-marker"));
    assert!(html.contains("Save or cancel the current draft"));
    let accept = html
        .split("Sign consent")
        .next()
        .unwrap()
        .rsplit("<button")
        .next()
        .unwrap();
    assert!(accept.contains("disabled"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.page = Page::Vault)
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Preserved draft"));
}

#[test]
fn wizard_follows_artifacts_and_resumes_a_recipient_waiting_for_admission() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "owner");
    let mut guest = vault(&directory, "guest");
    let certificate = guest.device_certificate().unwrap();
    let invitation = owner
        .issue_invitation(certificate.clone(), Role::Reader, None)
        .unwrap();
    let response = guest
        .accept_invitation(invitation.clone(), invitation.descriptor())
        .unwrap();
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::core::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h2>Finish connecting</h2>"));
    assert!(html.contains("Open admission confirmation"));
    assert!(html.contains("Awaiting owner confirmation"));
    assert!(!html.contains("Prepare device file"));

    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault = owner;
            u.sharing.review = Some(Artifact::Certificate(certificate));
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h2>Get their device file</h2>"));
    assert!(html.contains("Access already granted does not expire"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.sharing.recipient_verified = true;
            u.sharing_action(Action::Issue(Role::Reader), 0);
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h2>Send an invitation</h2>"));
    assert!(html.contains("Save sharing file"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.sharing.output = None;
            u.sharing.review = Some(Artifact::Response(response));
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h2>Approve access</h2>"));
    assert!(html.contains("Admit device"));
}
