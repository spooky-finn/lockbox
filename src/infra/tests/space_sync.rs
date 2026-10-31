use super::*;
use crate::{
    core::sync::{Machine, Role, Transaction},
    infra::{profile::ProviderConfiguration, sync::Download},
};

const FAST: KdfParams = KdfParams {
    memory_kib: 8192,
    passes: 1,
    lanes: 1,
};

fn profile_vault(path: &Path, name: &str) -> Vault {
    let profile = BootstrapProfile::generate(
        name.into(),
        "Team".into(),
        ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "test-bucket".into(),
            access_key: "access".into(),
            secret_key: "secret".into(),
        },
        &SecretString::from("recovery password".to_owned()),
        FAST,
    )
    .unwrap();
    Vault::create_profile(
        path,
        &SecretString::from("master".to_owned()),
        FAST,
        profile,
    )
    .unwrap()
}

fn entry(title: &str) -> Entry {
    let mut entry = Entry::new();
    entry.title = title.into();
    entry.password = "private password".into();
    entry
}

fn delivery(from: &Vault) -> Download {
    Download {
        objects: from
            .profile
            .as_ref()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .objects
            .iter()
            .rev()
            .map(|(h, b)| (*h, b.0.clone()))
            .collect(),
        incomplete: false,
    }
}

#[test]
fn local_event_and_outbox_survive_restart_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alice.lbx");
    let mut vault = profile_vault(&path, "Alice");
    vault.entries.push(entry("Durable"));
    vault.save_changes().unwrap();
    let sync = vault.profile.as_ref().unwrap().sync.as_ref().unwrap();
    assert_eq!(sync.objects.len(), 2);
    assert_eq!(sync.outbox.len(), 2);
    assert!(sync.intent.is_none());
    let expected = sync.objects.clone();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.windows(16).any(|s| s == b"private password"));
    drop(vault);
    let vault = Vault::open(path, &SecretString::from("master".to_owned())).unwrap();
    assert_eq!(vault.entries[0].title, "Durable");
    assert_eq!(
        vault
            .profile
            .as_ref()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .objects,
        expected
    );
}

#[test]
fn crash_after_durable_intent_recovers_exact_event_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alice.lbx");
    let mut vault = profile_vault(&path, "Alice");
    let profile = vault.profile.as_ref().unwrap();
    let device = profile.sync_secret();
    let sync = profile.sync.as_ref().unwrap();
    let transaction = Transaction::diff(
        &Machine::replay(sync, &device).unwrap().projection,
        &[entry("Recovered")],
        &[],
    )
    .unwrap();
    let intent = Machine::prepare(sync, &device, transaction).unwrap();
    vault
        .profile
        .as_mut()
        .unwrap()
        .sync
        .as_mut()
        .unwrap()
        .intent = Some(intent);
    vault.save_inner(false).unwrap();
    let mut expected = vault
        .profile
        .as_ref()
        .unwrap()
        .sync
        .as_ref()
        .unwrap()
        .clone();
    let event = Machine::complete_intent(&mut expected, &device).unwrap();
    drop(vault);
    let vault = Vault::open(path, &SecretString::from("master".to_owned())).unwrap();
    let sync = vault.profile.as_ref().unwrap().sync.as_ref().unwrap();
    assert_eq!(sync.objects[&event], expected.objects[&event]);
    assert!(sync.outbox.contains(&event));
    assert!(sync.intent.is_none());
    assert_eq!(vault.entries[0].title, "Recovered");
}

#[test]
fn failed_intent_write_never_allocates_a_different_signing_slot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alice.lbx");
    let mut vault = profile_vault(&path, "Alice");
    std::fs::create_dir(sibling(&path, "tmp")).unwrap();
    let original = std::fs::read(&path).unwrap();
    vault.entries.push(entry("Retry"));
    assert!(vault.save_changes().is_err());
    let profile = vault.profile.as_ref().unwrap();
    let device = profile.sync_secret();
    let mut expected = profile.sync.as_ref().unwrap().clone();
    assert!(expected.intent.is_some());
    let hash = Machine::complete_intent(&mut expected, &device).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir(sibling(&path, "tmp")).unwrap();
    // A restart has no emitted signature if intent never reached disk.
    // This still-open installation must resume its reserved exact body.
    vault.recover_sync().unwrap();
    assert_eq!(
        vault
            .profile
            .as_ref()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .objects[&hash],
        expected.objects[&hash]
    );
    assert_eq!(vault.entries[0].title, "Retry");
}

