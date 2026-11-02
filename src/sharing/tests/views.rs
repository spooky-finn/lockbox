use crate::desktop::runtime;
use crate::{
    desktop::{
        view_test_support::{mounted, update},
        workspace::Page,
    },
    records::entry::Entry,
    sharing::views::*,
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, Vault,
        witness::MemoryWitness,
    },
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
        runtime::set_language(crate::settings::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = shared;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("People and access"));
    assert!(html.contains("Open recipient device file"));
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
        runtime::set_language(crate::settings::language::Language::English);
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
        runtime::set_language(crate::settings::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<h2>Finish connecting</h2>"));
    assert!(html.contains("Open admission confirmation"));
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
    assert!(html.contains("<h2>Send an invitation</h2>"));
    assert!(html.contains("Invitation sent — continue"));
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
    use crate::sync::protocol::Admission;
    use crate::sync::protocol::AdmissionReceipt;

    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "owner");
    let guest = vault(&directory, "guest");
    let invitation = owner
        .issue_invitation(guest.device_certificate().unwrap(), Role::Reader, None)
        .unwrap();
    let code = invitation.verification_code().unwrap().to_string();
    let mut guest_dom = mounted();
    update(&mut guest_dom, |state| {
        runtime::set_language(crate::settings::language::Language::English);
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
    assert!(html.contains("<h2>Prepare your device</h2>"));
    assert!(html.contains("Request sent — continue"));
    assert!(html.contains("Copy message for administrator"));
    assert!(html.contains("second trusted channel"));

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
    assert!(html.contains("<h2>Accept the invitation</h2>"));
    assert!(html.contains("Consent sent — continue"));
    assert!(html.contains("Awaiting owner confirmation"));

    let mut owner_dom = mounted();
    let mut receipt = None;
    update(&mut owner_dom, |state| {
        runtime::set_language(crate::settings::language::Language::English);
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
    update(&mut guest_dom, |_| {
        act(Action::Join);
        let settings = consume_context::<Signal<crate::settings::state::Settings>>();
        assert_eq!(
            settings
                .read()
                .config
                .space_names
                .get(&directory.path().join("guest.lbx"))
                .map(String::as_str),
            Some("Shared fixture")
        );
    });
    let html = dioxus_ssr::render(&guest_dom);
    assert!(html.contains("This device joined the space"));
    assert!(!html.contains("sharing-navigation"));
    assert!(!html.contains("Open admission confirmation"));
}

#[test]
fn next_step_requires_a_successfully_saved_file_and_an_explicit_handoff() {
    let directory = tempfile::tempdir().unwrap();
    let guest = vault(&directory, "guest");
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::settings::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.sharing.joining = true;
            u.open_sharing();
            let initial = Workflow::new(u, None, 0);
            assert_eq!(initial.step(), Step::PrepareDevice);
            assert!(initial.next_action().is_none());
            u.sharing_action(Action::Certificate, 0);
            assert!(u.error.is_none());
            // A closed or failed save dialog must not let the user skip sending.
            assert!(Workflow::new(u, None, 0).next_action().is_none());
            u.sharing.saved_output = u.sharing.output.take();
            let saved = Workflow::new(u, None, 0);
            assert_eq!(saved.step(), Step::PrepareDevice);
            assert!(!saved.show_step_action());
            let (next, label) = saved.next_action().unwrap();
            assert_eq!(label, Text::SharingRequestSent);
            u.sharing.cancel_output();
            let sent = Workflow::new(u, Some(next), 0);
            assert_eq!(sent.step(), Step::AcceptInvitation);
            assert!(sent.next_action().is_none());
        });
    });
    let html = dioxus_ssr::render(&dom);
    let progress = html
        .split("sharing-steps")
        .nth(1)
        .unwrap()
        .split("</ol>")
        .next()
        .unwrap();
    assert!(
        !progress.contains("<button"),
        "progress indicator must not skip steps"
    );
}

