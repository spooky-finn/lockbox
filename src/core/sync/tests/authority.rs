use super::*;
use crate::core::sync::{ControlApproval, ControlTransfer, RecoveryAnchor};

fn current_owner() -> (SyncState, DeviceSecret, SigningKey) {
    let root = SigningKey::from_bytes(&[1; 32]);
    let (device, cert) = DeviceSecret::generate(&root).unwrap();
    let state = Machine::create(
        Hash([1; 32]),
        Hash([2; 32]),
        cert,
        &device,
        &[3; 32],
        Hash([4; 32]),
    )
    .unwrap();
    (state, device, root)
}

fn additional_owner(
    state: &mut SyncState,
    owner: &DeviceSecret,
    root: &SigningKey,
) -> DeviceSecret {
    let (device, cert) = DeviceSecret::generate(root).unwrap();
    let offer = Machine::offer_invitation(state, owner, cert.clone(), Role::Owner, None).unwrap();
    let consent = Machine::accept_invitation(&offer, &state.descriptor(), &device, &cert).unwrap();
    state.intent = Some(Machine::prepare_admission(state, owner, consent).unwrap());
    Machine::complete_intent(state, owner).unwrap();
    device
}

fn recovery(
    source: &SyncState,
    owner: &DeviceSecret,
    root: &SigningKey,
) -> (SyncState, DeviceSecret) {
    let (fresh, certificate) = DeviceSecret::generate(root).unwrap();
    let anchor = Machine::recovery_anchor(source, owner).unwrap();
    let anchor = RecoveryAnchor::decode(&anchor.bytes().unwrap()).unwrap();
    let prepared = Machine::prepare_control_recovery(source, owner, certificate, &anchor).unwrap();
    let mut state = Machine::join_at(source.descriptor(), anchor.control);
    deliver(source, &mut state);
    state.intent = Some(prepared);
    (state, fresh)
}

#[test]
fn transfer_requires_exact_incoming_controller_consent_and_revokes_old_control_authority() {
    let (mut state, old, root) = current_owner();
    let new = additional_owner(&mut state, &old, &root);
    edit(&mut state, &old, vec![entry(1, "preserved")]);
    let mut peer = state.clone();
    peer.outbox.clear();
    state.intent = Some(Machine::prepare_control_transfer(&state, &old, new.id()).unwrap());
    let before = state.objects.clone();
    assert!(matches!(
        Machine::complete_intent(&mut state, &old),
        Err(Error::ControlApprovalRequired)
    ));
    assert_eq!(state.objects, before);
    let offer = Machine::control_transfer_offer(&state, &old).unwrap();
    let offer = ControlTransfer::decode(&offer.bytes().unwrap()).unwrap();
    let approval = Machine::accept_control_transfer(&peer, &new, &offer).unwrap();
    let approval = Signed::<ControlApproval>::decode(&approval.bytes().unwrap()).unwrap();
    let mut wrong = approval.clone();
    wrong.body.statement = Hash([99; 32]);
    assert!(Machine::approve_control_transfer(&mut state, wrong).is_err());
    Machine::approve_control_transfer(&mut state, approval).unwrap();
    let mut restarted: SyncState = decode(&canonical(&state).unwrap()).unwrap();
    let head = Machine::complete_intent(&mut state, &old).unwrap();
    assert_eq!(
        Machine::complete_intent(&mut restarted, &old).unwrap(),
        head
    );
    assert_eq!(state.objects[&head], restarted.objects[&head]);
    deliver(&state, &mut peer);
    assert_eq!(Machine::controller(&peer).unwrap(), new.id());
    let roster = Machine::members(&state, &old).unwrap();
    assert!(matches!(
        Machine::prepare_membership(&state, &old, roster.clone()),
        Err(Error::InsufficientRole)
    ));
    peer.intent = Some(Machine::prepare_membership(&peer, &new, roster).unwrap());
    Machine::complete_intent(&mut peer, &new).unwrap();
    assert_eq!(
        Machine::replay(&peer, &new).unwrap().projection.entries()[0].password,
        "preserved"
    );
}

