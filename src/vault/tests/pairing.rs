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
    if let Some(pending) = vault.resolving_link() {
        let hash = pending.reference.ticket;
        let slot = Slot::Ticket(hash);
        if let Some(bytes) = messages.get(&(pending.reference.space_name.clone(), hash, slot)) {
            vault.receive_pairing(hash, slot, bytes, now).unwrap();
        }
    }
    for plan in vault.pairing_plans(now).unwrap() {
        let session = plan.ticket.body.id;
        let space = plan.ticket.body.space_name;
        let mut published = Vec::new();
        for outgoing in plan.outgoing {
            messages.insert(
                (space.clone(), outgoing.slot.address(session), outgoing.slot),
                outgoing.bytes.0,
            );
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
    assert!(owner.export_link(session, false).is_err());
    let slots: Vec<_> = plans[0]
        .outgoing
        .iter()
        .map(|delivery| delivery.slot)
        .collect();
    owner.acknowledge_pairing(session, &slots).unwrap();
    assert!(
        owner
            .export_link(session, false)
            .unwrap()
            .starts_with("lockbox://i/")
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

#[test]
fn short_reference_pins_exact_signed_ticket_and_rejects_ambiguous_encoding() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = profile(&store, &directory.path().join("owner.lbx"), "Owner");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = ticket(&owner, session);
    let reference = ticket.reference().unwrap();
    let link = reference.link();
    assert!(link.len() < 100);
    assert!(link.len() * 10 < ticket.link().unwrap().len());
    assert_eq!(
        LinkReference::parse(&format!("  {link}\n")).unwrap(),
        reference
    );
    assert!(
        reference
            .decode_ticket(&canonical(&ticket).unwrap())
            .is_ok()
    );
    for bad in [
        format!("{link}="),
        format!("{link}/extra"),
        format!("{link}#fragment"),
        "lockbox://i//AA".into(),
        "lockbox://i/VGVhbQ/AA".into(),
        format!("lockbox://i/{}/{}", "a".repeat(600), "a".repeat(43)),
    ] {
        assert!(LinkReference::parse(&bad).is_err(), "accepted {bad}");
    }
    let other = owner.create_link(0, 1).unwrap();
    let other = self::ticket(&owner, other);
    assert!(
        reference
            .decode_ticket(&canonical(&other).unwrap())
            .is_err()
    );
    let mut wrong_route = reference.clone();
    wrong_route.space_name = "Elsewhere".into();
    assert!(
        wrong_route
            .decode_ticket(&canonical(&ticket).unwrap())
            .is_err()
    );
    let mut noncanonical = canonical(&ticket).unwrap();
    noncanonical.push(b' ');
    assert!(reference.decode_ticket(&noncanonical).is_err());
    assert!(
        reference
            .decode_ticket(&vec![b' '; Slot::Ticket(reference.ticket).budget() + 1])
            .is_err()
    );
    let mut forged = ticket.clone();
    forged.signature.0[0] ^= 1;
    assert!(
        forged
            .reference()
            .unwrap()
            .decode_ticket(&canonical(&forged).unwrap())
            .is_err()
    );
    let mut unicode_route = reference;
    unicode_route.space_name = "Семья / 100%".into();
    assert_eq!(
        LinkReference::parse(&unicode_route.link()).unwrap(),
        unicode_route
    );
}

#[test]
fn short_resolution_checks_channel_code_storage_and_expiry_before_contact() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = profile(&store, &directory.path().join("owner.lbx"), "Owner");
    let mut guest = profile(&store, &directory.path().join("guest.lbx"), "Guest");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = ticket(&owner, session);
    let reference = ticket.reference().unwrap();
    let hash = reference.ticket;
    let bytes = canonical(&ticket).unwrap();
    let code = ticket.verification_code().unwrap().to_string();
    guest
        .request_link(&reference.link(), &"00".repeat(32), 0)
        .unwrap();
    assert!(guest.pairing_plans(0).unwrap().is_empty());
    assert!(
        guest
            .receive_pairing(hash, Slot::Ticket(hash), &bytes, 0)
            .is_err()
    );
    assert!(guest.joining_link().is_none());
    assert!(guest.profile.as_ref().unwrap().pairing.outgoing.is_empty());
    guest.dismiss_join_link(hash).unwrap();
    guest.profile.as_mut().unwrap().provider.bucket = "elsewhere".into();
    guest.request_link(&reference.link(), &code, 0).unwrap();
    assert!(
        guest
            .receive_pairing(hash, Slot::Ticket(hash), &bytes, 0)
            .is_err()
    );
    assert!(guest.joining_link().is_none());
    guest.dismiss_join_link(hash).unwrap();
    guest.profile.as_mut().unwrap().provider.bucket = "test-bucket".into();
    guest.request_link(&reference.link(), &code, 0).unwrap();
    assert!(
        guest
            .receive_pairing(hash, Slot::Ticket(hash), &bytes, 86400)
            .is_err()
    );
    assert!(guest.joining_link().is_none());
    assert!(guest.profile.as_ref().unwrap().pairing.outgoing.is_empty());
    guest.dismiss_join_link(hash).unwrap();
    assert!(!guest.pairing_waits_to_join());
    assert!(
        guest
            .receive_pairing(hash, Slot::Ticket(hash), &bytes, 0)
            .is_err()
    );
}

#[test]
fn short_link_readiness_and_pending_resolution_survive_restart_and_remain_single_use() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let owner_path = directory.path().join("owner.lbx");
    let guest_path = directory.path().join("guest.lbx");
    let mut owner = profile(&store, &owner_path, "Owner");
    let mut guest = profile(&store, &guest_path, "Alice");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = ticket(&owner, session);
    let hash = ticket.hash().unwrap();
    owner
        .acknowledge_pairing(session, &[Slot::Bootstrap])
        .unwrap();
    assert!(!owner.link_ready(session));
    assert!(owner.export_link(session, false).is_err());
    let mut messages = Messages::new();
    // This helper represents both publication and its durable acknowledgement.
    owner.retry_pairing().unwrap();
    round(&mut owner, &mut messages, 0);
    let link = owner.export_link(session, false).unwrap();
    let code = owner.export_link(session, true).unwrap();
    guest.request_link(&link, &code, 0).unwrap();
    assert!(guest.joining_link().is_none());
    assert!(guest.pairing_waits_to_join());
    drop(owner);
    drop(guest);
    let password = SecretString::from("test master".to_owned());
    let mut owner = store.open(&owner_path, &password).unwrap();
    let mut guest = store.open(&guest_path, &password).unwrap();
    assert_eq!(owner.export_link(session, false).unwrap(), link);
    assert_eq!(guest.resolving_link().unwrap().reference.ticket, hash);
    let mut downgraded = serde_json::to_value(guest.profile.as_ref().unwrap()).unwrap();
    downgraded["schema_version"] = 7.into();
    let downgraded: crate::vault::profile::BootstrapProfile =
        serde_json::from_value(downgraded).unwrap();
    assert!(!downgraded.validate());
    pump(&mut owner, &mut guest, &mut messages, 1);
    let request = guest
        .joining_link()
        .unwrap()
        .request
        .as_ref()
        .unwrap()
        .hash()
        .unwrap();
    assert!(
        owner
            .approve_link_request(session, request, Role::Reader, false, 1)
            .is_err()
    );
    owner
        .approve_link_request(session, request, Role::Reader, true, 1)
        .unwrap();
    pump(&mut owner, &mut guest, &mut messages, 1);
    assert!(!guest.joining_link().unwrap().joined);
    guest.consent_link(session, 1).unwrap();
    pump(&mut owner, &mut guest, &mut messages, 1);
    assert!(guest.joining_link().unwrap().joined);
    let mut other = profile(&store, &directory.path().join("other.lbx"), "Other");
    other.request_link(&link, &code, 2).unwrap();
    pump(&mut owner, &mut other, &mut messages, 2);
    assert_eq!(
        other.joining_link().unwrap().closed,
        Some(ClosedReason::Used)
    );
    assert_eq!(owner.space_members().unwrap().len(), 2);
}

