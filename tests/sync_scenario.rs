#[path = "support/sync_scenario.rs"]
mod scenario;

use lockbox::{
    sync::{
        protocol::Hash,
        transport::{Page, Storage, TransportError},
    },
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
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
fn owner_recovery_uses_current_reader_content_with_an_old_read_only_authority_backup()
-> scenario::Result<()> {
    use lockbox::records::entry::Entry;
    use lockbox::sync::protocol::Role;
    use lockbox::vault::ControllerRecovery;
    use lockbox::vault::Vault;
    let directory = tempfile::tempdir()?;
    let profiles = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let password = SecretString::from("disposable master".to_owned());
    let recovery_password = SecretString::from("disposable root recovery".to_owned());
    let kdf = KdfParams {
        memory_kib: 8192,
        passes: 1,
        lanes: 1,
    };
    let configuration = |name: &str| SharedProfile {
        username: name.into(),
        space_name: "Recovery fixture".into(),
        provider: ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "fixture".into(),
            access_key: format!("{name}-fictional-access"),
            secret_key: format!("{name}-fictional-secret"),
        },
    };
    let mut owner = profiles.create_shared(
        directory.path().join("owner.lbx"),
        &password,
        &recovery_password,
        kdf,
        configuration("Owner"),
    )?;
    let mut reader = profiles.create_shared(
        directory.path().join("reader.lbx"),
        &password,
        &recovery_password,
        kdf,
        configuration("Reader"),
    )?;
    let mut record = Entry::new();
    record.title = "Old backup content".into();
    record.password = "fictional secret".into();
    owner.upsert_entry(record)?;
    let backup = directory.path().join("old-authority-backup.lbx");
    std::fs::copy(owner.path(), &backup)?;
    let offer = owner.issue_invitation(reader.device_certificate().unwrap(), Role::Reader, None)?;
    let response = reader.accept_invitation(offer, &owner.space_descriptor().unwrap())?;
    reader.join_invitation(owner.admit_invitation(response, 0)?)?;
    let mut entry = owner.entries()[0].clone();
    entry.title = "Latest content held by the reader".into();
    owner.upsert_entry(entry)?;
    let mut storage = Memory(Arc::new(Mutex::new(BTreeMap::new())));
    owner.synchronize(&mut storage)?;
    reader.synchronize(&mut storage)?;
    let anchor = reader.recovery_anchor()?;
    let old_device = owner.device_certificate().unwrap().body.device;
    drop(owner);
    let read_only_store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let authority: Vault = read_only_store.open(&backup, &password)?;
    assert!(authority.signing_error().is_some());
    assert_eq!(authority.entries()[0].title, "Old backup content");
    let mut recovered = profiles.recover_controller(
        directory.path().join("recovered.lbx"),
        &password,
        kdf,
        ControllerRecovery {
            authority: &authority,
            content: &reader,
            recovery_password: &recovery_password,
            anchor,
            configuration: configuration("Recovered owner"),
        },
    )?;
    assert_ne!(
        recovered.device_certificate().unwrap().body.device,
        old_device
    );
    assert_eq!(
        recovered.entries()[0].title,
        "Latest content held by the reader"
    );
    recovered.synchronize(&mut storage)?;
    reader.synchronize(&mut storage)?;
    assert_eq!(
        reader.space_controller()?,
        recovered.device_certificate().unwrap().body.device
    );
    assert_eq!(reader.sync_replay()?.unwrap().role, Some(Role::Reader));
    let mut entry = recovered.entries()[0].clone();
    entry.title = "Edit by the recovered controller".into();
    recovered.upsert_entry(entry)?;
    recovered.synchronize(&mut storage)?;
    reader.synchronize(&mut storage)?;
    assert_eq!(reader.entries(), recovered.entries());
    assert_eq!(authority.entries()[0].title, "Old backup content");
    Ok(())
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