#[test]
fn root_recovery_preserves_content_with_fresh_keys_and_rejects_missing_or_wrong_approval() {
    let (mut source, old, root) = current_owner();
    edit(&mut source, &old, vec![entry(1, "current content")]);
    let (mut state, fresh) = recovery(&source, &old, &root);
    assert!(Machine::replay(&state, &fresh).unwrap().role.is_none());
    assert!(matches!(
        Machine::complete_intent(&mut state, &fresh),
        Err(Error::ControlApprovalRequired)
    ));
    assert!(
        Machine::approve_control_recovery(&mut state, &fresh, &SigningKey::from_bytes(&[9; 32]))
            .is_err()
    );
    Machine::approve_control_recovery(&mut state, &fresh, &root).unwrap();
    let mut restarted: SyncState = decode(&canonical(&state).unwrap()).unwrap();
    let head = Machine::complete_intent(&mut state, &fresh).unwrap();
    assert_eq!(
        Machine::complete_intent(&mut restarted, &fresh).unwrap(),
        head
    );
    let replay = Machine::replay(&state, &fresh).unwrap();
    assert!(replay.ready);
    assert_eq!(replay.projection.entries()[0].password, "current content");
    assert_eq!(Machine::controller(&state).unwrap(), fresh.id());
    deliver(&state, &mut source);
    assert!(Machine::replay(&source, &old).unwrap().role.is_none());
    assert!(matches!(
        Machine::prepare(&source, &old, empty_transaction()),
        Err(Error::MissingDependency)
    ));
}

#[test]
fn recovery_requires_exact_current_content_frontier_and_a_present_independent_pin() {
    let (mut state, old, root) = current_owner();
    let anchor = Machine::recovery_anchor(&state, &old).unwrap();
    edit(&mut state, &old, vec![entry(1, "newer within same epoch")]);
    let (fresh, cert) = DeviceSecret::generate(&root).unwrap();
    assert!(matches!(
        Machine::prepare_control_recovery(&state, &old, cert, &anchor),
        Err(Error::MissingDependency)
    ));
    let (mut recovery, _) = recovery(&state, &old, &root);
    recovery.required_control = Some(Hash([99; 32]));
    assert!(matches!(
        Machine::approve_control_recovery(&mut recovery, &fresh, &root),
        Err(Error::UnauthorizedDevice | Error::MissingDependency)
    ));
}

#[test]
fn normal_and_root_authorized_successors_form_a_fork_in_both_delivery_orders() {
    let (base, old, root) = current_owner();
    let (mut recovered, fresh) = recovery(&base, &old, &root);
    Machine::approve_control_recovery(&mut recovered, &fresh, &root).unwrap();
    Machine::complete_intent(&mut recovered, &fresh).unwrap();
    let mut normal = base.clone();
    let roster = Machine::members(&normal, &old).unwrap();
    normal.intent = Some(Machine::prepare_membership(&normal, &old, roster).unwrap());
    Machine::complete_intent(&mut normal, &old).unwrap();
    for (left, right) in [(&normal, &recovered), (&recovered, &normal)] {
        let mut joined = left.clone();
        deliver(right, &mut joined);
        assert!(matches!(
            Machine::replay(&joined, &fresh),
            Err(Error::ControlFork)
        ));
    }
}

#[test]
fn transfer_verifies_closed_checkpoint_and_preserves_later_edits_as_drafts() {
    let (mut old_state, old, root) = current_owner();
    let new = additional_owner(&mut old_state, &old, &root);
    let mut peer = old_state.clone();
    old_state.intent = Some(Machine::prepare_control_transfer(&old_state, &old, new.id()).unwrap());
    let offer = Machine::control_transfer_offer(&old_state, &old).unwrap();
    let late = edit(&mut peer, &new, vec![entry(1, "late draft")]);
    let approval = Machine::accept_control_transfer(&peer, &new, &offer).unwrap();
    Machine::approve_control_transfer(&mut old_state, approval).unwrap();
    Machine::complete_intent(&mut old_state, &old).unwrap();
    deliver(&old_state, &mut peer);
    assert!(
        Machine::replay(&peer, &new)
            .unwrap()
            .excluded
            .contains(&late)
    );
    assert_eq!(
        Machine::excluded_edits(&peer, &new).unwrap()[0]
            .transaction
            .mutations
            .len(),
        1
    );
}

#[test]
fn transfer_acceptance_preserves_state_when_an_intent_is_pending_or_the_offer_is_tampered() {
    let (mut state, old, root) = current_owner();
    let new = additional_owner(&mut state, &old, &root);
    let mut peer = state.clone();
    state.intent = Some(Machine::prepare_control_transfer(&state, &old, new.id()).unwrap());
    let offer = Machine::control_transfer_offer(&state, &old).unwrap();

    peer.intent = Some(Machine::prepare(&peer, &new, empty_transaction()).unwrap());
    let before = canonical(&peer).unwrap();
    assert!(matches!(
        Machine::accept_control_transfer(&peer, &new, &offer),
        Err(Error::PendingIntent)
    ));
    assert_eq!(canonical(&peer).unwrap(), before);

    peer.intent = None;
    let before = canonical(&peer).unwrap();
    let mut tampered = offer.clone();
    tampered.signature.0[0] ^= 1;
    assert!(matches!(
        Machine::accept_control_transfer(&peer, &new, &tampered),
        Err(Error::InvalidSignature)
    ));
    assert_eq!(canonical(&peer).unwrap(), before);
    assert!(Machine::accept_control_transfer(&peer, &new, &offer).is_ok());
}
