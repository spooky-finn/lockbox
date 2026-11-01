use crate::sync::protocol::machine::{
    history::{ControlHistory, controls},
    tests::*,
};
use std::collections::BTreeMap;

#[test]
fn verified_chain_rejects_invalid_siblings_without_following_unreachable_controls() {
    let (mut state, device) = owner();
    let mut expected = vec![state.genesis];
    for _ in 0..3 {
        let members = Machine::members(&state, &device).unwrap();
        state.intent = Some(Machine::prepare_membership(&state, &device, members).unwrap());
        expected.push(Machine::complete_intent(&mut state, &device).unwrap());
    }
    let mut controls = controls(&state).unwrap();
    let mut invalid = controls[&expected[1]].clone();
    invalid.signature.0[0] ^= 1;
    let invalid_hash = Object::Control(invalid.clone()).hash().unwrap();
    controls.insert(invalid_hash, invalid.clone());

    // An invalid sibling cannot become an ancestor, even if its child is present.
    let mut child = invalid.clone();
    child.body.epoch.previous = Some(invalid_hash);
    let child_hash = Object::Control(child.clone()).hash().unwrap();
    controls.insert(child_hash, child);

    // Disconnected and extra root controls remain outside the pinned chain.
    for previous in [Some(Hash([99; 32])), None] {
        let mut disconnected = invalid.clone();
        disconnected.body.epoch.previous = previous;
        let hash = Object::Control(disconnected.clone()).hash().unwrap();
        controls.insert(hash, disconnected);
    }

    let history = ControlHistory::verify(&controls, &state).unwrap();
    assert_eq!(
        history
            .chain
            .iter()
            .map(|(hash, _)| *hash)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(history.tip().0, *expected.last().unwrap());
    assert_eq!(
        history.rejected,
        BTreeMap::from([(invalid_hash, Error::InvalidSignature)])
    );
    assert!(!history.contains(child_hash));
}

#[test]
fn control_inspection_checks_hashes_and_encoding_of_content_objects_too() {
    let (mut state, device) = owner();
    let change = edit(&mut state, &device, vec![entry(1, "content")]);
    assert_eq!(controls(&state).unwrap().len(), 1);
    assert_eq!(Machine::controller(&state), Ok(device.id()));

    let mut wrong_hash = state.clone();
    let bytes = wrong_hash.objects.remove(&change).unwrap();
    wrong_hash.objects.insert(Hash([99; 32]), bytes);
    assert_eq!(Machine::controller(&wrong_hash), Err(Error::HashMismatch));
    assert_eq!(
        Machine::quarantine_unverified(&mut wrong_hash),
        Err(Error::HashMismatch)
    );

    state.objects.insert(change, Blob(b"{".to_vec()));
    assert_eq!(Machine::controller(&state), Err(Error::InvalidEncoding));
    assert_eq!(
        Machine::quarantine_unverified(&mut state),
        Err(Error::InvalidEncoding)
    );
}
