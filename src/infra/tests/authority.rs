use super::*;
use crate::{
    core::sync::{ControlTransition, Error, Machine, Role},
    infra::{sync::Download, witness::MemoryWitness},
};

const FAST: KdfParams = KdfParams {
    memory_kib: 8192,
    passes: 1,
    lanes: 1,
};
fn password() -> SecretString {
    SecretString::from("master".to_owned())
}
fn recovery_password() -> SecretString {
    SecretString::from("recovery".to_owned())
}
fn configuration(name: &str) -> SharedProfile {
    SharedProfile {
        username: name.into(),
        space_name: "Team".into(),
        provider: ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "fixture".into(),
            access_key: "fictional-access".into(),
            secret_key: "fictional-secret".into(),
        },
    }
}
fn create(store: &ProfileStore, path: &Path, name: &str) -> Vault {
    store
        .create_shared(
            path,
            &password(),
            &recovery_password(),
            FAST,
            configuration(name),
        )
        .unwrap()
}
fn add(vault: &mut Vault, title: &str) {
    let mut entry = Entry::new();
    entry.title = title.into();
    entry.password = "fictional private content".into();
    vault.entries.push(entry);
    vault.save_changes().unwrap();
}
fn delivery(vault: &Vault) -> Download {
    Download {
        incomplete: false,
        objects: vault
            .sync_state()
            .unwrap()
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
    }
}
fn another_owner(store: &ProfileStore, old: &mut Vault, path: &Path) -> Vault {
    let mut peer = store
        .restore_device(
            path,
            &password(),
            FAST,
            DeviceRecovery {
                source: old,
                recovery_password: &recovery_password(),
                descriptor: old.space_descriptor().unwrap(),
                control_anchor: old.space_anchor().unwrap(),
                configuration: configuration("Other owner device"),
            },
        )
        .unwrap();
    let offer = old
        .issue_invitation(peer.device_certificate().unwrap(), Role::Owner, None)
        .unwrap();
    let consent = peer
        .accept_invitation(offer, &old.space_descriptor().unwrap())
        .unwrap();
    peer.join_invitation(old.admit_invitation(consent, 0).unwrap())
        .unwrap();
    peer
}

#[test]
fn transfer_reservation_and_consent_survive_restart_without_old_controller_equivocation() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("owner.lbx");
    let peer_path = directory.path().join("peer.lbx");
    let mut owner = create(&store, &path, "Owner");
    let mut peer = another_owner(&store, &mut owner, &peer_path);
    add(&mut owner, "Preserved record");
    peer.receive_sync(delivery(&owner)).unwrap();
    let target = peer.device_certificate().unwrap().body.device;
    let offer = owner.begin_control_transfer(target).unwrap();
    assert_eq!(
        owner.pending_control_approval(),
        Some(ControlTransition::Transfer)
    );
    let before = owner.sync_state().unwrap().objects.clone();
    drop(owner);
    let mut owner = store.open(&path, &password()).unwrap();
    assert_eq!(
        owner
            .begin_control_transfer(target)
            .unwrap()
            .bytes()
            .unwrap(),
        offer.bytes().unwrap()
    );
    assert_eq!(owner.sync_state().unwrap().objects, before);
    let consent = peer.accept_control_transfer(&offer).unwrap();
    drop(peer);
    let mut peer = store.open(&peer_path, &password()).unwrap();
    assert_eq!(
        peer.accept_control_transfer(&offer)
            .unwrap()
            .bytes()
            .unwrap(),
        consent.bytes().unwrap()
    );
    owner.finish_control_transfer(consent.clone()).unwrap();
    let completed = owner.sync_state().unwrap().objects.clone();
    owner.finish_control_transfer(consent).unwrap();
    assert_eq!(owner.sync_state().unwrap().objects, completed);
    peer.receive_sync(delivery(&owner)).unwrap();
    assert_eq!(peer.space_controller().unwrap(), target);
    assert_eq!(peer.entries[0].title, "Preserved record");
    assert!(matches!(
        owner.set_space_members(owner.space_members().unwrap()),
        Err(VaultError::Sync(Error::InsufficientRole))
    ));
    peer.set_space_members(peer.space_members().unwrap())
        .unwrap();
}