#[test]
fn schema_seven_links_gain_short_delivery_without_changing_the_signed_invitation() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = profile(&store, &directory.path().join("owner.lbx"), "Owner");
    let session = owner.create_link(0, 1).unwrap();
    let ticket = ticket(&owner, session);
    owner
        .profile
        .as_mut()
        .unwrap()
        .pairing
        .outgoing
        .retain(|delivery| !matches!(delivery.slot, Slot::Ticket(_)));
    let mut legacy = serde_json::to_value(owner.profile.as_ref().unwrap()).unwrap();
    legacy["schema_version"] = 7.into();
    owner.profile = Some(serde_json::from_value(legacy).unwrap());
    assert!(owner.profile.as_ref().unwrap().validate());
    owner.save().unwrap();
    assert!(!owner.link_ready(session));
    let mut messages = Messages::new();
    round(&mut owner, &mut messages, 0);
    let reference = LinkReference::parse(&owner.export_link(session, false).unwrap()).unwrap();
    assert_eq!(reference.ticket, ticket.hash().unwrap());
    assert_eq!(
        Ticket::parse_link(&ticket.link().unwrap())
            .unwrap()
            .hash()
            .unwrap(),
        reference.ticket
    );
    assert_eq!(
        serde_json::to_value(owner.profile.as_ref().unwrap()).unwrap()["schema_version"],
        8
    );
}
