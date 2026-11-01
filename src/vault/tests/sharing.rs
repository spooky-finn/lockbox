use crate::{
    sync::protocol::{AdmissionReceipt, Role},
    vault::{witness::MemoryWitness, *},
};
use std::sync::Arc;

const FAST: KdfParams = KdfParams {
    memory_kib: 8192,
    passes: 1,
    lanes: 1,
};

fn password() -> SecretString {
    SecretString::from("test master".to_owned())
}

pub(super) fn create(store: &ProfileStore, path: &Path) -> Vault {
    store
        .create_shared(
            path,
            &password(),
            &SecretString::from("test recovery".to_owned()),
            FAST,
            SharedProfile {
                username: "Test user".into(),
                space_name: "Test space".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "test-bucket".into(),
                    access_key: "fictional-access".into(),
                    secret_key: "fictional-secret".into(),
                },
            },
        )
        .unwrap()
}

pub(super) fn admit(owner: &mut Vault, guest: &mut Vault, role: Role) -> AdmissionReceipt {
    let descriptor = owner.space_descriptor().unwrap();
    let offer = owner
        .issue_invitation(guest.device_certificate().unwrap(), role, None)
        .unwrap();
    let request = guest.accept_invitation(offer, &descriptor).unwrap();
    owner.admit_invitation(request, 0).unwrap()
}

pub(super) fn share(owner: &mut Vault, guest: &mut Vault, role: Role) {
    let receipt = admit(owner, guest, role);
    guest.join_invitation(receipt).unwrap();
}

#[test]
fn offers_responses_and_receipts_survive_restart_and_exact_retries() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let owner_path = directory.path().join("owner.lbx");
    let guest_path = directory.path().join("guest.lbx");
    let mut owner = create(&store, &owner_path);
    let mut guest = create(&store, &guest_path);
    let certificate = guest.device_certificate().unwrap();
    let descriptor = owner.space_descriptor().unwrap();
    let offer = owner
        .issue_invitation(certificate.clone(), Role::Editor, Some(100))
        .unwrap();
    let response = guest.accept_invitation(offer.clone(), &descriptor).unwrap();
    drop(owner);
    drop(guest);
    let mut owner = store.open(&owner_path, &password()).unwrap();
    let mut guest = store.open(&guest_path, &password()).unwrap();
    assert_eq!(
        owner
            .issue_invitation(certificate, Role::Editor, Some(100))
            .unwrap()
            .bytes()
            .unwrap(),
        offer.bytes().unwrap()
    );
    assert_eq!(
        guest
            .accept_invitation(offer, &descriptor)
            .unwrap()
            .bytes()
            .unwrap(),
        response.bytes().unwrap()
    );
    let receipt = owner.admit_invitation(response.clone(), 50).unwrap();
    let count = owner.sync_state().unwrap().objects.len();
    drop(owner);
    let mut owner = store.open(&owner_path, &password()).unwrap();
    assert_eq!(
        owner
            .admit_invitation(response, 200)
            .unwrap()
            .bytes()
            .unwrap(),
        receipt.bytes().unwrap()
    );
    assert_eq!(owner.sync_state().unwrap().objects.len(), count);
    guest.join_invitation(receipt.clone()).unwrap();
    let mut entry = Entry::new();
    entry.title = "Post admission edit".into();
    guest.entries.push(entry);
    guest.save_changes().unwrap();
    let event_count = guest.sync_state().unwrap().objects.len();
    guest.join_invitation(receipt).unwrap();
    assert_eq!(guest.sync_state().unwrap().objects.len(), event_count);
    assert_eq!(guest.entries.len(), 1);
}

#[test]
fn cancellation_expiry_and_different_acceptance_do_not_change_membership() {
    use crate::sync::protocol::Error;
    use crate::sync::protocol::Machine;
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("owner.lbx");
    let mut owner = create(&store, &path);
    let mut guest = create(&store, &directory.path().join("guest.lbx"));
    let descriptor = owner.space_descriptor().unwrap();
    let cert = guest.device_certificate().unwrap();
    let offer = owner
        .issue_invitation(cert.clone(), Role::Reader, Some(10))
        .unwrap();
    let request = guest.accept_invitation(offer.clone(), &descriptor).unwrap();
    assert!(matches!(
        owner.admit_invitation(request.clone(), 10),
        Err(VaultError::Sync(Error::InvitationExpired))
    ));
    owner.cancel_invitation(offer.id()).unwrap();
    drop(owner);
    let mut owner = store.open(&path, &password()).unwrap();
    assert!(matches!(
        owner.admit_invitation(request, 9),
        Err(VaultError::Sync(Error::InvitationCancelled))
    ));
    let offer = owner
        .issue_invitation(cert.clone(), Role::Reader, None)
        .unwrap();
    let first = guest.accept_invitation(offer.clone(), &descriptor).unwrap();
    let second = Machine::accept_invitation(
        &offer,
        &descriptor,
        &guest.profile.as_ref().unwrap().sync_secret(),
        &cert,
    )
    .unwrap();
    owner.admit_invitation(first, 0).unwrap();
    let count = owner.sync_state().unwrap().objects.len();
    assert!(matches!(
        owner.admit_invitation(second, 0),
        Err(VaultError::Sync(Error::InvitationUsed))
    ));
    assert_eq!(owner.sync_state().unwrap().objects.len(), count);
}

