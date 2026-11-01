use super::{Machine, PreparedChange, SyncState};
use super::{
    commands::control_at,
    membership::epoch_key,
    protocol::{Change, ChangeHeader, PROFILE},
};
use crate::core::sync::{
    Blob, Certificate, Counter, DeviceSecret, Error, Hash, Object, Projection, Role, Signed,
    Transaction, canonical,
    crypto::{self, CHANGE_DOMAIN, CONTROL_DOMAIN, nonce},
    decode,
};
use crate::core::{
    entry::Entry,
    sync::{Action, Entity, EntityKind, Mutation, Value},
};
use ed25519_dalek::SigningKey;
use std::collections::BTreeSet;

#[test]
fn change_wire_contract_survives_internal_refactors() {
    let signing = SigningKey::from_bytes(&[11; 32]);
    let body = Change {
        header: ChangeHeader {
            profile: PROFILE.into(),
            space: Hash([0x11; 32]),
            membership: Hash([0x22; 32]),
            epoch: Counter(17),
            author: Hash::digest(b"lockbox.device.v1\0", &signing.verifying_key().to_bytes()),
            sequence: Counter(42),
            previous: Some(Hash([0x33; 32])),
            parents: vec![Hash([0x33; 32]), Hash([0x44; 32])],
            nonce: Blob(vec![0x55; 24]),
        },
        ciphertext: Blob(vec![1, 2, 3, 4, 5]),
    };
    let signed = crypto::sign(body, &signing, CHANGE_DOMAIN).unwrap();
    crypto::verify(&signed, &signing.verifying_key().to_bytes(), CHANGE_DOMAIN).unwrap();
    let object = Object::Change(signed);
    let expected = include_bytes!("fixtures/change-v0.json");
    assert_eq!(object.bytes().unwrap(), expected);
    assert_eq!(
        object.hash().unwrap().to_string(),
        "f2029d61aef0fdfb6ad9b5f008d4aa26bda81b08433ccd5c1409fa120c0673c0"
    );
    let parsed: Object = decode(expected).unwrap();
    assert_eq!(parsed.hash().unwrap(), object.hash().unwrap());
}

fn identity(seed: u8) -> (DeviceSecret, Signed<Certificate>) {
    DeviceSecret::generate(&SigningKey::from_bytes(&[seed; 32])).unwrap()
}

fn owner() -> (SyncState, DeviceSecret) {
    let (device, certificate) = identity(1);
    let state = Machine::create(
        Hash([1; 32]),
        Hash([2; 32]),
        certificate,
        &device,
        &[3; 32],
        Hash([4; 32]),
    )
    .unwrap();
    (state, device)
}

fn entry(id: u8, password: &str) -> Entry {
    let mut entry = Entry::default();
    entry.id = format!("{id:032x}");
    entry.title = format!("Account {id}");
    entry.password = password.into();
    entry
}

fn edit(state: &mut SyncState, device: &DeviceSecret, entries: Vec<Entry>) -> Hash {
    let replay = Machine::replay(state, device).unwrap();
    let transaction =
        Transaction::diff(&replay.projection, &entries, &replay.projection.folders()).unwrap();
    commit(state, device, transaction)
}

fn commit(state: &mut SyncState, device: &DeviceSecret, transaction: Transaction) -> Hash {
    state.intent = Some(Machine::prepare(state, device, transaction).unwrap());
    Machine::complete_intent(state, device).unwrap()
}

fn admit(
    state: &mut SyncState,
    owner: &DeviceSecret,
    certificate: Signed<Certificate>,
    role: Role,
) {
    let mut members = Machine::members(state, owner).unwrap();
    members.push((certificate, role));
    state.intent = Some(Machine::prepare_membership(state, owner, members).unwrap());
    Machine::complete_intent(state, owner).unwrap();
}

fn deliver(from: &SyncState, to: &mut SyncState) {
    for (hash, bytes) in from.objects.iter().rev() {
        Machine::receive(to, *hash, bytes.0.clone()).unwrap();
    }
}

