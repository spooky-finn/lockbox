use super::*;
use crate::{
    records::entry::Entry,
    vault::{ProfileStore, witness::MemoryWitness},
};
use secrecy::SecretString;

type Messages = BTreeMap<(String, Hash, Slot), Vec<u8>>;

fn profile(store: &ProfileStore, path: &std::path::Path, username: &str) -> Vault {
    let mut vault = crate::vault::sharing_tests::create(store, path);
    vault.profile.as_mut().unwrap().username = username.into();
    vault.save().unwrap();
    vault
}

fn round(vault: &mut Vault, messages: &mut Messages, now: u64) {
    for plan in vault.pairing_plans(now).unwrap() {
        let session = plan.ticket.body.id;
        let space = plan.ticket.body.space_name;
        let mut published = Vec::new();
        for outgoing in plan.outgoing {
            messages.insert((space.clone(), session, outgoing.slot), outgoing.bytes.0);
            published.push(outgoing.slot);
        }
        vault.acknowledge_pairing(session, &published).unwrap();
        let mut probes = plan.probes;
        if plan.list_requests {
            probes.extend(messages.keys().filter_map(|(name, id, slot)| {
                (name == &space && *id == session && matches!(slot, Slot::Request(_)))
                    .then_some(*slot)
            }));
        }
        for slot in probes {
            if let Some(bytes) = messages.get(&(space.clone(), session, slot)) {
                vault.receive_pairing(session, slot, bytes, now).unwrap();
            }
        }
    }
}

fn pump(owner: &mut Vault, guest: &mut Vault, messages: &mut Messages, now: u64) {
    for _ in 0..6 {
        round(owner, messages, now);
        round(guest, messages, now);
    }
}

fn ticket(owner: &Vault, session: Hash) -> Ticket {
    owner
        .issued_links()
        .find(|link| link.ticket.body.id == session)
        .unwrap()
        .ticket
        .clone()
}

