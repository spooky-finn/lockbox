use crate::sync::protocol::Replay;
use crate::sync::protocol::machine::tests::*;

fn replay_facts(replay: &Replay) -> impl Eq + std::fmt::Debug + '_ {
    let Replay {
        history: _,
        membership,
        epoch,
        projection,
        frontier,
        accepted,
        pending,
        rejected,
        excluded,
        role,
        ready,
        history_verified,
    } = replay;
    (
        membership,
        epoch,
        canonical(projection).unwrap(),
        frontier,
        accepted,
        pending,
        rejected,
        excluded,
        role,
        ready,
        history_verified,
    )
}

fn compare_replays(state: &SyncState, device: &DeviceSecret) -> Result<Replay, Error> {
    let full = Machine::replay(state, device);
    let current = Machine::replay_state(state, device);
    match (full, current) {
        (Ok(full), Ok(current)) => {
            assert!(current.history.is_empty());
            assert_eq!(replay_facts(&full), replay_facts(&current));
            Ok(full)
        }
        (Err(full), Err(current)) => {
            assert_eq!(full, current);
            Err(full)
        }
        (Ok(_), Err(error)) => panic!("state replay failed: {error:?}"),
        (Err(error), Ok(_)) => panic!("full replay failed: {error:?}"),
    }
}

fn compare_selected_history(state: &SyncState, device: &DeviceSecret, records: &[Option<&str>]) {
    let full = Machine::replay(state, device);
    for &record in records {
        let selected = Machine::replay_record_history(state, device, record);
        match (&full, selected) {
            (Ok(full), Ok(selected)) => {
                assert_eq!(replay_facts(full), replay_facts(&selected));
                let expected: Vec<_> = full
                    .history
                    .iter()
                    .filter(|change| Some(change.id.as_str()) == record)
                    .collect();
                assert_eq!(
                    canonical(&selected.history).unwrap(),
                    canonical(&expected).unwrap()
                );
            }
            (Err(full), Err(selected)) => assert_eq!(*full, selected),
            (Ok(_), Err(error)) => panic!("selected history failed: {error:?}"),
            (Err(error), Ok(_)) => panic!("full history failed: {error:?}"),
        }
    }
}

#[test]
fn selected_record_history_matches_full_filter_without_weakening_verification() {
    let (mut state, device) = owner();
    let mut first = entry(1, "first original");
    first
        .add_attachment("first.bin".into(), vec![0, 255, 42])
        .unwrap();
    let mut second = entry(2, "second original");
    second
        .add_attachment("second.bin".into(), vec![3, 2, 1])
        .unwrap();
    let initial = edit(&mut state, &device, vec![first.clone(), second.clone()]);
    edit(&mut state, &device, vec![first.clone()]);
    let (guest, certificate) = identity(2);
    admit(&mut state, &device, &guest, certificate, Role::Editor);
    first.password = "after checkpoint".into();
    edit(&mut state, &device, vec![first.clone(), second.clone()]);
    let mut concurrent = state.clone();
    first.password = "owner conflict".into();
    edit(&mut state, &device, vec![first.clone(), second.clone()]);
    first.password = "guest conflict".into();
    edit(&mut concurrent, &guest, vec![first.clone(), second.clone()]);
    deliver(&concurrent, &mut state);
    let full = Machine::replay(&state, &device).unwrap();
    assert_eq!(full.projection.conflicts().count(), 1);
    assert!(full.history.iter().any(|change| change.action == "Delete"));
    assert!(full.history.iter().any(|change| change.action == "Restore"));
    let records = [
        Some(first.id.as_str()),
        Some(second.id.as_str()),
        None,
        Some("missing"),
    ];
    compare_selected_history(&state, &device, &records);
    compare_selected_history(&state, &guest, &records);
    let mut incomplete = state.clone();
    incomplete.objects.remove(&initial);
    compare_selected_history(&incomplete, &device, &records);
    let mut frozen = state.clone();
    frozen.fault = Some(Error::DeviceFork);
    compare_selected_history(&frozen, &device, &records);
    state.objects.insert(state.genesis, Blob(b"{".to_vec()));
    compare_selected_history(&state, &device, &records);
}

