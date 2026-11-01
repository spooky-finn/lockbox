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

fn bootstrap_vault(path: &Path) -> Vault {
    let mut profile = BootstrapProfile::generate(
        "Alice".into(),
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
    profile.cancel_activation();
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
    assert!(vault.recover_sync().is_err());
    let unsaved = vault.sync_state().unwrap();
    assert_eq!(unsaved.objects.len(), 1);
    assert!(unsaved.intent.is_some());
    assert!(!unsaved.outbox.contains(&hash));
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
fn automatic_frontier_folding_uses_durable_intents_before_the_requested_edit() {
    use crate::core::sync::{DeviceSecret, PreparedChange};
    use ed25519_dalek::SigningKey;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wide.lbx");
    let mut vault = profile_vault(&path, "Owner");
    let mut members = vec![(vault.device_certificate().unwrap(), Role::Owner)];
    let mut devices = Vec::new();
    for seed in 1..=65 {
        let (device, certificate) =
            DeviceSecret::generate(&SigningKey::from_bytes(&[seed; 32])).unwrap();
        devices.push(device);
        members.push((certificate, Role::Editor));
    }
    vault.set_space_members(members).unwrap();
    let template = vault.sync_state().unwrap().clone();
    let mut objects = Vec::new();
    for device in &devices {
        let mut branch = template.clone();
        branch.intent = Some(
            Machine::prepare(
                &branch,
                device,
                Transaction {
                    mutations: vec![],
                    rebased_from: None,
                },
            )
            .unwrap(),
        );
        let hash = Machine::complete_intent(&mut branch, device).unwrap();
        objects.push((hash, branch.objects[&hash].0.clone()));
    }
    vault
        .receive_sync(Download {
            objects,
            incomplete: false,
        })
        .unwrap();
    let before = vault.sync_replay().unwrap().unwrap();
    assert_eq!(before.frontier.len(), 65);
    let object_count = vault.sync_state().unwrap().objects.len();
    let desired = entry("Saved after folding");
    vault.entries.push(desired.clone());
    std::fs::create_dir(sibling(&path, "tmp")).unwrap();
    assert!(vault.save_changes().is_err());
    let pending = vault.sync_state().unwrap();
    assert_eq!(pending.objects.len(), object_count);
    assert!(matches!(pending.intent, Some(PreparedChange::Change(_))));
    // The blocked intent is the empty merge, never a premature user mutation.
    let mut recovered = pending.clone();
    let device = vault.profile.as_ref().unwrap().sync_secret();
    let expected_merge = Machine::complete_intent(&mut recovered, &device).unwrap();
    std::fs::remove_dir(sibling(&path, "tmp")).unwrap();
    vault.recover_sync().unwrap();
    assert_eq!(
        vault.sync_state().unwrap().objects[&expected_merge],
        recovered.objects[&expected_merge]
    );
    assert!(vault.entries.is_empty());
    assert_eq!(vault.sync_replay().unwrap().unwrap().frontier.len(), 2);
    vault.entries.push(desired);
    vault.save_changes().unwrap();
    assert_eq!(vault.sync_state().unwrap().objects.len(), object_count + 2);
    assert_eq!(vault.sync_replay().unwrap().unwrap().frontier.len(), 1);
    drop(vault);
    let reopened = Vault::open(&path, &SecretString::from("master".to_owned())).unwrap();
    assert_eq!(reopened.entries[0].title, "Saved after folding");
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
fn bootstrap_activation_preserves_records_and_user_but_creates_a_fresh_signer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.lbx");
    let mut vault = bootstrap_vault(&path);
    vault.entries.push(entry("Existing local secret"));
    vault.save().unwrap();
    let identity = vault.profile.as_ref().unwrap().identity.device_id;
    let user = vault.profile.as_ref().unwrap().identity.user_id;
    assert!(matches!(
        vault.activate_sync(&SecretString::from("wrong".to_owned())),
        Err(VaultError::RecoveryFailed)
    ));
    assert!(vault.space_descriptor().is_none());
    assert_eq!(vault.entries[0].title, "Existing local secret");
    vault
        .activate_sync(&SecretString::from("recovery password".to_owned()))
        .unwrap();
    assert_ne!(vault.device_certificate().unwrap().body.device.0, identity);
    assert_eq!(vault.device_certificate().unwrap().body.user.0, user);
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
    let mut vault = bootstrap_vault(&path);
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

#[test]
fn incomplete_change_commit_failure_rolls_back_the_candidate_journal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.lbx");
    let mut vault = profile_vault(&path, "Alice");
    vault.entries.push(entry("Readable baseline"));
    vault.save_changes().unwrap();
    let profile = vault.profile.as_ref().unwrap();
    let device = profile.sync_secret();
    let mut remote = profile.sync.as_ref().unwrap().clone();
    let mut latest = vault.entries.clone();
    latest[0].title = "Missing parent".into();
    let before = Machine::replay(&remote, &device).unwrap().projection;
    remote.intent = Some(
        Machine::prepare(
            &remote,
            &device,
            Transaction::diff(&before, &latest, &[]).unwrap(),
        )
        .unwrap(),
    );
    Machine::complete_intent(&mut remote, &device).unwrap();
    latest[0].title = "Delayed child".into();
    let before = Machine::replay(&remote, &device).unwrap().projection;
    remote.intent = Some(
        Machine::prepare(
            &remote,
            &device,
            Transaction::diff(&before, &latest, &[]).unwrap(),
        )
        .unwrap(),
    );
    let child = Machine::complete_intent(&mut remote, &device).unwrap();
    let previous = crate::core::sync::canonical(vault.sync_state().unwrap()).unwrap();
    let file = std::fs::read(&path).unwrap();
    std::fs::create_dir(sibling(&path, "tmp")).unwrap();
    let partial = || Download {
        objects: vec![(child, remote.objects[&child].0.clone())],
        incomplete: true,
    };
    assert!(vault.receive_sync(partial()).is_err());
    assert_eq!(
        crate::core::sync::canonical(vault.sync_state().unwrap()).unwrap(),
        previous
    );
    assert_eq!(std::fs::read(&path).unwrap(), file);
    assert_eq!(vault.entries[0].title, "Readable baseline");
    std::fs::remove_dir(sibling(&path, "tmp")).unwrap();
    vault.receive_sync(partial()).unwrap();
    assert!(vault.sync_state().unwrap().objects.contains_key(&child));
    assert_eq!(vault.entries[0].title, "Readable baseline");
    vault
        .receive_sync(Download {
            objects: remote
                .objects
                .into_iter()
                .map(|(hash, bytes)| (hash, bytes.0))
                .collect(),
            incomplete: false,
        })
        .unwrap();
    assert_eq!(vault.entries[0].title, "Delayed child");
}

#[test]
fn missing_genesis_commit_failure_restores_the_empty_join_journal() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = profile_vault(&directory.path().join("owner.lbx"), "Alice");
    let path = directory.path().join("member.lbx");
    let mut member = profile_vault(&path, "Bob");
    owner
        .set_space_members(vec![
            (owner.device_certificate().unwrap(), Role::Owner),
            (member.device_certificate().unwrap(), Role::Editor),
        ])
        .unwrap();
    let state = owner.sync_state().unwrap();
    let successor = state
        .objects
        .iter()
        .find(|(hash, _)| **hash != state.genesis)
        .map(|(hash, bytes)| (*hash, bytes.0.clone()))
        .unwrap();
    member
        .join_space(owner.space_descriptor().unwrap())
        .unwrap();
    let previous = crate::core::sync::canonical(member.sync_state().unwrap()).unwrap();
    std::fs::create_dir(sibling(&path, "tmp")).unwrap();
    assert!(
        member
            .receive_sync(Download {
                objects: vec![successor],
                incomplete: true
            })
            .is_err()
    );
    assert_eq!(
        crate::core::sync::canonical(member.sync_state().unwrap()).unwrap(),
        previous
    );
    assert!(member.entries.is_empty());
    std::fs::remove_dir(sibling(&path, "tmp")).unwrap();
    member.receive_sync(delivery(&owner)).unwrap();
    assert!(member.sync_replay().unwrap().unwrap().ready);
}

#[derive(Default)]
struct ExchangeProbe {
    reads: usize,
    writes: usize,
}

impl crate::infra::sync::Storage for ExchangeProbe {
    fn list(
        &mut self,
        _: crate::core::sync::Hash,
        _: Option<&str>,
    ) -> crate::infra::sync::Result<crate::infra::sync::Page> {
        self.reads += 1;
        Ok(crate::infra::sync::Page {
            objects: vec![],
            next: None,
        })
    }
    fn get(
        &mut self,
        _: crate::core::sync::Hash,
        _: crate::core::sync::Hash,
    ) -> crate::infra::sync::Result<Vec<u8>> {
        Err(crate::infra::sync::TransportError::NotFound)
    }
    fn put(
        &mut self,
        _: crate::core::sync::Hash,
        _: crate::core::sync::Hash,
        _: &[u8],
    ) -> crate::infra::sync::Result<()> {
        self.writes += 1;
        Ok(())
    }
}

#[test]
fn failed_revocation_commit_blocks_old_epoch_publication_until_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = profile_vault(&directory.path().join("owner.lbx"), "Alice");
    let path = directory.path().join("member.lbx");
    let mut member = profile_vault(&path, "Bob");
    owner
        .set_space_members(vec![
            (owner.device_certificate().unwrap(), Role::Owner),
            (member.device_certificate().unwrap(), Role::Editor),
        ])
        .unwrap();
    member
        .join_space(owner.space_descriptor().unwrap())
        .unwrap();
    member.receive_sync(delivery(&owner)).unwrap();
    member.entries.push(entry("Late offline edit"));
    member.save_changes().unwrap();
    let old_outbox = member.sync_state().unwrap().outbox.clone();
    owner
        .set_space_members(vec![(owner.device_certificate().unwrap(), Role::Owner)])
        .unwrap();
    std::fs::create_dir(sibling(&path, "tmp")).unwrap();
    assert!(member.receive_sync(delivery(&owner)).is_err());
    assert!(member.has_pending_exchange());
    assert_eq!(member.sync_state().unwrap().outbox, old_outbox);
    assert_eq!(member.entries[0].title, "Late offline edit");
    assert!(matches!(
        member.sync_replay(),
        Err(VaultError::PendingSyncCommit)
    ));
    assert!(matches!(
        member.save_changes(),
        Err(VaultError::PendingSyncCommit)
    ));
    assert!(matches!(member.save(), Err(VaultError::PendingSyncCommit)));
    let mut provider = ExchangeProbe::default();
    assert!(member.synchronize(&mut provider).is_err());
    assert_eq!((provider.reads, provider.writes), (0, 0));
    std::fs::remove_dir(sibling(&path, "tmp")).unwrap();
    member.recover_exchange().unwrap();
    assert!(!member.has_pending_exchange());
    assert!(member.sync_replay().unwrap().unwrap().role.is_none());
    assert!(member.synchronize(&mut provider).is_err());
    assert_eq!(provider.writes, 0);
    assert_eq!(member.entries[0].title, "Late offline edit");
}

#[test]
fn failed_acknowledgement_restores_receipts_and_exchange_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.lbx");
    let mut vault = profile_vault(&path, "Alice");
    let outbox = vault.sync_state().unwrap().outbox.clone();
    let hashes: Vec<_> = outbox.iter().copied().collect();
    std::fs::create_dir(sibling(&path, "tmp")).unwrap();
    assert!(vault.acknowledge_sync(&hashes, true).is_err());
    assert_eq!(vault.sync_state().unwrap().outbox, outbox);
    assert!(vault.sync_state().unwrap().last_exchange.is_none());
    std::fs::remove_dir(sibling(&path, "tmp")).unwrap();
    vault.acknowledge_sync(&hashes, true).unwrap();
    assert!(vault.sync_state().unwrap().outbox.is_empty());
    assert!(vault.sync_state().unwrap().last_exchange.is_some());
}
