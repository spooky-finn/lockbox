use super::*;

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

#[test]
fn a_pinned_minimum_control_prevents_writes_from_an_older_valid_prefix() {
    let (mut owner, device) = owner();
    edit(&mut owner, &device, vec![entry(1, "current value")]);
    let prefix = owner.clone();
    let members = Machine::members(&owner, &device).unwrap();
    owner.intent = Some(Machine::prepare_membership(&owner, &device, members).unwrap());
    let anchor = Machine::complete_intent(&mut owner, &device).unwrap();
    let mut joined = Machine::join_at(owner.descriptor(), anchor);
    deliver(&prefix, &mut joined);
    let replay = Machine::replay(&joined, &device).unwrap();
    assert_eq!(replay.role, Some(Role::Owner));
    assert!(!replay.ready);
    assert!(replay.pending.contains(&anchor));
    assert!(matches!(
        Machine::prepare(&joined, &device, empty_transaction()),
        Err(Error::MissingDependency)
    ));
    deliver(&owner, &mut joined);
    let replay = Machine::replay(&joined, &device).unwrap();
    assert!(replay.ready);
    assert!(replay.pending.is_empty());
    assert_eq!(replay.membership, anchor);
    assert_eq!(
        replay.projection.digest().unwrap(),
        Machine::replay(&owner, &device)
            .unwrap()
            .projection
            .digest()
            .unwrap()
    );
}