#[test]
fn identity_copy_uses_current_full_ids_and_localized_username_without_secrets() {
    use crate::{
        desktop::context::SecurityHandle,
        platform::clipboard::{Guard, tests::FakeBoard},
        settings::language::Language,
    };

    let directory = tempfile::tempdir().unwrap();
    let first = vault(&directory, "first");
    let second = vault(&directory, "Alice <Admin> {name}");
    let expected = second.device_certificate().unwrap();
    let board = FakeBoard::default();
    let mut dom = mounted();
    update(&mut dom, |state| {
        consume_context::<SecurityHandle>().borrow_mut().clipboard =
            Guard::new(Box::new(board.clone()));
        runtime::set_language(Language::English);
        edit_unlocked(state, |u| {
            u.vault = first;
            u.sharing.joining = true;
            u.open_sharing();
        });
        assert!(verification::copy(
            state,
            Language::English,
            CopyTarget::Identity
        ));
        // An existing copy control must resolve the replacement profile, not capture old IDs.
        edit_unlocked(state, |u| u.vault = second);
        for language in [Language::English, Language::Russian] {
            assert!(verification::copy(state, language, CopyTarget::Identity));
            let text = board.0.borrow().clone().unwrap();
            assert!(text.contains("Username: Alice <Admin> {name}"));
            assert!(text.contains(&format!("User ID: {}", expected.body.user)));
            assert!(text.contains(&format!("Device ID: {}", expected.body.device)));
            for secret in [
                "private-access-marker",
                "private-provider-marker",
                "master",
                "recovery",
            ] {
                assert!(!text.contains(secret));
            }
            let screen = state.read();
            let Screen::Unlocked(u) = &*screen else {
                panic!("expected unlocked profile")
            };
            assert_eq!(text, verification::message(u, language).unwrap());
        }
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Copy message for administrator"));
    assert!(html.contains("Alice &#60;Admin&#62; {name}"));
    update(&mut dom, |state| {
        runtime::lock(None);
        assert!(!verification::copy(
            state,
            Language::English,
            CopyTarget::Identity
        ));
    });
    assert!(board.0.borrow().is_none());
}

#[test]
fn invitation_code_copy_reuses_the_current_export_and_stops_after_cancellation() {
    use crate::{
        desktop::context::SecurityHandle,
        platform::clipboard::{Guard, tests::FakeBoard},
        settings::language::Language,
    };

    let directory = tempfile::tempdir().unwrap();
    let owner = vault(&directory, "owner");
    let guest = vault(&directory, "guest");
    let board = FakeBoard::default();
    let mut dom = mounted();
    update(&mut dom, |state| {
        consume_context::<SecurityHandle>().borrow_mut().clipboard =
            Guard::new(Box::new(board.clone()));
        edit_unlocked(state, |u| {
            u.vault = owner;
            u.open_sharing();
            u.sharing.review = Some(Artifact::Certificate(guest.device_certificate().unwrap()));
            u.sharing.recipient_verified = true;
            u.sharing_action(Action::Issue(Role::Reader), 0);
            assert!(u.error.is_none());
            u.sharing.saved_output = u.sharing.output.take();
        });
        assert!(verification::copy(
            state,
            Language::English,
            CopyTarget::InvitationCode
        ));
        let expected = match &*state.read() {
            Screen::Unlocked(u) => u.sharing.saved_output.as_ref().unwrap().code.unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(
            board.0.borrow().as_deref(),
            Some(expected.to_string().as_str())
        );
        edit_unlocked(state, |u| {
            let id = u.vault.issued_invitations(0)[0].id;
            u.sharing_action(Action::Cancel(id), 0);
        });
        *board.0.borrow_mut() = Some("a newer unrelated clipboard value".into());
        assert!(!verification::copy(
            state,
            Language::English,
            CopyTarget::InvitationCode
        ));
        assert_eq!(
            board.0.borrow().as_deref(),
            Some("a newer unrelated clipboard value")
        );
    });
}

#[test]
fn failed_verification_copy_reports_error_without_claiming_success() {
    use crate::{
        desktop::context::SecurityHandle,
        platform::clipboard::{Board, Guard},
        settings::language::Language,
    };

    struct Unavailable;
    impl Board for Unavailable {
        fn get(&mut self) -> Option<zeroize::Zeroizing<String>> {
            None
        }
        fn set(&mut self, _: &str) -> Result<(), String> {
            Err("clipboard is unavailable".into())
        }
        fn clear(&mut self) {}
    }

    let directory = tempfile::tempdir().unwrap();
    let guest = vault(&directory, "guest");
    let mut dom = mounted();
    update(&mut dom, |state| {
        consume_context::<SecurityHandle>().borrow_mut().clipboard =
            Guard::new(Box::new(Unavailable));
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.open_sharing();
        });
        assert!(!verification::copy(
            state,
            Language::English,
            CopyTarget::Identity
        ));
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!("expected unlocked profile")
        };
        assert_eq!(
            u.error.as_deref(),
            Some("Could not copy: clipboard is unavailable")
        );
    });
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
    let restored: crate::vault::profile::BootstrapProfile =
        serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(
        restored.participant_names.get(&user).unwrap(),
        "Alice <Admin>"
    );
    serialized
        .as_object_mut()
        .unwrap()
        .remove("participant_names");
    let legacy: crate::vault::profile::BootstrapProfile =
        serde_json::from_value(serialized).unwrap();
    assert!(legacy.participant_names.is_empty());
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::settings::language::Language::English);
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

