#[path = "support/sync_scenario.rs"]
mod scenario;

use lockbox::{
    core::sync::Hash,
    infra::{
        sync::{Page, Storage, TransportError},
        vault::{KdfParams, ProfileStore, ProviderConfiguration, SharedProfile},
        witness::MemoryWitness,
    },
};
use secrecy::SecretString;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

type Objects = Arc<Mutex<BTreeMap<(Hash, Hash), Vec<u8>>>>;

struct Memory(Objects);

impl Storage for Memory {
    fn list(&mut self, namespace: Hash, _: Option<&str>) -> Result<Page, TransportError> {
        Ok(Page {
            objects: self
                .0
                .lock()
                .unwrap()
                .keys()
                .rev()
                .filter(|(ns, _)| *ns == namespace)
                .map(|(_, hash)| *hash)
                .collect(),
            next: None,
        })
    }

    fn get(&mut self, namespace: Hash, hash: Hash) -> Result<Vec<u8>, TransportError> {
        self.0
            .lock()
            .unwrap()
            .get(&(namespace, hash))
            .cloned()
            .ok_or(TransportError::NotFound)
    }

    fn put(&mut self, namespace: Hash, hash: Hash, bytes: &[u8]) -> Result<(), TransportError> {
        self.0
            .lock()
            .unwrap()
            .insert((namespace, hash), bytes.to_vec());
        Ok(())
    }
}

#[test]
fn release_scenario_survives_conflicts_role_changes_revocation_and_restart() -> scenario::Result<()>
{
    let directory = tempfile::tempdir()?;
    let profiles = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let create = |name: &str| {
        profiles.create_shared(
            directory.path().join(format!("{name}.lbx")),
            &SecretString::from("disposable master password".to_owned()),
            &SecretString::from("disposable recovery password".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: name.into(),
                space_name: "Probe".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "fixture-bucket".into(),
                    access_key: format!("{name}-access"),
                    secret_key: "fixture-secret".into(),
                },
            },
        )
    };
    let mut owner = create("Owner")?;
    let mut editor = create("Editor")?;
    let objects = Objects::default();
    scenario::exercise(
        &mut owner,
        &mut editor,
        &mut Memory(objects.clone()),
        &mut Memory(objects),
    )?;
    let owner_path = owner.path().to_owned();
    let editor_path = editor.path().to_owned();
    let owner_key = owner.export_key();
    let editor_key = editor.export_key();
    drop(owner);
    drop(editor);
    scenario::check_reopened(
        &profiles.open_with_key(owner_path, &owner_key)?,
        &profiles.open_with_key(editor_path, &editor_key)?,
    )
}