#[test]
fn two_independent_profiles_exchange_and_reopen_without_cloned_signers() {
    let dir = tempfile::tempdir().unwrap();
    let alice_path = dir.path().join("alice.lbx");
    let bob_path = dir.path().join("bob.lbx");
    let mut alice = profile_vault(&alice_path, "Alice");
    let mut bob = profile_vault(&bob_path, "Bob");
    let bob_certificate = bob.device_certificate().unwrap();
    assert_ne!(
        alice.device_certificate().unwrap().body.device,
        bob_certificate.body.device
    );
    alice.entries.push(entry("Before admission"));
    alice.save_changes().unwrap();
    let members = vec![
        (alice.device_certificate().unwrap(), Role::Owner),
        (bob_certificate, Role::Editor),
    ];
    alice.set_space_members(members).unwrap();
    bob.join_space(alice.space_descriptor().unwrap()).unwrap();
    // An awaiting-admission profile must remain reopenable before first download.
    drop(bob);
    let mut bob = Vault::open(&bob_path, &SecretString::from("master".to_owned())).unwrap();
    bob.receive_sync(delivery(&alice)).unwrap();
    assert_eq!(bob.entries[0].title, "Before admission");
    bob.entries[0].title = "Bob changed it".into();
    bob.save_changes().unwrap();
    alice.receive_sync(delivery(&bob)).unwrap();
    assert_eq!(alice.entries[0].title, "Bob changed it");
    let (alice_event, bob_event) = (
        alice
            .profile
            .as_ref()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .objects
            .clone(),
        bob.profile
            .as_ref()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .objects
            .clone(),
    );
    assert_eq!(alice_event, bob_event);
    drop(alice);
    drop(bob);
    let alice = Vault::open(alice_path, &SecretString::from("master".to_owned())).unwrap();
    let bob = Vault::open(bob_path, &SecretString::from("master".to_owned())).unwrap();
    assert_eq!(alice.entries, bob.entries);
    assert!(alice.sync_replay().unwrap().unwrap().history_verified);
    assert!(!bob.sync_replay().unwrap().unwrap().history_verified);
}

#[test]
fn verified_fork_is_persistently_frozen_but_last_readable_data_survives() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alice.lbx");
    let mut vault = profile_vault(&path, "Alice");
    vault.entries.push(entry("Known safe"));
    vault.save_changes().unwrap();
    let profile = vault.profile.as_ref().unwrap();
    let device = profile.sync_secret();
    let mut fork = profile.sync.as_ref().unwrap().clone();
    let baseline = Machine::replay(&fork, &device).unwrap().projection;
    let mut left = baseline.entries();
    left[0].title = "Branch A".into();
    let mut right = baseline.entries();
    right[0].title = "Branch B".into();
    fork.intent = Some(
        Machine::prepare(
            &fork,
            &device,
            Transaction::diff(&baseline, &left, &[]).unwrap(),
        )
        .unwrap(),
    );
    Machine::complete_intent(&mut fork, &device).unwrap();
    vault.entries = right;
    vault.save_changes().unwrap();
    assert!(
        vault
            .receive_sync(Download {
                objects: fork.objects.into_iter().map(|(h, b)| (h, b.0)).collect(),
                incomplete: false
            })
            .is_err()
    );
    assert_eq!(vault.entries[0].title, "Branch B");
    drop(vault);
    let mut vault = Vault::open(path, &SecretString::from("master".to_owned())).unwrap();
    assert_eq!(vault.entries[0].title, "Branch B");
    assert_eq!(
        vault.profile.as_ref().unwrap().sync.as_ref().unwrap().fault,
        Some(crate::core::sync::Error::DeviceFork)
    );
    vault.entries[0].title = "Blocked".into();
    assert!(vault.save_changes().is_err());
}

#[test]
fn bootstrap_profile_activation_requires_recovery_and_preserves_records_and_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.lbx");
    let mut vault = profile_vault(&path, "Alice");
    vault.profile.as_mut().unwrap().cancel_activation();
    vault.entries.push(entry("Existing local secret"));
    vault.save().unwrap();
    let identity = vault.profile.as_ref().unwrap().identity.device_id;
    assert!(matches!(
        vault.activate_sync(&SecretString::from("wrong".to_owned())),
        Err(VaultError::RecoveryFailed)
    ));
    assert!(vault.space_descriptor().is_none());
    assert_eq!(vault.entries[0].title, "Existing local secret");
    vault
        .activate_sync(&SecretString::from("recovery password".to_owned()))
        .unwrap();
    assert_eq!(vault.device_certificate().unwrap().body.device.0, identity);
    assert_eq!(
        vault.sync_replay().unwrap().unwrap().projection.entries(),
        vault.entries
    );
    drop(vault);
    let vault = Vault::open(path, &SecretString::from("master".to_owned())).unwrap();
    assert_eq!(vault.entries[0].title, "Existing local secret");
    assert_eq!(
        vault
            .profile
            .as_ref()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .outbox
            .len(),
        2
    );
}

#[test]
fn failed_activation_keeps_existing_records_readable_and_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.lbx");
    let mut vault = profile_vault(&path, "Alice");
    vault.profile.as_mut().unwrap().cancel_activation();
    vault.entries.push(entry("Keep this"));
    vault.save().unwrap();
    std::fs::create_dir(sibling(&path, "tmp")).unwrap();
    assert!(
        vault
            .activate_sync(&SecretString::from("recovery password".to_owned()))
            .is_err()
    );
    assert_eq!(vault.entries[0].title, "Keep this");
    std::fs::remove_dir(sibling(&path, "tmp")).unwrap();
    vault.recover_sync().unwrap();
    drop(vault);
    let vault = Vault::open(path, &SecretString::from("master".to_owned())).unwrap();
    assert_eq!(vault.entries[0].title, "Keep this");
    assert!(vault.space_descriptor().is_some());
}