#[test]
fn state_replay_preserves_closed_epochs_conflicts_and_unavailable_history() {
    let (mut state, device) = owner();
    let empty = compare_replays(&state, &device).unwrap();
    assert!(empty.ready && empty.frontier.is_empty());
    let mut missing_anchor = state.clone();
    let required = Hash([99; 32]);
    missing_anchor.required_control = Some(required);
    let waiting = compare_replays(&missing_anchor, &device).unwrap();
    assert!(!waiting.ready);
    assert!(waiting.frontier.is_empty());
    assert_eq!(waiting.pending, BTreeSet::from([required]));
    let mut record = entry(1, "original");
    record
        .add_attachment("key.bin".into(), vec![0, 255, 42])
        .unwrap();
    let first = edit(&mut state, &device, vec![record.clone()]);
    let (guest, certificate) = identity(2);
    admit(&mut state, &device, &guest, certificate, Role::Editor);
    let mut concurrent = state.clone();
    record.password = "owner edit".into();
    edit(&mut state, &device, vec![record.clone()]);
    record.password = "guest edit".into();
    edit(&mut concurrent, &guest, vec![record]);
    deliver(&concurrent, &mut state);

    let full = compare_replays(&state, &device).unwrap();
    assert!(full.ready && full.history_verified);
    assert_eq!(full.projection.conflicts().count(), 1);
    assert_eq!(full.history.len(), 3);
    assert!(full.history.iter().all(|change| {
        change
            .after
            .as_ref()
            .is_some_and(|entry| entry.attachments[0].data == [0, 255, 42])
    }));
    assert!(full.history.iter().skip(1).all(|change| {
        change.before[0].password == "original"
            && change.before[0].attachments[0].data == [0, 255, 42]
    }));

    let joined = compare_replays(&state, &guest).unwrap();
    assert!(joined.ready && !joined.history_verified);
    assert_eq!(joined.role, Some(Role::Editor));
    assert_eq!(joined.history.len(), 2);
    let (outsider, _) = identity(3);
    let outside = compare_replays(&state, &outsider).unwrap();
    assert!(!outside.ready && !outside.history_verified);
    assert!(outside.frontier.is_empty());
    assert_eq!(outside.role, None);

    let mut incomplete = state.clone();
    incomplete.objects.remove(&first);
    let waiting = compare_replays(&incomplete, &device).unwrap();
    assert!(!waiting.ready);
    assert!(waiting.frontier.is_empty());
    assert!(waiting.pending.contains(&first));
}

#[test]
fn state_replay_keeps_failures_and_candidate_diagnostics() {
    let (mut state, device) = owner();
    let mut invalid = encrypted_change(
        &state,
        state.genesis,
        &device,
        1,
        None,
        vec![],
        &empty_transaction(),
    );
    let Object::Change(change) = &mut invalid else {
        unreachable!()
    };
    change.signature.0[0] ^= 1;
    let hash = invalid.hash().unwrap();
    Machine::receive(&mut state, hash, invalid.bytes().unwrap()).unwrap();
    assert_eq!(
        compare_replays(&state, &device).unwrap().rejected[&hash],
        Error::InvalidSignature
    );

    let missing = Hash([99; 32]);
    let child = encrypted_change(
        &state,
        state.genesis,
        &device,
        1,
        None,
        vec![missing],
        &empty_transaction(),
    );
    Machine::receive(&mut state, child.hash().unwrap(), child.bytes().unwrap()).unwrap();
    let waiting = compare_replays(&state, &device).unwrap();
    assert!(!waiting.ready);
    assert!(waiting.frontier.is_empty());
    assert!(waiting.pending.contains(&missing));

    let mut frozen = state.clone();
    frozen.fault = Some(Error::DeviceFork);
    assert!(matches!(
        compare_replays(&frozen, &device),
        Err(Error::DeviceFork)
    ));
    let mut malformed = state.clone();
    malformed.objects.insert(state.genesis, Blob(b"{".to_vec()));
    assert!(matches!(
        compare_replays(&malformed, &device),
        Err(Error::InvalidEncoding)
    ));
    let mut substituted = state.clone();
    let bytes = substituted.objects.remove(&state.genesis).unwrap();
    substituted.objects.insert(Hash([98; 32]), bytes);
    assert!(matches!(
        compare_replays(&substituted, &device),
        Err(Error::HashMismatch)
    ));

    let (mut checkpoint, key) = owner();
    let mut control = control_at(&checkpoint, checkpoint.genesis).unwrap();
    control.body.checkpoint.0[0] ^= 1;
    let object =
        Object::Control(crypto::sign(control.body, &key.signer(), CONTROL_DOMAIN).unwrap());
    checkpoint.genesis = object.hash().unwrap();
    checkpoint.objects.clear();
    checkpoint
        .objects
        .insert(checkpoint.genesis, Blob(object.bytes().unwrap()));
    assert!(matches!(
        compare_replays(&checkpoint, &key),
        Err(Error::AuthenticationFailed)
    ));
}

