use crate::{
    records::{entry::Entry, folder::Folder},
    sync::{protocol::Role, transport::Download},
    vault::{KdfParams, ProfileStore, Vault, sharing_tests, witness::MemoryWitness},
};
use secrecy::SecretString;
use std::sync::Arc;

fn entry(id: &str, title: &str) -> Entry {
    let mut entry = Entry::new();
    entry.id = id.repeat(32);
    entry.title = title.into();
    entry.password = "fictional password".into();
    entry
}

fn deliver(from: &Vault, to: &mut Vault) {
    to.receive_sync(Download {
        objects: from
            .sync_state()
            .unwrap()
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
        incomplete: false,
    })
    .unwrap();
}

#[test]
fn local_trash_keeps_latest_deleted_values_sorted_by_id_without_live_records() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(
        directory.path().join("history.lbx"),
        &SecretString::from("test master".to_owned()),
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
    )
    .unwrap();
    let mut first = entry("2", "First deleted");
    let second = entry("1", "Second deleted");
    let live = entry("3", "Live record");
    vault
        .append_records(&[], &[first.clone(), second.clone(), live.clone()])
        .unwrap();
    vault.delete_entry(&first.id).unwrap();
    vault.restore_record(&first.id).unwrap();
    first.password = "latest fictional password".into();
    first
        .add_attachment("latest.bin".into(), vec![3, 2, 1])
        .unwrap();
    vault.upsert_entry(first.clone()).unwrap();
    vault.delete_entry(&first.id).unwrap();
    vault.delete_entry(&second.id).unwrap();
    let revisions = vault.record_history().unwrap().len();

    assert_eq!(vault.trash().unwrap(), [second, first.clone()]);
    assert_eq!(vault.record_history().unwrap().len(), revisions);
    assert_eq!(vault.entries(), std::slice::from_ref(&live));
    vault.restore_record(&first.id).unwrap();
    assert_eq!(vault.trash().unwrap().len(), 1);
    assert_eq!(vault.entries(), [live, first]);
}

#[test]
fn shared_trash_excludes_delete_edit_conflicts_and_folder_tombstones_with_same_id() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut owner = sharing_tests::create(&store, &directory.path().join("owner.lbx"));
    let mut writer = sharing_tests::create(&store, &directory.path().join("writer.lbx"));
    sharing_tests::share(&mut owner, &mut writer, Role::Editor);
    let mut conflicted = entry("1", "Concurrent edit");
    let deleted = entry("2", "Deleted record");
    let folder = Folder {
        id: conflicted.id.clone(),
        name: "Same ID as record".into(),
        parent_id: None,
    };
    owner
        .append_records(
            std::slice::from_ref(&folder),
            &[conflicted.clone(), deleted.clone()],
        )
        .unwrap();
    deliver(&owner, &mut writer);
    owner.delete_entry(&conflicted.id).unwrap();
    conflicted.password = "concurrent fictional password".into();
    writer.upsert_entry(conflicted.clone()).unwrap();
    owner.delete_entry(&deleted.id).unwrap();
    owner.delete_folder(&folder.id).unwrap();
    deliver(&writer, &mut owner);

    let replay = owner.sync_replay().unwrap().unwrap();
    assert_eq!(replay.projection.conflicts().count(), 1);
    assert!(owner.entries().is_empty());
    assert_eq!(owner.trash().unwrap(), std::slice::from_ref(&deleted));
    assert!(owner.restore_record(&conflicted.id).is_err());
    owner.restore_record(&deleted.id).unwrap();
    assert!(owner.trash().unwrap().is_empty());
    assert_eq!(owner.entries(), [deleted]);
    assert_eq!(
        owner
            .sync_replay()
            .unwrap()
            .unwrap()
            .projection
            .conflicts()
            .count(),
        1
    );
}