#[test]
fn recovery_keeps_sources_intact_and_relays_unpublished_closed_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = create(&store, &directory.path().join("owner.lbx"), "Owner");
    add(&mut owner, "Recovered complete content");
    let anchor = owner.recovery_anchor().unwrap();
    let source_file = fs::read(owner.path()).unwrap();
    let old = owner.device_certificate().unwrap();
    let recovered = store
        .recover_controller(
            directory.path().join("recovered.lbx"),
            &password(),
            FAST,
            ControllerRecovery {
                authority: &owner,
                content: &owner,
                recovery_password: &recovery_password(),
                anchor,
                configuration: configuration("Recovered controller"),
            },
        )
        .unwrap();
    assert_eq!(fs::read(owner.path()).unwrap(), source_file);
    let fresh = recovered.device_certificate().unwrap();
    assert_eq!(fresh.body.user, old.body.user);
    assert_ne!(fresh.body.device, old.body.device);
    assert_ne!(fresh.body.hpke_key, old.body.hpke_key);
    assert_eq!(recovered.space_controller().unwrap(), fresh.body.device);
    assert_eq!(recovered.entries[0].title, "Recovered complete content");
    let replay = recovered.sync_replay().unwrap().unwrap();
    let plan = crate::infra::sync::publication(recovered.sync_state().unwrap(), &replay).unwrap();
    assert_eq!(plan.len(), recovered.sync_state().unwrap().objects.len());
    // No provider had the offline source events. The recovered installation can
    // publish their signed ciphertext before the recovery record that closes them.
    let mut reader = Machine::join_at(recovered.space_descriptor().unwrap(), replay.membership);
    for (hash, bytes) in plan {
        Machine::receive(&mut reader, hash, bytes.0).unwrap();
    }
    let old_key = owner.profile.as_ref().unwrap().sync_secret();
    let replay = Machine::replay(&reader, &old_key).unwrap();
    assert!(replay.role.is_none());
    assert_eq!(
        replay.projection.entries()[0].title,
        "Recovered complete content"
    );
}

#[test]
fn root_claim_reservation_prevents_a_second_profile_fork_and_allows_resume_after_lost_ack() {
    let directory = tempfile::tempdir().unwrap();
    let witnesses = Arc::new(MemoryWitness::default());
    let store = ProfileStore::new(witnesses.clone());
    let mut owner = create(&store, &directory.path().join("owner.lbx"), "Owner");
    add(&mut owner, "Recovery must retain this");
    let anchor = owner.recovery_anchor().unwrap();
    let target = directory.path().join("recovered.lbx");
    // Fresh identity reservation, file, preapproval save, root claim succeed;
    // the root-approval file rename succeeds but its device-witness ACK fails.
    witnesses.fail_writes_after(4, 1);
    assert!(
        store
            .recover_controller(
                &target,
                &password(),
                FAST,
                ControllerRecovery {
                    authority: &owner,
                    content: &owner,
                    recovery_password: &recovery_password(),
                    anchor: anchor.clone(),
                    configuration: configuration("Recovery")
                }
            )
            .is_err()
    );
    let recovered = store.open(&target, &password()).unwrap();
    assert!(recovered.pending_control_approval().is_none());
    assert_eq!(recovered.entries[0].title, "Recovery must retain this");
    let rejected = store.recover_controller(
        directory.path().join("second-recovery.lbx"),
        &password(),
        FAST,
        ControllerRecovery {
            authority: &owner,
            content: &owner,
            recovery_password: &recovery_password(),
            anchor,
            configuration: configuration("Forbidden duplicate"),
        },
    );
    assert!(matches!(
        rejected,
        Err(VaultError::Sync(Error::RecoveryReserved))
    ));
}

#[test]
fn stale_content_boundary_and_wrong_root_password_fail_before_creating_a_recovery_profile() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = create(&store, &directory.path().join("owner.lbx"), "Owner");
    let anchor = owner.recovery_anchor().unwrap();
    add(&mut owner, "Newer data in the same control epoch");
    let target = directory.path().join("recovered.lbx");
    let result = store.recover_controller(
        &target,
        &password(),
        FAST,
        ControllerRecovery {
            authority: &owner,
            content: &owner,
            recovery_password: &recovery_password(),
            anchor,
            configuration: configuration("Recovery"),
        },
    );
    assert!(matches!(
        result,
        Err(VaultError::Sync(Error::MissingDependency))
    ));
    assert!(!target.exists());
    let result = store.recover_controller(
        &target,
        &password(),
        FAST,
        ControllerRecovery {
            authority: &owner,
            content: &owner,
            recovery_password: &SecretString::from("wrong".to_owned()),
            anchor: owner.recovery_anchor().unwrap(),
            configuration: configuration("Recovery"),
        },
    );
    assert!(matches!(result, Err(VaultError::RecoveryFailed)));
    assert!(!target.exists());
}