#[test]
fn concurrent_passwords_converge_in_every_order_and_resolve_explicitly() {
    let (mut alice, alice_key) = owner();
    edit(&mut alice, &alice_key, vec![entry(1, "original")]);
    let (bob_key, bob_certificate) = identity(2);
    admit(
        &mut alice,
        &alice_key,
        &bob_key,
        bob_certificate,
        Role::Editor,
    );
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
    assert!(replay.history.is_empty());
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
    assert_eq!(replay.history.len(), 2);
    assert!(
        replay
            .history
            .iter()
            .all(|event| !event.revision.starts_with(&hash.to_string()))
    );
    assert_eq!(replay.projection.entries()[0].password, "two");
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
    use crate::records::folder::Folder;
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
    admit(&mut state, &alice, &bob, certificate, Role::Editor);
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
    admit(&mut state, &alice, &bob, certificate, Role::Editor);
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
fn invalid_ancestry_is_rejected_instead_of_waiting_forever_in_every_order() {
    let (state, device) = owner();
    let invalid = Transaction {
        mutations: vec![Mutation {
            entity: Value::Entry(entry(1, "fictional")).entity(),
            bases: vec![],
            action: Action::Delete, // no existing revision to delete
            value: None,
        }],
        rebased_from: None,
    };
    let first = encrypted_change(&state, state.genesis, &device, 1, None, vec![], &invalid);
    let first_hash = first.hash().unwrap();
    let second = encrypted_change(
        &state,
        state.genesis,
        &device,
        2,
        Some(first_hash),
        vec![first_hash],
        &empty_transaction(),
    );
    let second_hash = second.hash().unwrap();
    let third = encrypted_change(
        &state,
        state.genesis,
        &device,
        3,
        Some(second_hash),
        vec![second_hash],
        &empty_transaction(),
    );
    let objects = [first, second, third];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut receiver = state.clone();
        for index in order {
            let object = &objects[index];
            Machine::receive(
                &mut receiver,
                object.hash().unwrap(),
                object.bytes().unwrap(),
            )
            .unwrap();
        }
        let replay = Machine::replay(&receiver, &device).unwrap();
        assert!(replay.ready);
        assert!(replay.pending.is_empty());
        assert!(replay.accepted.is_empty());
        assert_eq!(replay.rejected.len(), 3);
        assert!(replay.projection.records.is_empty());
    }
}

#[test]
fn present_parents_from_another_epoch_or_object_kind_are_invalid() {
    let (mut state, device) = owner();
    let old = edit(&mut state, &device, vec![entry(1, "old")]);
    let members = Machine::members(&state, &device).unwrap();
    state.intent = Some(Machine::prepare_membership(&state, &device, members).unwrap());
    let membership = Machine::complete_intent(&mut state, &device).unwrap();
    let mut pending_control = control_at(&state, membership).unwrap();
    pending_control.body.epoch.previous = Some(Hash([99; 32]));
    let pending_control = Object::Control(pending_control);
    let pending = pending_control.hash().unwrap();
    Machine::receive(&mut state, pending, pending_control.bytes().unwrap()).unwrap();
    assert!(state.candidates.contains_key(&pending));

    for parent in [old, state.genesis, membership, pending] {
        let mut receiver = state.clone();
        let object = encrypted_change(
            &state,
            membership,
            &device,
            1,
            None,
            vec![parent],
            &empty_transaction(),
        );
        let hash = object.hash().unwrap();
        Machine::receive(&mut receiver, hash, object.bytes().unwrap()).unwrap();
        let replay = Machine::replay(&receiver, &device).unwrap();
        assert!(replay.ready);
        assert!(replay.pending.is_empty());
        assert_eq!(replay.rejected[&hash], Error::InvalidTransaction);
    }
}

