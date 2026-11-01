use super::*;

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
