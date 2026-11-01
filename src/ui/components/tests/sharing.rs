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
    shared.upsert_entry(entry).unwrap();
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::core::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = shared;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("People and access"));
    assert!(html.contains("Open sharing file"));
    assert!(html.contains("Controls membership"));
    assert!(html.contains("<h3>owner</h3>"));
    assert!(html.contains("Devices and access"));
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
        .split("Sign and save consent")
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
    assert!(html.contains("Open sharing file"));
    assert!(html.contains("Awaiting owner confirmation"));
    assert!(!html.contains("Create and save join request"));

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
    let mut verification_code = String::new();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            verification_code = u.sharing.output.as_ref().unwrap().code.unwrap().to_string();
            u.sharing.saved_output = u.sharing.output.take();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h2>Approve access</h2>"));
    assert!(html.contains("Awaiting consent"));
    assert!(html.contains(&verification_code));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.sharing.output = None;
            u.sharing.review = Some(Artifact::Response(response));
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h2>Approve access</h2>"));
    assert!(html.contains("Approve and save confirmation"));
}

#[test]
fn wizard_follows_file_outputs_through_a_complete_admission() {
    use crate::core::sync::{Admission, AdmissionReceipt};

    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "owner");
    let guest = vault(&directory, "guest");
    let invitation = owner
        .issue_invitation(guest.device_certificate().unwrap(), Role::Reader, None)
        .unwrap();
    let code = invitation.verification_code().unwrap().to_string();
    let mut guest_dom = mounted();
    update(&mut guest_dom, |state| {
        runtime::set_language(crate::core::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.sharing.joining = true;
            u.open_sharing();
            u.sharing_action(Action::Certificate, 0);
        });
    });
    let html = dioxus_ssr::render(&guest_dom);
    assert!(html.contains("<h2>Prepare your device</h2>"));
    assert!(html.contains("Save sharing file"));
    assert!(!html.contains("Invite a participant"));
    assert!(!html.contains("Controls membership"));
    update(&mut guest_dom, |state| {
        edit_unlocked(state, |u| u.sharing.saved_output = u.sharing.output.take());
    });
    let html = dioxus_ssr::render(&guest_dom);
    assert!(html.contains("<h2>Accept the invitation</h2>"));
    assert!(html.contains("Request saved. Send it to the owner and wait for an invitation."));

    let mut response = None;
    update(&mut guest_dom, |state| {
        edit_unlocked(state, |u| {
            u.sharing.review = Some(Artifact::Invitation(invitation));
            u.sharing.code = code;
            u.sharing_action(Action::Accept, 0);
            response = Some(Admission::decode(&u.sharing.output.as_ref().unwrap().bytes).unwrap());
        });
    });
    let html = dioxus_ssr::render(&guest_dom);
    assert!(html.contains("<h2>Accept the invitation</h2>"));
    assert!(html.contains("Save sharing file"));
    update(&mut guest_dom, |state| {
        edit_unlocked(state, |u| u.sharing.saved_output = u.sharing.output.take());
    });
    let html = dioxus_ssr::render(&guest_dom);
    assert!(html.contains("<h2>Finish connecting</h2>"));
    assert!(html.contains("Awaiting owner confirmation"));

    let mut owner_dom = mounted();
    let mut receipt = None;
    update(&mut owner_dom, |state| {
        runtime::set_language(crate::core::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = owner;
            u.open_sharing();
            u.sharing.review = Some(Artifact::Response(response.unwrap()));
            u.sharing_action(Action::Admit, 0);
            receipt =
                Some(AdmissionReceipt::decode(&u.sharing.output.as_ref().unwrap().bytes).unwrap());
        });
    });
    let html = dioxus_ssr::render(&owner_dom);
    assert!(html.contains("<h2>Approve access</h2>"));
    assert!(html.contains("Save sharing file"));

    update(&mut guest_dom, |state| {
        edit_unlocked(state, |u| {
            u.sharing.output = None;
            u.sharing.review = Some(Artifact::Receipt(receipt.unwrap()));
        });
    });
    assert!(dioxus_ssr::render(&guest_dom).contains("<h2>Finish connecting</h2>"));
    update(&mut guest_dom, |state| {
        edit_unlocked(state, |u| u.sharing_action(Action::Join, 0));
    });
    let html = dioxus_ssr::render(&guest_dom);
    assert!(html.contains("This device joined the space"));
    assert!(!html.contains("sharing-navigation"));
    assert!(!html.contains("Open admission confirmation"));
}

#[test]
fn participant_names_are_rendered_and_saved_in_the_encrypted_profile() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "owner");
    let mut guest = vault(&directory, "guest");
    let certificate = guest.device_certificate().unwrap();
    let user = certificate.body.user;
    let offer = owner
        .issue_invitation(certificate, Role::Reader, None)
        .unwrap();
    let response = guest
        .accept_invitation(offer.clone(), offer.descriptor())
        .unwrap();
    owner.admit_invitation(response, 0).unwrap();
    owner
        .test_profile_mut()
        .unwrap()
        .participant_names
        .insert(user, "Alice <Admin>".into());
    owner.save().unwrap();
    let bytes = std::fs::read(directory.path().join("owner.lbx")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("Alice <Admin>"));
    // Serialization keeps names while older profiles without this field remain readable.
    let profile = owner.test_profile().unwrap();
    let mut serialized = serde_json::to_value(profile).unwrap();
    let restored: crate::infra::profile::BootstrapProfile =
        serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(
        restored.participant_names.get(&user).unwrap(),
        "Alice <Admin>"
    );
    serialized
        .as_object_mut()
        .unwrap()
        .remove("participant_names");
    let legacy: crate::infra::profile::BootstrapProfile =
        serde_json::from_value(serialized).unwrap();
    assert!(legacy.participant_names.is_empty());
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::core::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = owner;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h3>Alice &#60;Admin&#62;</h3>"));
    assert!(html.contains("Allow editing"));
    assert!(html.contains("Remove device access"));
    assert!(html.contains("Names are saved on this device only"));
}
