use crate::sync::protocol::machine::tests::*;

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
fn preparation_encrypts_exact_payload_without_mutating_reviewed_transaction() {
    let (mut state, device) = owner();
    let mut account = entry(1, "fictional prepared password");
    account.notes = "Заметки\n<&>".into();
    account
        .add_attachment("payload.bin".into(), vec![0, 255, 42])
        .unwrap();
    let transaction = Transaction::diff(&Projection::default(), &[account.clone()], &[]).unwrap();
    let reviewed = canonical(&transaction).unwrap();
    let journal = canonical(&state).unwrap();
    let prepared = Machine::prepare_borrowed(&state, &device, &transaction).unwrap();
    assert_eq!(canonical(&transaction).unwrap(), reviewed);
    assert_eq!(canonical(&state).unwrap(), journal);
    let PreparedChange::Change(change) = &prepared else {
        panic!();
    };
    let control = control_at(&state, change.header.membership).unwrap();
    let key = epoch_key(&control.body, &device).unwrap();
    let content_key = crypto::content_key(&key, state.space, control.body.epoch.context, false);
    let plaintext = crypto::decrypt(
        &content_key,
        &nonce(&change.header.nonce).unwrap(),
        &change.ciphertext,
        &canonical(&change.header).unwrap(),
    )
    .unwrap();
    assert_eq!(plaintext.as_slice(), reviewed);
    state.intent = Some(prepared);
    let reserved = canonical(&state).unwrap();
    assert!(matches!(
        Machine::prepare_borrowed(&state, &device, &transaction),
        Err(Error::PendingIntent)
    ));
    assert_eq!(canonical(&state).unwrap(), reserved);
    assert_eq!(canonical(&transaction).unwrap(), reviewed);
    Machine::complete_intent(&mut state, &device).unwrap();
    assert_eq!(
        Machine::replay(&state, &device)
            .unwrap()
            .projection
            .entries(),
        [account]
    );
}

#[test]
fn invalid_intent_completion_preserves_the_exact_original_journal() {
    let (mut state, device) = owner();
    let object = encrypted_change(
        &state,
        state.genesis,
        &device,
        2, // no preceding event
        None,
        vec![],
        &empty_transaction(),
    );
    let Object::Change(change) = object else {
        panic!()
    };
    state.intent = Some(PreparedChange::Change(change.body));
    let before = canonical(&state).unwrap();
    for _ in 0..2 {
        assert_eq!(
            Machine::complete_intent(&mut state, &device),
            Err(Error::InvalidTransaction)
        );
        assert_eq!(canonical(&state).unwrap(), before);
    }
}