#[test]
fn link_connects_exact_named_device_with_consent_survives_restart_and_is_single_use() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let owner_path = directory.path().join("owner.lbx");
    let guest_path = directory.path().join("guest.lbx");
    let mut owner = profile(&store, &owner_path, "Administrator");
    let mut guest = profile(&store, &guest_path, "Alice <Device>");
    let mut record = Entry::new();
    record.title = "Shared fictional record".into();
    record.password = "fictional-record-password".into();
    owner.upsert_entry(record).unwrap();
    let session = owner.create_link(10, 1).unwrap();
    let ticket = ticket(&owner, session);
    let link = ticket.link().unwrap();
    assert!(link.starts_with("lockbox://invite/"));
    assert!(!link.contains("fictional-secret"));
    assert!(guest.request_link(&link, &"00".repeat(32), 10).is_err());
    assert!(guest.joining_link().is_none());
    guest
        .request_link(&link, &ticket.verification_code().unwrap().to_string(), 10)
        .unwrap();
    let mut messages = Messages::new();
    pump(&mut owner, &mut guest, &mut messages, 10);
    let contact = owner
        .issued_links()
        .next()
        .unwrap()
        .requests
        .values()
        .next()
        .unwrap()
        .clone();
    let request = contact.hash().unwrap();
    assert_eq!(contact.body.username, "Alice <Device>");
    assert_eq!(
        contact.body.certificate.hash().unwrap(),
        guest.device_certificate().unwrap().hash().unwrap()
    );
    assert!(
        owner
            .approve_link_request(session, request, Role::Editor, false, 10)
            .is_err()
    );
    assert!(owner.issued_links().next().unwrap().approval.is_none());
    assert!(
        !String::from_utf8_lossy(
            &messages[&(
                ticket.body.space_name.clone(),
                session,
                Slot::Request(request)
            )]
        )
        .contains("Alice")
    );
    owner
        .approve_link_request(session, request, Role::Editor, true, 10)
        .unwrap();
    drop(owner);
    drop(guest);
    let password = SecretString::from("test master".to_owned());
    let mut owner = store.open(&owner_path, &password).unwrap();
    let mut guest = store.open(&guest_path, &password).unwrap();
    pump(&mut owner, &mut guest, &mut messages, 11);
    assert!(guest.joining_link().unwrap().offer.is_some());
    assert!(!guest.joining_link().unwrap().consented);
    assert!(owner.space_descriptor() != guest.space_descriptor());
    guest.consent_link(session, 12).unwrap();
    pump(&mut owner, &mut guest, &mut messages, 12);
    assert!(guest.joining_link().unwrap().joined);
    assert!(owner.space_descriptor() == guest.space_descriptor());
    assert_eq!(guest.entries()[0].password, "fictional-record-password");
    assert_eq!(
        owner.participant_name(contact.body.certificate.body.user),
        Some("Alice <Device>")
    );
    assert_eq!(
        owner
            .issued_links()
            .next()
            .unwrap()
            .closed
            .as_ref()
            .unwrap()
            .body
            .reason,
        ClosedReason::Used
    );
    let objects = owner.sync_state().unwrap().objects.len();
    owner.retry_pairing().unwrap();
    guest.retry_pairing().unwrap();
    pump(&mut owner, &mut guest, &mut messages, 13);
    assert_eq!(owner.sync_state().unwrap().objects.len(), objects);
    let mut other = profile(&store, &directory.path().join("other.lbx"), "Other");
    other
        .request_link(&link, &ticket.verification_code().unwrap().to_string(), 13)
        .unwrap();
    pump(&mut owner, &mut other, &mut messages, 13);
    assert_eq!(
        other.joining_link().unwrap().closed,
        Some(ClosedReason::Used)
    );
    assert_eq!(owner.space_members().unwrap().len(), 2);
    assert!(
        owner
            .approve_link_request(session, request, Role::Reader, true, 13)
            .is_err()
    );
    assert!(!String::from_utf8_lossy(&std::fs::read(&owner_path).unwrap()).contains("Alice"));
}

#[test]
fn concurrent_requests_cannot_inherit_approval_or_decrypt_another_devices_offer() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = profile(&store, &directory.path().join("owner.lbx"), "Owner");
    let mut alice = profile(&store, &directory.path().join("alice.lbx"), "Alice");
    let mut bob = profile(&store, &directory.path().join("bob.lbx"), "Bob");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = ticket(&owner, session);
    let link = ticket.link().unwrap();
    let code = ticket.verification_code().unwrap().to_string();
    alice.request_link(&link, &code, 0).unwrap();
    bob.request_link(&link, &code, 0).unwrap();
    let mut messages = Messages::new();
    pump(&mut owner, &mut alice, &mut messages, 0);
    pump(&mut owner, &mut bob, &mut messages, 0);
    let a = alice
        .joining_link()
        .unwrap()
        .request
        .as_ref()
        .unwrap()
        .clone();
    let b = bob
        .joining_link()
        .unwrap()
        .request
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(owner.issued_links().next().unwrap().requests.len(), 2);
    let mut tampered = a.clone();
    tampered.body.username = "Bob".into();
    let bad_hash = tampered.hash().unwrap();
    let frame = Envelope::seal(
        &ticket,
        bad_hash,
        Kind::Request,
        &ticket.body.controller.body,
        &tampered,
    )
    .unwrap();
    assert!(
        owner
            .receive_pairing(session, Slot::Request(bad_hash), &frame.bytes().unwrap(), 0)
            .is_err()
    );
    owner
        .approve_link_request(session, a.hash().unwrap(), Role::Reader, true, 0)
        .unwrap();
    assert!(
        owner
            .approve_link_request(session, b.hash().unwrap(), Role::Editor, true, 0)
            .is_err()
    );
    pump(&mut owner, &mut alice, &mut messages, 1);
    let encrypted = &messages[&(
        ticket.body.space_name.clone(),
        session,
        Slot::Offer(a.hash().unwrap()),
    )];
    let frame = Envelope::decode(encrypted).unwrap();
    assert!(
        frame
            .open::<Invitation>(
                &ticket,
                a.hash().unwrap(),
                Kind::Offer,
                &bob.profile.as_ref().unwrap().sync_secret()
            )
            .is_err()
    );
    assert!(
        frame
            .open::<Invitation>(
                &ticket,
                b.hash().unwrap(),
                Kind::Offer,
                &alice.profile.as_ref().unwrap().sync_secret()
            )
            .is_err()
    );
    alice.consent_link(session, 2).unwrap();
    pump(&mut owner, &mut alice, &mut messages, 2);
    let members = owner.space_members().unwrap();
    assert!(
        members
            .iter()
            .any(|(certificate, role)| certificate.hash().unwrap()
                == a.body.certificate.hash().unwrap()
                && *role == Role::Reader)
    );
    assert!(
        !members
            .iter()
            .any(|(certificate, _)| certificate.hash().unwrap()
                == b.body.certificate.hash().unwrap())
    );
}

