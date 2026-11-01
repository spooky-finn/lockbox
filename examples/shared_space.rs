//! Offline, executable multiuser demonstration through the same Storage trait as
//! Storj. All passwords below are disposable fixtures inside a temporary folder.
use lockbox::{
    core::{
        entry::Entry,
        sync::{Hash, Role, Value},
    },
    infra::{
        sync::{Page, Storage, TransportError},
        vault::{KdfParams, ProfileStore, ProviderConfiguration, SharedProfile},
        witness::MemoryWitness,
    },
};
use secrecy::SecretString;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default)]
struct Memory {
    objects: BTreeMap<(Hash, Hash), Vec<u8>>,
    lose_response: bool,
}
impl Storage for Memory {
    fn list(&mut self, namespace: Hash, _: Option<&str>) -> Result<Page, TransportError> {
        Ok(Page {
            objects: self
                .objects
                .keys()
                .rev()
                .filter(|(ns, _)| *ns == namespace)
                .map(|(_, hash)| *hash)
                .collect(),
            next: None,
        })
    }
    fn get(&mut self, namespace: Hash, hash: Hash) -> Result<Vec<u8>, TransportError> {
        self.objects
            .get(&(namespace, hash))
            .cloned()
            .ok_or(TransportError::NotFound)
    }
    fn put(&mut self, namespace: Hash, hash: Hash, bytes: &[u8]) -> Result<(), TransportError> {
        self.objects.insert((namespace, hash), bytes.to_vec());
        if std::mem::take(&mut self.lose_response) {
            Err(TransportError::Unavailable)
        } else {
            Ok(())
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let profiles = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let create = |name: &str| {
        profiles.create_shared(
            directory.path().join(format!("{name}.lbx")),
            &SecretString::from("disposable master password".to_owned()),
            &SecretString::from("independent recovery password".to_owned()),
            KdfParams::default(),
            SharedProfile {
                username: name.into(),
                space_name: "Example team".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "example-lockbox".into(),
                    access_key: "unused-fixture".into(),
                    secret_key: "unused-fixture".into(),
                },
            },
        )
    };
    let mut alice = create("Alice")?;
    let mut bob = create("Bob")?;
    let mut entry = Entry::new();
    entry.title = "Shared account".into();
    entry.password = "initial fixture".into();
    alice.entries.push(entry);
    alice.save_changes()?;
    // Real callers compare complete user fingerprints through an authenticated
    // channel before supplying certificates and this descriptor.
    alice.set_space_members(vec![
        (alice.device_certificate().unwrap(), Role::Owner),
        (bob.device_certificate().unwrap(), Role::Editor),
    ])?;
    bob.join_space(alice.space_descriptor().unwrap())?;
    let mut storage = Memory::default();
    alice.synchronize(&mut storage)?;
    bob.synchronize(&mut storage)?;
    alice.entries[0].password = "Alice's fixture".into();
    alice.save_changes()?;
    bob.entries[0].password = "Bob's fixture".into();
    bob.save_changes()?;
    // The provider stores the object but loses its response. Retry uses the same
    // durable signed bytes instead of creating another logical edit.
    storage.lose_response = true;
    assert!(alice.synchronize(&mut storage).is_err());
    alice.synchronize(&mut storage)?;
    bob.synchronize(&mut storage)?;
    alice.synchronize(&mut storage)?;
    let replay = alice.sync_replay()?.unwrap();
    let conflict = replay.projection.conflicts().next().unwrap();
    assert_eq!(conflict.versions.len(), 2);
    let chosen = conflict.versions.iter().find(|version| matches!(&version.value,Some(Value::Entry(entry)) if entry.password == "Bob's fixture")).unwrap();
    alice.resolve_sync(conflict.entity.clone(), chosen.revision)?;
    alice.synchronize(&mut storage)?;
    bob.synchronize(&mut storage)?;
    assert_eq!(alice.entries, bob.entries);
    assert_eq!(alice.entries[0].password, "Bob's fixture");
    println!(
        "Two independent profiles converged. Concurrent versions survived, retries were idempotent, and explicit resolution synchronized."
    );
    Ok(())
}
