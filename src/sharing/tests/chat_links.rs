use super::*;
use crate::{
    desktop::{
        context::SecurityHandle,
        view_test_support::{mounted, update},
    },
    platform::clipboard::{Guard, tests::FakeBoard},
    settings::{actions, language::Language},
    sharing::views::tests::vault,
    sync::protocol::pairing::Slot,
};

#[test]
fn managed_owner_chooses_access_before_creating_an_invitation() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "owner");
    owner.test_profile_mut().unwrap().pairing.issuer = Some("test-parent".into());
    let mut dom = mounted();
    update(&mut dom, |state| {
        actions::set_language(Language::English);
        edit_unlocked(state, |u| {
            u.vault = owner;
            u.open_sharing();
        });
    });
    let html = dioxus_ssr::render(&dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(
        primary.contains("id=\"link-role\""),
        "missing role selector at issuance"
    );
    assert!(primary.contains("same private chat"));
    update(&mut dom, |state| {
        act(Command::Create(Role::Editor));
        edit_unlocked(state, |u| {
            assert!(u.error.is_none(), "{:?}", u.error);
            let ticket = &u.vault.issued_links().next().unwrap().ticket;
            assert_eq!(ticket.body.chat_role, Some(Role::Editor));
        });
    });
}

#[test]
fn chat_invitation_opens_as_preview_and_only_the_returned_device_can_be_approved() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = vault(&directory, "Owner");
    let guest = vault(&directory, "Invited guest");
    owner.test_profile_mut().unwrap().pairing.issuer = Some("test-parent".into());
    let now = crate::desktop::time::unix_now();
    let id = owner.create_chat_link(now, 1, Role::Editor).unwrap();
    let invitation = owner.issued_links().next().unwrap().public_link().unwrap();
    let proof = owner
        .pairing_plans(now)
        .unwrap()
        .into_iter()
        .flat_map(|plan| plan.outgoing)
        .find(|delivery| delivery.slot == Slot::Bootstrap)
        .unwrap();
    let board = FakeBoard::default();
    let mut guest_dom = mounted();
    update(&mut guest_dom, |state| {
        actions::set_language(Language::English);
        consume_context::<SecurityHandle>().borrow_mut().clipboard =
            Guard::new(Box::new(board.clone()));
        edit_unlocked(state, |u| {
            u.vault = guest;
            u.sharing.joining = true;
            u.open_sharing();
            u.sharing.links.link = invitation.clone();
        });
    });
    let html = dioxus_ssr::render(&guest_dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(primary.contains("Shared fixture"));
    assert!(primary.contains("Can edit"));
    assert!(primary.contains(">Join</button>"));
    assert!(!primary.contains("pairing-code"));
    update(&mut guest_dom, |state| {
        edit_unlocked(state, |u| assert!(u.vault.joining_link().is_none()));
        act(Command::Join(invitation));
        edit_unlocked(state, |u| {
            u.vault
                .receive_pairing(id, Slot::Bootstrap, &proof.bytes.0, now)
                .unwrap();
        });
    });
    let html = dioxus_ssr::render(&guest_dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(primary.contains("Copy confirmation link"));
    assert!(primary.contains("same private chat"));
    assert!(!primary.contains("second channel"));
    assert!(!primary.contains("Confirm and connect"));
    let mut request = None;
    update(&mut guest_dom, |state| {
        assert!(verification::copy(
            state,
            Language::English,
            CopyTarget::Confirmation
        ));
        edit_unlocked(state, |u| {
            request = u
                .vault
                .pairing_plans(now)
                .unwrap()
                .into_iter()
                .flat_map(|plan| plan.outgoing)
                .find(|delivery| matches!(delivery.slot, Slot::Request(_)));
        });
    });
    let confirmation = board.0.borrow().clone().unwrap();
    let mut owner_dom = mounted();
    update(&mut owner_dom, |state| {
        actions::set_language(Language::English);
        edit_unlocked(state, |u| {
            u.vault = owner;
            u.open_sharing();
            u.sharing.links.confirmation = confirmation.clone();
        });
    });
    let html = dioxus_ssr::render(&owner_dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(primary.contains("hasn’t arrived yet"));
    assert!(!primary.contains("Approve this device"));
    update(&mut owner_dom, |state| {
        edit_unlocked(state, |u| {
            let request = request.unwrap();
            u.vault
                .receive_pairing(id, request.slot, &request.bytes.0, now)
                .unwrap();
        });
    });
    let html = dioxus_ssr::render(&owner_dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(primary.contains("Invited guest"));
    assert!(primary.contains("Approve this device"));
    assert!(!primary.contains("type=\"checkbox\""));
    assert!(!primary.contains("Copy verification code"));
    update(&mut owner_dom, |state| {
        edit_unlocked(state, |u| {
            assert!(u.vault.issued_links().next().unwrap().approval.is_none());
            u.sharing.links.confirmation.clear();
        });
    });
    let html = dioxus_ssr::render(&owner_dom);
    let primary = html.split("Connect using files").next().unwrap();
    assert!(!primary.contains("Invited guest"));
    assert!(!primary.contains("Approve this device"));
    update(&mut owner_dom, |state| {
        // A queued click from the previous review cannot approve a discarded draft.
        act(Command::ApproveChat(
            ConfirmationLink::parse(&confirmation).unwrap(),
        ));
        edit_unlocked(state, |u| {
            assert!(u.vault.issued_links().next().unwrap().approval.is_none())
        });
        edit_unlocked(state, |u| {
            u.sharing.links.confirmation = confirmation.clone()
        });
        act(Command::ApproveChat(
            ConfirmationLink::parse(&confirmation).unwrap(),
        ));
        edit_unlocked(state, |u| {
            assert!(u.error.is_none(), "{:?}", u.error);
            assert_eq!(
                u.vault
                    .issued_links()
                    .next()
                    .unwrap()
                    .approval
                    .as_ref()
                    .unwrap()
                    .role,
                Role::Editor
            );
        });
    });
}