#[test]
fn cancellation_and_expiry_block_admission_but_reserved_admission_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = profile(&store, &directory.path().join("owner.lbx"), "Owner");
    let mut guest = profile(&store, &directory.path().join("guest.lbx"), "Guest");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = ticket(&owner, session);
    guest
        .request_link(
            &ticket.link().unwrap(),
            &ticket.verification_code().unwrap().to_string(),
            0,
        )
        .unwrap();
    let mut messages = Messages::new();
    pump(&mut owner, &mut guest, &mut messages, 0);
    let request = guest
        .joining_link()
        .unwrap()
        .request
        .as_ref()
        .unwrap()
        .hash()
        .unwrap();
    owner
        .approve_link_request(session, request, Role::Reader, true, 0)
        .unwrap();
    pump(&mut owner, &mut guest, &mut messages, 1);
    guest.consent_link(session, 1).unwrap();
    round(&mut guest, &mut messages, 2);
    round(&mut owner, &mut messages, 2);
    assert_eq!(owner.space_members().unwrap().len(), 2);
    assert!(owner.cancel_link(session).is_err());
    pump(&mut owner, &mut guest, &mut messages, 86401);
    assert!(guest.joining_link().unwrap().joined);
    assert_eq!(
        owner
            .issued_links()
            .next()
            .unwrap()
            .closed
            .as_ref()
            .unwrap()
            .body
            .reason,
        ClosedReason::Used
    );

    let unused = owner.create_link(86401, 1).unwrap();
    owner.advance_pairing(172801).unwrap();
    assert_eq!(
        owner
            .issued_links()
            .find(|link| link.ticket.body.id == unused)
            .unwrap()
            .closed
            .as_ref()
            .unwrap()
            .body
            .reason,
        ClosedReason::Expired
    );
    let cancelled = owner.create_link(172802, 1).unwrap();
    owner.cancel_link(cancelled).unwrap();
    assert_eq!(
        owner
            .issued_links()
            .find(|link| link.ticket.body.id == cancelled)
            .unwrap()
            .closed
            .as_ref()
            .unwrap()
            .body
            .reason,
        ClosedReason::Cancelled
    );
}

