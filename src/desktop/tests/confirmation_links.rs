use super::*;
use crate::{
    access::auth::{Locked, Opened},
    desktop::links::receive,
    desktop::{
        context::{BiometricProvider, SecurityHandle, Session},
        session::Screen,
        tabs::{self, Tabs},
        view_test_support::mounted_with_header,
        workspace::Page,
    },
    settings::state::Settings,
    sync::protocol::{
        DeviceSecret, Role,
        pairing::{Contact, Envelope, Kind, Slot},
    },
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
    },
};
use secrecy::SecretString;
use std::sync::Arc;

#[test]
fn native_confirmation_waits_for_issuing_vault_unlock_and_preserves_drafts() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = ProfileStore::new(Arc::new(MemoryWitness::default()))
        .create_shared(
            directory.path().join("owner.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Owner".into(),
                space_name: "Team".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "fixture".into(),
                    access_key: "fixture-access".into(),
                    secret_key: "fixture-secret".into(),
                },
            },
        )
        .unwrap();
    owner.test_profile_mut().unwrap().pairing.issuer = Some("test-parent".into());
    let now = crate::desktop::time::unix_now();
    let session = owner.create_chat_link(now, 1, Role::Reader).unwrap();
    let ticket = owner.issued_links().next().unwrap().ticket.clone();
    let (device, certificate) =
        DeviceSecret::generate(&ed25519_dalek::SigningKey::from_bytes(&[8; 32])).unwrap();
    let request = Contact::create(&ticket, "Guest", certificate, &device).unwrap();
    let request_hash = request.hash().unwrap();
    let envelope = Envelope::seal(
        &ticket,
        request_hash,
        Kind::Request,
        &ticket.body.controller.body,
        &request,
    )
    .unwrap();
    owner
        .receive_pairing(
            session,
            Slot::Request(request_hash),
            &envelope.bytes().unwrap(),
            now,
        )
        .unwrap();
    let link = format!(
        "lockbox://confirm/{}/{request_hash}",
        ticket.hash().unwrap()
    );
    let dom = mounted_with_header();
    dom.in_scope(ScopeId::APP, || {
        let mut state = consume_context::<Session>();
        if let Screen::Unlocked(u) = &mut *state.write() {
            u.start_new();
            u.draft.as_mut().unwrap().entry.title = "Unsaved".into();
        }
        for invalid in [format!(" {link}"), format!("{link}?other")] {
            receive(&invalid);
        }
        assert!(CONFIRMATION.borrow().is_none());
        let previous = format!(
            "lockbox://confirm/{}/{}",
            ticket.hash().unwrap(),
            "00".repeat(32)
        );
        receive(&previous);
        let superseded = CONFIRMATION.borrow().expect("first confirmation retained");
        receive(&link);
        assert!(!take_confirmation(superseded));
        let reference = CONFIRMATION.borrow().expect("native return link retained");
        assert_eq!(
            present_confirmation(reference),
            Err(Text::ConfirmationUnlockOwner)
        );
        tabs::new_tab();
        state.set(Screen::Locked(Box::new(Locked::new(Some(
            owner.path().into(),
        )))));
        assert_eq!(
            present_confirmation(reference),
            Err(Text::ConfirmationUnlockOwner)
        );
        state.write().open(
            Opened {
                vault: owner,
                with_password: true,
                recovery_notice: None,
            },
            &mut consume_context::<Signal<Settings>>().write(),
            &*consume_context::<BiometricProvider>().0,
            &mut consume_context::<SecurityHandle>().borrow_mut(),
        );
        if let Screen::Unlocked(u) = &mut *state.write() {
            u.start_new();
        }
        assert_eq!(
            present_confirmation(reference),
            Err(Text::ConfirmationFinishEditing)
        );
        if let Screen::Unlocked(u) = &mut *state.write() {
            assert!(u.draft.take().is_some());
        }
        tabs::activate(0);
        assert_eq!(present_confirmation(reference), Ok(true));
        assert!(CONFIRMATION.borrow().is_none());
        let tabs = consume_context::<Signal<Tabs>>();
        assert_eq!(tabs.peek().active, 1);
        let Screen::Unlocked(u) = &*state.peek() else {
            panic!("owner locked")
        };
        assert!(u.page == Page::Sharing);
        assert_eq!(u.sharing.links.confirmation, link);
        assert!(!u.sharing.joining);
        assert!(u.vault.review_chat_link(&reference, now).is_ok());
        assert!(u.vault.issued_links().next().unwrap().approval.is_none());
        let other = tabs.peek().sessions[0];
        let Screen::Unlocked(other) = &*other.peek() else {
            panic!("other vault closed")
        };
        assert_eq!(other.draft.as_ref().unwrap().entry.title, "Unsaved");
        assert!(other.sharing.links.confirmation.is_empty());
    });
}