#[test]
fn link_is_primary_and_admin_reviews_the_signed_name_and_exact_ids_before_approval() {
    use crate::sync::protocol::pairing::Ticket;
    use crate::vault::profile::pairing::Slot;
    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "owner");
    let mut guest = vault(&directory, "Alice <Device>");
    let now = crate::desktop::time::unix_now();
    let session = owner.create_link(now, 1).unwrap();
    let ticket: Ticket = owner.issued_links().next().unwrap().ticket.clone();
    guest
        .request_link(
            &ticket.link().unwrap(),
            &ticket.verification_code().unwrap().to_string(),
            now,
        )
        .unwrap();
    let proof = owner
        .pairing_plans(now)
        .unwrap()
        .into_iter()
        .flat_map(|plan| plan.outgoing)
        .find(|delivery| delivery.slot == Slot::Bootstrap)
        .unwrap();
    guest
        .receive_pairing(session, Slot::Bootstrap, &proof.bytes.0, now)
        .unwrap();
    let request = guest
        .pairing_plans(now)
        .unwrap()
        .into_iter()
        .flat_map(|plan| plan.outgoing)
        .find(|delivery| matches!(delivery.slot, Slot::Request(_)))
        .unwrap();
    owner
        .receive_pairing(session, request.slot, &request.bytes.0, now)
        .unwrap();
    let certificate = guest.device_certificate().unwrap();
    let mut dom = mounted();
    update(&mut dom, |state| {
        runtime::set_language(crate::settings::language::Language::English);
        edit_unlocked(state, |u| {
            u.vault = owner;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(primary.contains("Create invitation link"));
    assert!(primary.contains("Copy invitation link"));
    assert!(primary.contains("Alice &#60;Device&#62;"));
    assert!(primary.contains(&certificate.body.user.to_string()));
    assert!(primary.contains(&certificate.body.device.to_string()));
    assert!(!primary.contains("Open recipient device file"));
    let approve = primary
        .split("Approve this device")
        .next()
        .unwrap()
        .rsplit("<button")
        .next()
        .unwrap();
    assert!(approve.contains("disabled"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.sharing.joining = true;
        })
    });
    let html = dioxus_ssr::render(&dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(primary.contains("wait for the administrator to approve"));
    assert!(primary.contains("Copy message for administrator"));
    assert!(!primary.contains(&ticket.verification_code().unwrap().to_string()));
    assert!(!primary.contains("Save sharing file"));
}