#[test]
fn concurrent_passwords_converge_in_every_order_and_resolve_explicitly() {
    let (mut alice, alice_key) = owner();
    edit(&mut alice, &alice_key, vec![entry(1, "original")]);
    let (bob_key, bob_certificate) = identity(2);
    admit(&mut alice, &alice_key, bob_certificate, Role::Editor);
    let base = alice.clone();
    let mut bob = base.clone();
    let a = edit(&mut alice, &alice_key, vec![entry(1, "alice")]);
    let b = edit(&mut bob, &bob_key, vec![entry(1, "bob")]);
    let mut digests = Vec::new();
    for order in [[a, b], [b, a], [a, b], [b, a]] {
        let mut receiver = base.clone();
        for hash in order {
            let bytes = alice
                .objects
                .get(&hash)
                .or_else(|| bob.objects.get(&hash))
                .unwrap()
                .0
                .clone();
            Machine::receive(&mut receiver, hash, bytes.clone()).unwrap();
            assert!(!Machine::receive(&mut receiver, hash, bytes).unwrap());
        }
        let replay = Machine::replay(&receiver, &alice_key).unwrap();
        assert_eq!(replay.projection.conflicts().count(), 1);
        assert!(replay.projection.entries().is_empty());
        let passwords: BTreeSet<_> = replay.projection.records[0]
            .versions
            .iter()
            .map(|version| match &version.value {
                Some(Value::Entry(entry)) => entry.password.as_str(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(passwords, BTreeSet::from(["alice", "bob"]));
        digests.push(replay.projection.digest().unwrap());
    }
    assert!(digests.windows(2).all(|pair| pair[0] == pair[1]));
    deliver(&bob, &mut alice);
    let replay = Machine::replay(&alice, &alice_key).unwrap();
    let transaction = Transaction::resolve(
        &replay.projection,
        Value::Entry(entry(1, "chosen")).entity(),
        Some(Value::Entry(entry(1, "chosen"))),
    );
    commit(&mut alice, &alice_key, transaction);
    assert_eq!(
        Machine::replay(&alice, &alice_key)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "chosen"
    );
    // An unseen concurrent successor makes the conflict reappear after resolution.
    edit(&mut bob, &bob_key, vec![entry(1, "late bob")]);
    deliver(&bob, &mut alice);
    assert_eq!(
        Machine::replay(&alice, &alice_key)
            .unwrap()
            .projection
            .conflicts()
            .count(),
        1
    );
}

#[test]
fn dependency_wait_and_signature_substitution_never_mutate_projection() {
    let (mut writer, device) = owner();
    let genesis = writer.clone();
    let first = edit(&mut writer, &device, vec![entry(1, "one")]);
    let second = edit(&mut writer, &device, vec![entry(1, "two")]);
    let mut receiver = genesis;
    Machine::receive(&mut receiver, second, writer.objects[&second].0.clone()).unwrap();
    let replay = Machine::replay(&receiver, &device).unwrap();
    assert!(replay.pending.contains(&first));
    assert!(!replay.ready);
    assert!(replay.projection.records.is_empty());
    Machine::receive(&mut receiver, first, writer.objects[&first].0.clone()).unwrap();
    assert_eq!(
        Machine::replay(&receiver, &device)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "two"
    );
    let mut altered: Object = decode(&writer.objects[&second].0).unwrap();
    let Object::Change(signed) = &mut altered else {
        panic!()
    };
    signed.body.header.space = Hash([9; 32]);
    let hash = altered.hash().unwrap();
    Machine::receive(&mut receiver, hash, altered.bytes().unwrap()).unwrap();
    let replay = Machine::replay(&receiver, &device).unwrap();
    assert!(replay.rejected.contains_key(&hash));
    assert_eq!(replay.projection.entries()[0].password, "two");
}

#[test]
fn reader_knowing_the_epoch_key_still_cannot_write() {
    let (mut state, owner_key) = owner();
    let (reader, certificate) = identity(3);
    admit(&mut state, &owner_key, certificate, Role::Reader);
    assert!(matches!(
        Machine::prepare(
            &state,
            &reader,
            Transaction {
                mutations: vec![],
                rebased_from: None
            }
        ),
        Err(Error::InsufficientRole)
    ));
    let replay = Machine::replay(&state, &owner_key).unwrap();
    let PreparedChange::Change(mut forged) = Machine::prepare(
        &state,
        &owner_key,
        Transaction::diff(&replay.projection, &[entry(1, "forged")], &[]).unwrap(),
    )
    .unwrap() else {
        panic!()
    };
    let control = control_at(&state, replay.membership).unwrap();
    let key = epoch_key(&control.body, &reader).unwrap();
    let key = crypto::content_key(&key, state.space, control.body.epoch.context, false);
    let plaintext = crypto::decrypt(
        &key,
        &nonce(&forged.header.nonce).unwrap(),
        &forged.ciphertext,
        &canonical(&forged.header).unwrap(),
    )
    .unwrap();
    forged.header.author = reader.id();
    forged.ciphertext = crypto::encrypt(
        &key,
        &nonce(&forged.header.nonce).unwrap(),
        &plaintext,
        &canonical(&forged.header).unwrap(),
    )
    .unwrap();
    let object = Object::Change(crypto::sign(forged, &reader.signer(), CHANGE_DOMAIN).unwrap());
    let hash = object.hash().unwrap();
    Machine::receive(&mut state, hash, object.bytes().unwrap()).unwrap();
    let replay = Machine::replay(&state, &owner_key).unwrap();
    assert_eq!(replay.rejected[&hash], Error::InsufficientRole);
    assert!(replay.projection.records.is_empty());
}

#[test]
fn revocation_closes_frontier_and_preserves_late_offline_edits() {
    let (mut alice, alice_key) = owner();
    let (bob_key, bob_certificate) = identity(2);
    admit(&mut alice, &alice_key, bob_certificate, Role::Editor);
    let mut bob = alice.clone();
    let offline = edit(&mut bob, &bob_key, vec![entry(1, "offline secret")]);
    let members = Machine::members(&alice, &alice_key)
        .unwrap()
        .into_iter()
        .filter(|(certificate, _)| certificate.body.device != bob_key.id())
        .collect();
    alice.intent = Some(Machine::prepare_membership(&alice, &alice_key, members).unwrap());
    Machine::complete_intent(&mut alice, &alice_key).unwrap();
    edit(&mut alice, &alice_key, vec![entry(2, "new epoch")]);
    deliver(&bob, &mut alice);
    let replay = Machine::replay(&alice, &alice_key).unwrap();
    assert!(replay.excluded.contains(&offline));
    assert_eq!(replay.projection.entries().len(), 1);
    assert_eq!(replay.projection.entries()[0].password, "new epoch");
    deliver(&alice, &mut bob);
    let replay = Machine::replay(&bob, &bob_key).unwrap();
    assert_eq!(replay.role, None);
    assert!(!replay.ready);
    assert!(matches!(
        Machine::prepare(
            &bob,
            &bob_key,
            Transaction {
                mutations: vec![],
                rebased_from: None
            }
        ),
        Err(Error::MissingDependency | Error::UnauthorizedDevice)
    ));
}

#[test]
fn new_member_bootstraps_without_old_content_keys() {
    let (mut state, owner_key) = owner();
    edit(&mut state, &owner_key, vec![entry(1, "historical")]);
    let (bob, certificate) = identity(2);
    admit(&mut state, &owner_key, certificate, Role::Editor);
    let descriptor = state.descriptor();
    let mut joined = Machine::join(
        descriptor.space,
        descriptor.namespace,
        descriptor.genesis,
        descriptor.owner_root,
    );
    deliver(&state, &mut joined);
    let replay = Machine::replay(&joined, &bob).unwrap();
    assert!(replay.ready);
    assert!(!replay.history_verified);
    assert_eq!(replay.projection.entries()[0].password, "historical");
    edit(&mut joined, &bob, vec![entry(1, "new member edit")]);
    deliver(&joined, &mut state);
    assert_eq!(
        Machine::replay(&state, &owner_key)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "new member edit"
    );
}

#[test]
fn signing_intent_survives_restart_without_resigning_another_slot() {
    let (mut state, device) = owner();
    state.intent = Some(
        Machine::prepare(
            &state,
            &device,
            Transaction::diff(&Projection::default(), &[entry(1, "durable")], &[]).unwrap(),
        )
        .unwrap(),
    );
    let persisted = canonical(&state).unwrap();
    let mut restarted: SyncState = decode(&persisted).unwrap();
    let first = Machine::complete_intent(&mut state, &device).unwrap();
    let retry = Machine::complete_intent(&mut restarted, &device).unwrap();
    assert_eq!(first, retry);
    assert_eq!(state.objects[&first], restarted.objects[&retry]);
    assert_eq!(state.outbox, restarted.outbox);
    assert!(restarted.intent.is_none());
}

#[test]
fn device_and_control_forks_stop_replay_in_both_orders() {
    let (base, device) = owner();
    let mut left = base.clone();
    let mut right = base.clone();
    edit(&mut left, &device, vec![entry(1, "left")]);
    edit(&mut right, &device, vec![entry(1, "right")]);
    deliver(&left, &mut right);
    assert!(matches!(
        Machine::replay(&right, &device),
        Err(Error::DeviceFork)
    ));
    let (bob, cert) = identity(2);
    let mut left = base.clone();
    let mut right = base;
    admit(&mut left, &device, cert.clone(), Role::Editor);
    admit(&mut right, &device, cert, Role::Reader);
    deliver(&left, &mut right);
    assert!(matches!(
        Machine::replay(&right, &device),
        Err(Error::ControlFork)
    ));
    assert!(Machine::replay(&left, &bob).unwrap().ready);
}

#[test]
fn incomplete_bases_and_implicit_restore_are_rejected() {
    let (mut state, device) = owner();
    edit(&mut state, &device, vec![entry(1, "initial")]);
    let entity = Value::Entry(entry(1, "changed")).entity();
    let transaction = Transaction {
        rebased_from: None,
        mutations: vec![Mutation {
            entity: entity.clone(),
            bases: vec![],
            action: Action::Put,
            value: Some(Value::Entry(entry(1, "invalid"))),
        }],
    };
    assert!(matches!(
        Machine::prepare(&state, &device, transaction),
        Err(Error::InvalidTransaction)
    ));
    edit(&mut state, &device, vec![]);
    let replay = Machine::replay(&state, &device).unwrap();
    let transaction = Transaction {
        rebased_from: None,
        mutations: vec![Mutation {
            entity: entity.clone(),
            bases: replay.projection.heads(&entity),
            action: Action::Put,
            value: Some(Value::Entry(entry(1, "invalid"))),
        }],
    };
    assert!(matches!(
        Machine::prepare(&state, &device, transaction),
        Err(Error::InvalidTransaction)
    ));
    edit(&mut state, &device, vec![entry(1, "explicit restore")]);
    assert_eq!(
        Machine::replay(&state, &device)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "explicit restore"
    );
}

#[test]
fn concurrent_folder_cycle_has_identical_evidence_in_both_orders() {
    use crate::core::folder::Folder;
    let (mut state, alice) = owner();
    let f = Folder {
        id: format!("{:032x}", 1),
        name: "F".into(),
        parent_id: None,
    };
    let g = Folder {
        id: format!("{:032x}", 2),
        name: "G".into(),
        parent_id: None,
    };
    commit(
        &mut state,
        &alice,
        Transaction::diff(&Projection::default(), &[], &[f.clone(), g.clone()]).unwrap(),
    );
    let (bob, certificate) = identity(2);
    admit(&mut state, &alice, certificate, Role::Editor);
    let mut other = state.clone();
    let base = Machine::replay(&state, &alice).unwrap().projection;
    let mut moved_f = f.clone();
    moved_f.parent_id = Some(g.id.clone());
    let mut moved_g = g.clone();
    moved_g.parent_id = Some(f.id.clone());
    commit(
        &mut state,
        &alice,
        Transaction::diff(&base, &[], &[moved_f, g]).unwrap(),
    );
    commit(
        &mut other,
        &bob,
        Transaction::diff(&base, &[], &[f, moved_g]).unwrap(),
    );
    deliver(&other, &mut state);
    deliver(&state, &mut other);
    let a = Machine::replay(&state, &alice).unwrap().projection;
    let b = Machine::replay(&other, &bob).unwrap().projection;
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    assert_eq!(a.structural_conflicts().len(), 2);
    assert!(a.structural_conflicts().contains(&Entity {
        kind: EntityKind::Folder,
        id: format!("{:032x}", 1)
    }));
}

#[test]
fn wire_rejects_duplicates_unknown_fields_and_noncanonical_primitives() {
    assert!(decode::<Counter>(b"\"01\"").is_err());
    assert!(decode::<Counter>(b"\"18446744073709551616\"").is_err());
    assert!(decode::<Blob>(b"\"YQ==\"").is_err());
    assert!(decode::<Hash>(b"\"ABC\"").is_err());
    let (state, _) = owner();
    let bytes = &state.objects[&state.genesis].0;
    let text = String::from_utf8(bytes.clone()).unwrap();
    let duplicate = text.replacen(
        "\"kind\":\"control\"",
        "\"kind\":\"control\",\"kind\":\"control\"",
        1,
    );
    assert!(decode::<Object>(duplicate.as_bytes()).is_err());
    let unknown = text.replacen('{', "{\"unknown\":0,", 1);
    assert!(decode::<Object>(unknown.as_bytes()).is_err());
    assert!(decode::<Object>(&vec![b' '; super::super::MAX_OBJECT_BYTES + 1]).is_err());
}

#[test]
fn explicit_rebase_preserves_content_but_uses_new_epoch_and_author_signature() {
    let (mut state, alice) = owner();
    let (bob, certificate) = identity(2);
    admit(&mut state, &alice, certificate, Role::Editor);
    let mut offline = state.clone();
    let excluded = edit(&mut offline, &bob, vec![entry(1, "offline")]);
    let members = Machine::members(&state, &alice).unwrap();
    state.intent = Some(Machine::prepare_membership(&state, &alice, members).unwrap());
    Machine::complete_intent(&mut state, &alice).unwrap();
    deliver(&offline, &mut state);
    let edits = Machine::excluded_edits(&state, &bob).unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].author, bob.id());
    // Alice explicitly re-authors Bob's proposed content; Bob is never credited
    // as signer of Alice's new event, and provenance remains advisory metadata.
    state.intent = Some(Machine::prepare_rebase(&state, &alice, excluded).unwrap());
    let rebased = Machine::complete_intent(&mut state, &alice).unwrap();
    let Object::Change(signed) = decode::<Object>(&state.objects[&rebased].0).unwrap() else {
        panic!()
    };
    assert_eq!(signed.body.header.author, alice.id());
    assert_eq!(signed.body.header.epoch.0, 2);
    assert_eq!(
        Machine::replay(&state, &alice)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "offline"
    );
}

#[test]
fn a_closed_epoch_ignores_unincluded_orphans_and_late_signer_forks() {
    let (mut state, alice) = owner();
    let (bob, certificate) = identity(2);
    admit(&mut state, &alice, certificate, Role::Editor);
    let mut left = state.clone();
    let mut right = state.clone();
    edit(&mut left, &bob, vec![entry(1, "left")]);
    edit(&mut right, &bob, vec![entry(1, "right")]);
    let second = edit(&mut right, &bob, vec![entry(1, "orphan child")]);
    let members = Machine::members(&state, &alice)
        .unwrap()
        .into_iter()
        .filter(|(c, _)| c.body.device != bob.id())
        .collect();
    state.intent = Some(Machine::prepare_membership(&state, &alice, members).unwrap());
    Machine::complete_intent(&mut state, &alice).unwrap();
    Machine::receive(&mut state, second, right.objects[&second].0.clone()).unwrap();
    assert!(Machine::replay(&state, &alice).unwrap().ready);
    deliver(&left, &mut state);
    deliver(&right, &mut state);
    assert!(Machine::replay(&state, &alice).unwrap().ready);
    assert_eq!(Machine::replay(&state, &alice).unwrap().excluded.len(), 3);
}

#[test]
fn all_twenty_four_delivery_permutations_preserve_independent_transactions() {
    fn permutations(values: &mut [usize], index: usize, out: &mut Vec<Vec<usize>>) {
        if index == values.len() {
            out.push(values.to_vec());
            return;
        }
        for next in index..values.len() {
            values.swap(index, next);
            permutations(values, index + 1, out);
            values.swap(index, next);
        }
    }
    let (mut state, alice) = owner();
    let (bob, certificate) = identity(2);
    admit(&mut state, &alice, certificate, Role::Editor);
    let base = state.clone();
    let mut other = base.clone();
    let a = edit(&mut state, &alice, vec![entry(1, "one")]);
    let b = edit(&mut state, &alice, vec![entry(1, "one"), entry(2, "two")]);
    let c = edit(&mut other, &bob, vec![entry(3, "three")]);
    let d = edit(&mut other, &bob, vec![entry(3, "three"), entry(4, "four")]);
    let objects = [a, b, c, d].map(|hash| {
        (
            hash,
            state
                .objects
                .get(&hash)
                .or_else(|| other.objects.get(&hash))
                .unwrap()
                .0
                .clone(),
        )
    });
    let mut orders = Vec::new();
    permutations(&mut [0, 1, 2, 3], 0, &mut orders);
    let mut expected = None;
    for order in orders {
        let mut receiver = base.clone();
        for i in order {
            Machine::receive(&mut receiver, objects[i].0, objects[i].1.clone()).unwrap();
            // Prefixes may be incomplete, never half-apply one transaction.
            let replay = Machine::replay(&receiver, &alice).unwrap();
            assert!(replay.projection.conflicts().next().is_none());
        }
        let replay = Machine::replay(&receiver, &alice).unwrap();
        assert_eq!(replay.projection.entries().len(), 4);
        let digest = replay.projection.digest().unwrap();
        assert_eq!(*expected.get_or_insert(digest), digest);
    }
}

#[test]
fn malicious_checkpoint_cannot_replace_independently_replayed_history() {
    let (mut state, alice) = owner();
    edit(&mut state, &alice, vec![entry(1, "preserve")]);
    let members = Machine::members(&state, &alice).unwrap();
    let PreparedChange::Control(mut control) =
        Machine::prepare_membership(&state, &alice, members).unwrap()
    else {
        panic!()
    };
    let key = epoch_key(&control, &alice).unwrap();
    let checkpoint_key = crypto::content_key(&key, state.space, control.epoch.context, true);
    control.checkpoint = crypto::encrypt(
        &checkpoint_key,
        &nonce(&control.checkpoint_nonce).unwrap(),
        &canonical(&Projection::default()).unwrap(),
        &canonical(&control.epoch).unwrap(),
    )
    .unwrap();
    let object = Object::Control(crypto::sign(control, &alice.signer(), CONTROL_DOMAIN).unwrap());
    Machine::receive(&mut state, object.hash().unwrap(), object.bytes().unwrap()).unwrap();
    assert!(matches!(
        Machine::replay(&state, &alice),
        Err(Error::CheckpointMismatch)
    ));
}
