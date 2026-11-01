use super::*;
use crate::core::sync::{Admission, AdmissionReceipt, Invitation};

fn current_owner() -> (SyncState, DeviceSecret) {
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

fn consent(
    state: &SyncState,
    owner: &DeviceSecret,
    guest: &DeviceSecret,
    certificate: Signed<Certificate>,
) -> (Invitation, Admission) {
    let offer =
        Machine::offer_invitation(state, owner, certificate.clone(), Role::Editor, None).unwrap();
    let offer = Invitation::decode(&offer.bytes().unwrap()).unwrap();
    let proof =
        Machine::accept_invitation(&offer, &state.descriptor(), guest, &certificate).unwrap();
    let proof = Admission::decode(&proof.bytes().unwrap()).unwrap();
    (offer, proof)
}

fn complete(state: &mut SyncState, owner: &DeviceSecret, proof: Admission) -> Hash {
    state.intent = Some(Machine::prepare_admission(state, owner, proof).unwrap());
    let encoded = canonical(state).unwrap();
    *state = decode(&encoded).unwrap();
    Machine::complete_intent(state, owner).unwrap()
}

#[test]
fn signed_admission_bootstraps_current_checkpoint_and_is_retry_stable() {
    let (mut state, owner) = current_owner();
    edit(&mut state, &owner, vec![entry(1, "prior history")]);
    let (guest, cert) = identity(2);
    let (offer, proof) = consent(&state, &owner, &guest, cert);
    let reserved = Machine::prepare_admission(&state, &owner, proof.clone()).unwrap();
    state.intent = Some(reserved);
    let mut retry = state.clone();
    let admitted = Machine::complete_intent(&mut state, &owner).unwrap();
    assert_eq!(
        Machine::complete_intent(&mut retry, &owner).unwrap(),
        admitted
    );
    assert_eq!(state.objects[&admitted], retry.objects[&admitted]);
    let receipt = Machine::admission_receipt(&state, &owner, &proof).unwrap();
    let receipt = AdmissionReceipt::decode(&receipt.bytes().unwrap()).unwrap();
    let mut joined =
        Machine::join_admission(&offer, &proof, &receipt, &state.descriptor(), &guest).unwrap();
    assert_eq!(joined.required_control, Some(admitted));
    deliver(&state, &mut joined);
    let replay = Machine::replay(&joined, &guest).unwrap();
    assert!(replay.ready);
    assert!(!replay.history_verified);
    assert_eq!(replay.projection.entries()[0].password, "prior history");
    edit(&mut joined, &guest, vec![entry(1, "guest edit")]);
    deliver(&joined, &mut state);
    assert_eq!(
        Machine::replay(&state, &owner)
            .unwrap()
            .projection
            .digest()
            .unwrap(),
        Machine::replay(&joined, &guest)
            .unwrap()
            .projection
            .digest()
            .unwrap()
    );
    assert!(matches!(
        Machine::prepare_admission(&state, &owner, proof),
        Err(Error::InvitationUsed)
    ));
}

#[test]
fn consent_binds_recipient_role_offer_and_independent_root() {
    let (mut state, owner) = current_owner();
    let (guest, cert) = identity(2);
    let (other, other_cert) = identity(3);
    let (offer, proof) = consent(&state, &owner, &guest, cert.clone());
    assert!(matches!(
        Machine::accept_invitation(&offer, &state.descriptor(), &other, &other_cert),
        Err(Error::UnauthorizedDevice)
    ));
    let mut wrong_root = state.descriptor();
    wrong_root.owner_root = [99; 32];
    assert!(Machine::verify_invitation(&offer, &wrong_root).is_err());
    let mut tampered = proof.clone();
    tampered.offer.body.role = Role::Reader;
    assert!(Machine::prepare_admission(&state, &owner, tampered).is_err());
    let mut tampered = proof.clone();
    tampered.request = crypto::sign(
        tampered.request.body,
        &other.signer(),
        crate::core::sync::invitation::JOIN_DOMAIN,
    )
    .unwrap();
    assert!(Machine::prepare_admission(&state, &owner, tampered).is_err());
    let mut roster = Machine::members(&state, &owner).unwrap();
    roster.push((cert, Role::Editor));
    assert!(matches!(
        Machine::prepare_membership(&state, &owner, roster),
        Err(Error::ConsentRequired)
    ));
    complete(&mut state, &owner, proof);
    assert!(matches!(
        Machine::offer_invitation(&state, &guest, other_cert, Role::Editor, None),
        Err(Error::InsufficientRole)
    ));
}

#[test]
fn receipt_signature_detects_truncation_and_revoked_device_cannot_rejoin() {
    let (mut state, owner) = current_owner();
    let (guest, cert) = identity(2);
    let (offer, proof) = consent(&state, &owner, &guest, cert.clone());
    complete(&mut state, &owner, proof.clone());
    let old = Machine::admission_receipt(&state, &owner, &proof).unwrap();
    let members = Machine::members(&state, &owner).unwrap();
    state.intent = Some(Machine::prepare_membership(&state, &owner, members).unwrap());
    Machine::complete_intent(&mut state, &owner).unwrap();
    let mut receipt = Machine::admission_receipt(&state, &owner, &proof).unwrap();
    receipt.history = old.history.clone();
    assert!(
        Machine::join_admission(&offer, &proof, &receipt, &state.descriptor(), &guest).is_err()
    );
    receipt.confirmation.body.control = old.control();
    assert!(matches!(
        Machine::join_admission(&offer, &proof, &receipt, &state.descriptor(), &guest),
        Err(Error::InvalidSignature)
    ));
    let members = Machine::members(&state, &owner)
        .unwrap()
        .into_iter()
        .filter(|(c, _)| c.body.device != guest.id())
        .collect();
    state.intent = Some(Machine::prepare_membership(&state, &owner, members).unwrap());
    Machine::complete_intent(&mut state, &owner).unwrap();
    assert!(matches!(
        Machine::admission_receipt(&state, &owner, &proof),
        Err(Error::RetiredDevice)
    ));
    assert!(matches!(
        Machine::offer_invitation(&state, &owner, cert, Role::Editor, None),
        Err(Error::RetiredDevice)
    ));
}

#[test]
fn signed_control_without_consent_cannot_admit_a_device_on_receive() {
    let (mut state, owner) = current_owner();
    let (guest, cert) = identity(2);
    let (_, proof) = consent(&state, &owner, &guest, cert);
    let PreparedChange::Control(mut body) =
        Machine::prepare_admission(&state, &owner, proof).unwrap()
    else {
        panic!()
    };
    body.admissions.clear();
    let object = Object::Control(
        crypto::sign(
            body,
            &owner.signer(),
            super::super::profile::Profile::Current.control_domain(),
        )
        .unwrap(),
    );
    let hash = object.hash().unwrap();
    Machine::receive(&mut state, hash, object.bytes().unwrap()).unwrap();
    let replay = Machine::replay(&state, &owner).unwrap();
    assert_eq!(replay.rejected[&hash], Error::ConsentRequired);
    assert_eq!(replay.membership, state.genesis);
    assert!(Machine::replay(&state, &guest).unwrap().role.is_none());
}

#[test]
fn legacy_profile_upgrades_with_consent_and_cannot_downgrade() {
    let (mut state, owner) = owner();
    let (guest, cert) = identity(2);
    let (_, proof) = consent(&state, &owner, &guest, cert);
    complete(&mut state, &owner, proof);
    let members = Machine::members(&state, &owner).unwrap();
    let PreparedChange::Control(mut body) =
        Machine::prepare_membership(&state, &owner, members).unwrap()
    else {
        panic!()
    };
    body.epoch.profile = PROFILE.into();
    state.intent = Some(PreparedChange::Control(body));
    assert!(matches!(
        Machine::complete_intent(&mut state, &owner),
        Err(Error::UnsupportedProtocol)
    ));
}