#[test]
fn malformed_links_storage_substitution_and_unsigned_metadata_never_create_requests() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = profile(&store, &directory.path().join("owner.lbx"), "Owner");
    let mut guest = profile(&store, &directory.path().join("guest.lbx"), "Guest");
    let session = owner.create_link(0, 1).unwrap();
    let mut signed = ticket(&owner, session);
    assert!(Ticket::parse_link("https://attacker.invalid/invite").is_err());
    assert!(Ticket::parse_link(&format!("lockbox://invite/{}", "a".repeat(20000))).is_err());
    signed.body.space_name = "Different".into();
    assert!(Ticket::parse_link(&signed.link().unwrap()).is_err());
    let ticket = ticket(&owner, session);
    guest.profile.as_mut().unwrap().provider.bucket = "other-bucket".into();
    assert!(
        guest
            .request_link(
                &ticket.link().unwrap(),
                &ticket.verification_code().unwrap().to_string(),
                0
            )
            .is_err()
    );
    assert!(guest.joining_link().is_none());
    guest.profile.as_mut().unwrap().provider.bucket = "test-bucket".into();
    guest
        .request_link(
            &ticket.link().unwrap(),
            &ticket.verification_code().unwrap().to_string(),
            0,
        )
        .unwrap();
    let proof = owner.pairing_plans(0).unwrap()[0].outgoing[0]
        .bytes
        .0
        .clone();
    let mut bad = proof.clone();
    bad.push(b' ');
    assert!(
        guest
            .receive_pairing(session, Slot::Bootstrap, &bad, 0)
            .is_err()
    );
    assert!(guest.joining_link().unwrap().request.is_none());
    guest
        .receive_pairing(session, Slot::Bootstrap, &proof, 0)
        .unwrap();
    assert!(guest.joining_link().unwrap().request.is_some());
    let mut old = serde_json::to_value(guest.profile.as_ref().unwrap()).unwrap();
    old["schema_version"] = 6.into();
    let old: crate::vault::profile::BootstrapProfile = serde_json::from_value(old).unwrap();
    assert!(!old.validate());
}

#[test]
fn failed_link_save_cannot_export_or_publish_until_exact_session_is_committed() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("owner.lbx");
    let mut owner = profile(&store, &path, "Owner");
    let temporary = crate::vault::file::sibling(&path, "tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert!(owner.create_link(0, 1).is_err());
    let session = owner.issued_links().next().unwrap().ticket.body.id;
    assert!(owner.export_link(session, false).is_err());
    assert!(owner.pairing_plans(0).is_err());
    std::fs::remove_dir(&temporary).unwrap();
    let plans = owner.pairing_plans(0).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].ticket.body.id, session);
    assert!(
        owner
            .export_link(session, false)
            .unwrap()
            .starts_with("lockbox://invite/")
    );
}

#[test]
fn restoring_a_pre_cancel_profile_cannot_reactivate_a_link() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("owner.lbx");
    let mut owner = profile(&store, &path, "Owner");
    let session = owner.create_link(0, 1).unwrap();
    let before_cancel = std::fs::read(&path).unwrap();
    owner.cancel_link(session).unwrap();
    drop(owner);
    std::fs::write(&path, before_cancel).unwrap();
    let mut restored = store
        .open(&path, &SecretString::from("test master".to_owned()))
        .unwrap();
    assert!(restored.signing_error().is_some());
    assert!(restored.create_link(0, 1).is_err());
    assert!(restored.pairing_plans(0).is_err());
}

#[test]
fn a_signed_rejection_for_one_request_cannot_be_replayed_as_global_link_closure() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = profile(&store, &directory.path().join("owner.lbx"), "Owner");
    let mut guest = profile(&store, &directory.path().join("guest.lbx"), "Guest");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = ticket(&owner, session);
    guest
        .request_link(
            &ticket.link().unwrap(),
            &ticket.verification_code().unwrap().to_string(),
            0,
        )
        .unwrap();
    let rejection = Closure::create(
        &ticket,
        ClosedReason::Rejected,
        Some(Hash([17; 32])),
        &owner.profile.as_ref().unwrap().sync_secret(),
    )
    .unwrap();
    assert!(
        guest
            .receive_pairing(session, Slot::Status, &canonical(&rejection).unwrap(), 0)
            .is_err()
    );
    assert!(guest.joining_link().unwrap().closed.is_none());
}