#[test]
fn failed_artifact_save_never_exports_a_different_retry() {
    use crate::vault::file::sibling;
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let owner_path = directory.path().join("owner.lbx");
    let guest_path = directory.path().join("guest.lbx");
    let mut owner = create(&store, &owner_path);
    let mut guest = create(&store, &guest_path);
    let cert = guest.device_certificate().unwrap();
    let original = std::fs::read(&owner_path).unwrap();
    std::fs::create_dir(sibling(&owner_path, "tmp")).unwrap();
    assert!(
        owner
            .issue_invitation(cert.clone(), Role::Editor, None)
            .is_err()
    );
    assert_eq!(std::fs::read(&owner_path).unwrap(), original);
    let reserved = owner
        .profile
        .as_ref()
        .unwrap()
        .sharing
        .issued
        .values()
        .next()
        .unwrap()
        .invitation
        .bytes()
        .unwrap();
    std::fs::remove_dir(sibling(&owner_path, "tmp")).unwrap();
    let offer = owner.issue_invitation(cert, Role::Editor, None).unwrap();
    assert_eq!(offer.bytes().unwrap(), reserved);
    let descriptor = owner.space_descriptor().unwrap();
    std::fs::create_dir(sibling(&guest_path, "tmp")).unwrap();
    assert!(guest.accept_invitation(offer.clone(), &descriptor).is_err());
    let reserved = guest.profile.as_ref().unwrap().sharing.accepted[&offer.id()]
        .admission
        .bytes()
        .unwrap();
    std::fs::remove_dir(sibling(&guest_path, "tmp")).unwrap();
    let response = guest.accept_invitation(offer, &descriptor).unwrap();
    assert_eq!(response.bytes().unwrap(), reserved);
    std::fs::create_dir(sibling(&owner_path, "tmp")).unwrap();
    assert!(owner.admit_invitation(response.clone(), 0).is_err());
    assert!(owner.sync_state().unwrap().intent.is_some());
    assert_eq!(owner.sync_state().unwrap().objects.len(), 1);
    std::fs::remove_dir(sibling(&owner_path, "tmp")).unwrap();
    let receipt = owner.admit_invitation(response, 100).unwrap();
    assert_eq!(owner.sync_state().unwrap().objects.len(), 2);
    guest.join_invitation(receipt).unwrap();
    assert_eq!(
        guest.sync_replay().unwrap().unwrap().role,
        Some(Role::Editor)
    );
}

#[test]
fn receipt_commit_ack_failure_recovers_without_a_second_admission() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let owner_path = directory.path().join("owner.lbx");
    let mut owner = create(&store, &owner_path);
    let mut guest = create(&store, &directory.path().join("guest.lbx"));
    let descriptor = owner.space_descriptor().unwrap();
    let offer = owner
        .issue_invitation(guest.device_certificate().unwrap(), Role::Editor, None)
        .unwrap();
    let response = guest.accept_invitation(offer.clone(), &descriptor).unwrap();
    // Intent and control commit succeed; acknowledgement of the receipt file fails.
    witnesses.fail_writes_after(2, 1);
    assert!(owner.admit_invitation(response.clone(), 0).is_err());
    assert_eq!(owner.sync_state().unwrap().objects.len(), 2);
    assert!(
        owner.profile.as_ref().unwrap().sharing.issued[&offer.id()]
            .receipt
            .is_some()
    );
    let expected = owner.profile.as_ref().unwrap().sharing.issued[&offer.id()]
        .receipt
        .as_ref()
        .unwrap()
        .bytes()
        .unwrap();
    drop(owner);
    let mut owner = store.open(&owner_path, &password()).unwrap();
    let receipt = owner.admit_invitation(response, 0).unwrap();
    assert_eq!(receipt.bytes().unwrap(), expected);
    assert_eq!(owner.sync_state().unwrap().objects.len(), 2);
    guest.join_invitation(receipt).unwrap();
}

#[test]
fn independent_minimum_anchor_rejects_an_old_receipt_until_owner_refreshes() {
    use crate::sync::protocol::Error;
    use crate::sync::protocol::Machine;
    use crate::sync::protocol::canonical;
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = create(&store, &directory.path().join("owner.lbx"));
    let mut guest = create(&store, &directory.path().join("guest.lbx"));
    let descriptor = owner.space_descriptor().unwrap();
    let offer = owner
        .issue_invitation(guest.device_certificate().unwrap(), Role::Editor, None)
        .unwrap();
    let response = guest.accept_invitation(offer.clone(), &descriptor).unwrap();
    let old_receipt = owner.admit_invitation(response, 0).unwrap();
    let members = owner.sync_replay().unwrap().unwrap();
    let device = owner.profile.as_ref().unwrap().sync_secret();
    let roster = Machine::members(owner.sync_state().unwrap(), &device).unwrap();
    owner.set_space_members(roster).unwrap();
    let minimum = owner.space_anchor().unwrap();
    assert_ne!(minimum, members.membership);
    // Model the independently authenticated anchor carried by a recovery profile.
    let profile = guest.profile.as_mut().unwrap();
    profile.space.space_id = descriptor.space.0;
    profile.space.namespace = descriptor.namespace.0;
    profile.sync = Some(Machine::join_at(descriptor, minimum));
    guest.save_inner(false).unwrap();
    let before = canonical(guest.sync_state().unwrap()).unwrap();
    assert!(matches!(
        guest.join_invitation(old_receipt),
        Err(VaultError::Sync(Error::MissingDependency))
    ));
    assert_eq!(canonical(guest.sync_state().unwrap()).unwrap(), before);
    let refreshed = owner.refresh_invitation_receipt(offer.id()).unwrap();
    assert_eq!(refreshed.control(), minimum);
    guest.join_invitation(refreshed).unwrap();
    assert_eq!(guest.sync_state().unwrap().required_control, Some(minimum));
}