#[test]
fn closing_over_semantically_invalid_history_is_a_checkpoint_fault() {
    let (mut state, device) = owner();
    let invalid = encrypted_change(
        &state,
        state.genesis,
        &device,
        2,
        None,
        vec![],
        &empty_transaction(),
    );
    let hash = invalid.hash().unwrap();
    let members = Machine::members(&state, &device).unwrap();
    let PreparedChange::Control(mut successor) =
        Machine::prepare_membership(&state, &device, members).unwrap()
    else {
        panic!()
    };
    successor.closed_frontier = vec![hash];
    let successor =
        Object::Control(crypto::sign(successor, &device.signer(), CONTROL_DOMAIN).unwrap());
    for object in [invalid, successor] {
        Machine::receive(&mut state, object.hash().unwrap(), object.bytes().unwrap()).unwrap();
    }
    assert!(matches!(
        Machine::replay(&state, &device),
        Err(Error::CheckpointMismatch)
    ));
}

#[test]
fn wide_frontier_merge_preserves_conflicts_sequence_and_retry_bytes() {
    let (mut state, owner) = owner();
    let mut admissions = vec![];
    let mut devices = vec![owner];
    for seed in 2..=66 {
        let (device, certificate) = identity(seed);
        let offer =
            Machine::offer_invitation(&state, &devices[0], certificate.clone(), Role::Editor, None)
                .unwrap();
        admissions.push(
            Machine::accept_invitation(&offer, &state.descriptor(), &device, &certificate).unwrap(),
        );
        devices.push(device);
    }
    state.intent = Some(Machine::prepare_admissions(&state, &devices[0], admissions).unwrap());
    let membership = Machine::complete_intent(&mut state, &devices[0]).unwrap();
    let base = state.clone();
    let mut authors = std::collections::BTreeMap::new();
    for (index, device) in devices.iter().enumerate() {
        let transaction = Transaction::diff(
            &Projection::default(),
            &[entry(1, &format!("fictional revision {index}"))],
            &[],
        )
        .unwrap();
        let object = encrypted_change(&state, membership, device, 1, None, vec![], &transaction);
        let hash = object.hash().unwrap();
        authors.insert(hash, index);
        Machine::receive(&mut state, hash, object.bytes().unwrap()).unwrap();
    }
    let before = Machine::replay(&state, &devices[0]).unwrap();
    assert_eq!(before.frontier.len(), 66);
    assert_eq!(before.projection.records[0].versions.len(), 66);
    let digest = before.projection.digest().unwrap();
    let (previous, index) = authors.last_key_value().unwrap();
    let device = &devices[*index];
    assert!(matches!(
        Machine::prepare(&state, device, empty_transaction()),
        Err(Error::FrontierTooWide)
    ));
    assert!(matches!(
        Machine::prepare_membership(
            &state,
            &devices[0],
            Machine::members(&state, &devices[0]).unwrap()
        ),
        Err(Error::FrontierTooWide)
    ));
    let prepared = Machine::prepare_merge(&state, device).unwrap().unwrap();
    let PreparedChange::Change(change) = &prepared else {
        panic!()
    };
    assert_eq!(change.header.parents.len(), 64);
    assert!(change.header.parents.contains(previous));
    assert_eq!(change.header.sequence, Counter(2));
    state.intent = Some(prepared);
    let mut restarted: SyncState = serde_json::from_slice(&canonical(&state).unwrap()).unwrap();
    let merged = Machine::complete_intent(&mut state, device).unwrap();
    assert_eq!(
        Machine::complete_intent(&mut restarted, device).unwrap(),
        merged
    );
    assert_eq!(state.objects[&merged], restarted.objects[&merged]);
    let after = Machine::replay(&state, device).unwrap();
    assert_eq!(after.frontier.len(), 3);
    assert_eq!(after.projection.digest().unwrap(), digest);
    assert!(Machine::prepare_merge(&state, device).unwrap().is_none());
    let resolution = Transaction::resolve(
        &after.projection,
        Value::Entry(entry(1, "chosen")).entity(),
        Some(Value::Entry(entry(1, "chosen"))),
    );
    commit(&mut state, device, resolution);
    let final_view = Machine::replay(&state, device).unwrap();
    assert_eq!(final_view.projection.conflicts().count(), 0);
    let mut receiver = base;
    deliver(&state, &mut receiver);
    let received = Machine::replay(&receiver, &devices[0]).unwrap();
    assert!(received.ready);
    assert_eq!(
        received.projection.digest().unwrap(),
        final_view.projection.digest().unwrap()
    );
    assert_eq!(received.accepted, final_view.accepted);
}
