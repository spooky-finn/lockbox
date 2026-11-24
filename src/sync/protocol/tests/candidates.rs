use super::*;

#[test]
fn malformed_changes_keep_rejection_precedence_across_verification_paths() {
    let (state, device) = owner();
    let template = encrypted_change(
        &state,
        state.genesis,
        &device,
        1,
        None,
        vec![],
        &empty_transaction(),
    );
    let members = Machine::members(&state, &device).unwrap();
    let PreparedChange::Control(control) =
        Machine::prepare_membership(&state, &device, members).unwrap()
    else {
        panic!("membership prepares a control")
    };
    type Case = (&'static str, fn(&mut Change), Error);
    let cases: [Case; 8] = [
        (
            "space",
            |c| c.header.space = Hash([99; 32]),
            Error::InvalidTransaction,
        ),
        (
            "epoch",
            |c| c.header.epoch = Counter(99),
            Error::InvalidTransaction,
        ),
        (
            "sequence",
            |c| c.header.sequence = Counter(0),
            Error::InvalidTransaction,
        ),
        (
            "parent count",
            |c| {
                c.header.parents = (0..=crate::sync::protocol::MAX_PARENTS)
                    .map(|index| Hash([index as u8; 32]))
                    .collect()
            },
            Error::InvalidTransaction,
        ),
        (
            "duplicate parents",
            |c| c.header.parents = vec![Hash([1; 32]), Hash([1; 32])],
            Error::InvalidTransaction,
        ),
        (
            "parent order",
            |c| c.header.parents = vec![Hash([2; 32]), Hash([1; 32])],
            Error::InvalidTransaction,
        ),
        (
            "nonce",
            |c| c.header.nonce.0.truncate(23),
            Error::InvalidSignature,
        ),
        (
            "ciphertext",
            |c| c.ciphertext.0.clear(),
            Error::InvalidSignature,
        ),
    ];

    for (label, damage, replay_error) in cases {
        let mut object = template.clone();
        let Object::Change(change) = &mut object else {
            unreachable!()
        };
        // Leave the old signature: shared shape checks must still run first,
        // while replay must retain its later nonce/ciphertext validation.
        damage(&mut change.body);
        let hash = object.hash().unwrap();
        let bytes = object.bytes().unwrap();
        let mut candidate = state.clone();
        Machine::receive(&mut candidate, hash, bytes.clone()).unwrap();
        assert_eq!(
            candidate.rejected_candidates[&hash],
            Error::InvalidTransaction,
            "{label}"
        );
        assert!(!candidate.objects.contains_key(&hash), "{label}");

        let mut legacy = state.clone();
        legacy.objects.insert(hash, Blob(bytes));
        let replay = Machine::replay(&legacy, &device).unwrap();
        assert_eq!(replay.rejected[&hash], replay_error, "{label}");
        assert!(replay.accepted.is_empty(), "{label}");

        let mut closing = control.clone();
        closing.closed_frontier = vec![hash];
        let closing =
            Object::Control(crypto::sign(closing, &device.signer(), CONTROL_DOMAIN).unwrap());
        legacy
            .objects
            .insert(closing.hash().unwrap(), Blob(closing.bytes().unwrap()));
        assert_eq!(
            Machine::relayable_closed_history(&legacy),
            Err(Error::InvalidTransaction),
            "{label}"
        );
    }
}

#[test]
fn discarded_invalid_parent_is_not_mistaken_for_missing_history() {
    let (mut state, device) = owner();
    let membership = state.genesis;
    let mut invalid = encrypted_change(
        &state,
        membership,
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
    let invalid_hash = invalid.hash().unwrap();
    Machine::receive(&mut state, invalid_hash, invalid.bytes().unwrap()).unwrap();
    assert!(!state.objects.contains_key(&invalid_hash));
    assert!(state.rejected_candidates.contains_key(&invalid_hash));
    let child = encrypted_change(
        &state,
        membership,
        &device,
        1,
        None,
        vec![invalid_hash],
        &empty_transaction(),
    );
    let child_hash = child.hash().unwrap();
    Machine::receive(&mut state, child_hash, child.bytes().unwrap()).unwrap();
    let replay = Machine::replay(&state, &device).unwrap();
    assert!(replay.ready);
    assert!(replay.pending.is_empty());
    assert_eq!(replay.rejected[&child_hash], Error::InvalidTransaction);
}

#[test]
fn unsigned_flood_is_bounded_and_cannot_block_local_edits_after_restart() {
    let (mut state, device) = owner();
    let template: Object = decode(&state.objects[&state.genesis].0).unwrap();
    for index in 0..crate::sync::protocol::MAX_OBJECTS + 16 {
        let mut object = template.clone();
        let Object::Control(control) = &mut object else {
            unreachable!()
        };
        control.signature = Blob(vec![0; 64]);
        control.signature.0[..8].copy_from_slice(&(index as u64).to_le_bytes());
        Machine::receive(&mut state, object.hash().unwrap(), object.bytes().unwrap()).unwrap();
    }
    assert_eq!(state.objects.len(), 1);
    assert!(state.candidates.len() <= crate::sync::protocol::MAX_CANDIDATES);
    assert!(Machine::replay(&state, &device).unwrap().ready);
    let mut reopened: SyncState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    edit(
        &mut reopened,
        &device,
        vec![entry(1, "legitimate replacement")],
    );
    assert_eq!(
        Machine::replay(&reopened, &device)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "legitimate replacement"
    );
}

#[test]
fn quarantined_changes_keep_exact_bytes_and_promote_when_membership_arrives() {
    let (mut writer, owner) = owner();
    let (guest, certificate) = identity(2);
    admit(&mut writer, &owner, &guest, certificate, Role::Editor);
    let change = edit(&mut writer, &guest, vec![entry(1, "out of order")]);
    let bytes = writer.objects[&change].0.clone();
    let mut receiver = Machine::join(
        writer.space,
        writer.namespace,
        writer.genesis,
        writer.owner_root,
    );
    Machine::receive(&mut receiver, change, bytes.clone()).unwrap();
    assert!(receiver.objects.is_empty());
    assert_eq!(receiver.candidates[&change].0, bytes);
    let mut receiver: SyncState =
        serde_json::from_slice(&serde_json::to_vec(&receiver).unwrap()).unwrap();
    deliver(&writer, &mut receiver);
    assert_eq!(receiver.objects[&change].0, bytes);
    assert!(receiver.candidates.is_empty());
    assert_eq!(
        Machine::replay(&receiver, &guest)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "out of order"
    );
}

#[test]
fn unknown_membership_ciphertexts_cannot_exceed_the_quarantine_byte_budget() {
    let (mut state, device) = owner();
    let change = edit(&mut state, &device, vec![entry(1, "existing")]);
    let template: Object = decode(&state.objects[&change].0).unwrap();
    for index in 0..5 {
        let mut object = template.clone();
        let Object::Change(change) = &mut object else {
            unreachable!()
        };
        change.body.header.membership = Hash([index + 20; 32]);
        change.body.ciphertext = Blob(vec![0; 1536 * 1024]);
        Machine::receive(&mut state, object.hash().unwrap(), object.bytes().unwrap()).unwrap();
        let bytes: usize = state.candidates.values().map(|bytes| bytes.0.len()).sum();
        assert!(bytes <= crate::sync::protocol::MAX_CANDIDATE_BYTES);
    }
    assert!(state.candidates.len() < 5);
    edit(&mut state, &device, vec![entry(1, "still writable")]);
    assert_eq!(
        Machine::replay(&state, &device)
            .unwrap()
            .projection
            .entries()[0]
            .password,
        "still writable"
    );
}

fn remove(state: &mut SyncState, controller: &DeviceSecret, removed: Hash) {
    let members = Machine::members(state, controller)
        .unwrap()
        .into_iter()
        .filter(|(certificate, _)| certificate.body.device != removed)
        .collect();
    state.intent = Some(Machine::prepare_membership(state, controller, members).unwrap());
    Machine::complete_intent(state, controller).unwrap();
}

fn late_changes_by(state: &SyncState, author: Hash, head: Hash) -> (usize, usize) {
    state
        .objects
        .values()
        .filter_map(|bytes| match decode::<Object>(&bytes.0).unwrap() {
            Object::Change(change)
                if change.body.header.author == author && change.body.header.membership != head =>
            {
                Some(bytes.0.len())
            }
            _ => None,
        })
        .fold((0, 0), |(count, size), len| (count + 1, size + len))
}

fn head(state: &SyncState) -> Hash {
    state
        .objects
        .iter()
        .filter_map(|(hash, bytes)| match decode::<Object>(&bytes.0).unwrap() {
            Object::Control(control) => Some((control.body.epoch.number.0, *hash)),
            Object::Change(_) => None,
        })
        .max()
        .unwrap()
        .1
}

/// Attack: a removed editor keeps signing changes for the epoch where they
/// were a member. Valid signatures must not let them fill the durable journal.
#[test]
fn removed_member_cannot_fill_the_journal_with_late_signed_changes() {
    let (mut alice, alice_key) = owner();
    let (bob_key, bob_certificate) = identity(2);
    admit(
        &mut alice,
        &alice_key,
        &bob_key,
        bob_certificate,
        Role::Editor,
    );
    let mut bob = alice.clone();
    remove(&mut alice, &alice_key, bob_key.id());

    for index in 0..100u32 {
        edit(&mut bob, &bob_key, vec![entry(1, &format!("late {index}"))]);
    }
    deliver(&bob, &mut alice);

    let (count, _) = late_changes_by(&alice, bob_key.id(), head(&alice));
    assert!(
        count <= crate::sync::protocol::LATE_EDITS_PER_AUTHOR,
        "removed member stored {count} late changes"
    );
    assert!(
        count >= 1,
        "an honest late edit must still be kept for rebase"
    );
    edit(&mut alice, &alice_key, vec![entry(2, "owner still writes")]);
    let replay = Machine::replay(&alice, &alice_key).unwrap();
    assert!(replay.ready);
    assert!(
        replay
            .projection
            .entries()
            .iter()
            .any(|entry| entry.password == "owner still writes")
    );
}

/// Large late changes are bounded by bytes as well as by count.
#[test]
fn late_changes_are_bounded_by_bytes_per_author() {
    let (mut alice, alice_key) = owner();
    let (bob_key, bob_certificate) = identity(2);
    admit(
        &mut alice,
        &alice_key,
        &bob_key,
        bob_certificate,
        Role::Editor,
    );
    let mut bob = alice.clone();
    remove(&mut alice, &alice_key, bob_key.id());

    for index in 0..8u8 {
        let mut large = entry(1, "late");
        large.notes = char::from(b'a' + index).to_string().repeat(900 * 1024);
        edit(&mut bob, &bob_key, vec![large]);
    }
    deliver(&bob, &mut alice);

    let (_, size) = late_changes_by(&alice, bob_key.id(), head(&alice));
    assert!(
        size <= crate::sync::protocol::LATE_BYTES_PER_AUTHOR,
        "removed member stored {size} late bytes"
    );
}

/// Honest flow: an editor works offline while the owner changes the roster.
/// The editor rebases their own late edits and the owner receives the result.
#[test]
fn offline_editor_rebases_late_edits_after_a_roster_change() {
    let (mut alice, alice_key) = owner();
    let (bob_key, bob_certificate) = identity(2);
    admit(
        &mut alice,
        &alice_key,
        &bob_key,
        bob_certificate,
        Role::Editor,
    );
    let mut bob = alice.clone();

    let mut offline = Vec::new();
    for index in 0..40u32 {
        offline.push(edit(
            &mut bob,
            &bob_key,
            vec![entry(1, &format!("offline {index}"))],
        ));
    }
    let (carol_key, carol_certificate) = identity(3);
    admit(
        &mut alice,
        &alice_key,
        &carol_key,
        carol_certificate,
        Role::Editor,
    );

    deliver(&alice, &mut bob);
    let replay = Machine::replay(&bob, &bob_key).unwrap();
    assert!(replay.ready);
    let last = *offline.last().unwrap();
    assert!(replay.excluded.contains(&last));
    bob.intent = Some(Machine::prepare_rebase(&bob, &bob_key, last).unwrap());
    let rebased = Machine::complete_intent(&mut bob, &bob_key).unwrap();

    deliver(&bob, &mut alice);
    assert!(alice.objects.contains_key(&rebased));
    let replay = Machine::replay(&alice, &alice_key).unwrap();
    assert!(replay.ready);
    assert_eq!(replay.projection.entries()[0].password, "offline 39");
}

/// Late edits over the per-author budget stay quarantined as candidates. They
/// still replay as excluded, and reviewing excluded edits must not crash.
#[test]
fn excluded_review_skips_late_edits_left_in_quarantine() {
    let (mut alice, alice_key) = owner();
    let (bob_key, bob_certificate) = identity(2);
    admit(
        &mut alice,
        &alice_key,
        &bob_key,
        bob_certificate,
        Role::Editor,
    );
    let mut bob = alice.clone();
    remove(&mut alice, &alice_key, bob_key.id());

    for index in 0..40u32 {
        edit(&mut bob, &bob_key, vec![entry(1, &format!("late {index}"))]);
    }
    deliver(&bob, &mut alice);

    let replay = Machine::replay(&alice, &alice_key).unwrap();
    assert!(
        replay
            .excluded
            .iter()
            .any(|hash| alice.candidates.contains_key(hash)),
        "the scenario needs an excluded edit that is still a candidate"
    );
    let edits = Machine::excluded_edits(&alice, &alice_key).unwrap();
    assert!(!edits.is_empty(), "stored late edits stay reviewable");
    assert!(edits.len() <= crate::sync::protocol::LATE_EDITS_PER_AUTHOR);
    assert!(
        edits
            .iter()
            .all(|edit| alice.objects.contains_key(&edit.hash))
    );
}
