use crate::{
    records::{entry::Entry, folder::Folder},
    sync::{
        protocol::{Role, canonical},
        transport::Download,
    },
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
    assert!(vault.sync_replay_state().unwrap().is_none());
    assert!(vault.sync_replay().unwrap().is_none());
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
    let full = vault.record_history().unwrap();
    for record in [
        Some(first.id.as_str()),
        Some(second.id.as_str()),
        Some(live.id.as_str()),
        None,
        Some("missing"),
    ] {
        let expected: Vec<_> = full
            .iter()
            .filter(|change| Some(change.id.as_str()) == record)
            .collect();
        assert_eq!(
            canonical(&vault.record_history_for(record).unwrap()).unwrap(),
            canonical(&expected).unwrap()
        );
    }

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
    let mut deleted = entry("2", "Deleted record");
    deleted
        .add_attachment("trash.bin".into(), vec![0, 255, 42])
        .unwrap();
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

    let history = canonical(&owner.record_history().unwrap()).unwrap();
    let current = owner.sync_replay_state().unwrap().unwrap();
    assert!(current.history.is_empty());
    let replay = owner.sync_replay().unwrap().unwrap();
    assert_eq!(
        canonical(&current.projection).unwrap(),
        canonical(&replay.projection).unwrap()
    );
    assert_eq!(
        (
            current.membership,
            current.ready,
            current.role,
            current.history_verified
        ),
        (
            replay.membership,
            replay.ready,
            replay.role,
            replay.history_verified
        ),
    );
    assert!(!replay.history.is_empty());
    assert_eq!(canonical(&replay.history).unwrap(), history);
    for record in [
        Some(conflicted.id.as_str()),
        Some(deleted.id.as_str()),
        None,
        Some("missing"),
    ] {
        let expected: Vec<_> = replay
            .history
            .iter()
            .filter(|change| Some(change.id.as_str()) == record)
            .collect();
        assert_eq!(
            canonical(&owner.record_history_for(record).unwrap()).unwrap(),
            canonical(&expected).unwrap()
        );
    }
    assert_eq!(
        canonical(&owner.record_history().unwrap()).unwrap(),
        history
    );
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

#[test]
fn batch_history_orders_changed_ids_and_distinguishes_put_delete_and_restore_on_retry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("batch.lbx");
    let password = SecretString::from("test master".to_owned());
    let mut vault = Vault::create(
        &path,
        &password,
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
    )
    .unwrap();
    let restored = entry("1", "Restore");
    let mut original = entry("2", "Update");
    original
        .add_attachment("keep.bin".into(), vec![0, 255, 42])
        .unwrap();
    let deleted = entry("3", "Delete");
    let unchanged = entry("4", "Unchanged");
    vault
        .append_records(
            &[],
            &[
                unchanged.clone(),
                deleted.clone(),
                original.clone(),
                restored.clone(),
            ],
        )
        .unwrap();
    vault.delete_entry(&restored.id).unwrap();
    let count = vault.record_history().unwrap().len();
    let previous = serde_json::to_vec(&vault.record_history().unwrap()).unwrap();
    let mut updated = original.clone();
    updated.password = "updated fictional password".into();
    let created = entry("5", "Create");
    let desired = vec![
        unchanged,
        created.clone(),
        updated.clone(),
        restored.clone(),
    ];
    *vault.test_entries_mut() = desired.clone();
    let before = std::fs::read(&path).unwrap();
    let temporary = path.with_extension("lbx.tmp");
    std::fs::create_dir(&temporary).unwrap();

    assert!(vault.save_changes().is_err());
    assert_eq!(vault.record_history().unwrap().len(), count);
    assert_eq!(
        serde_json::to_vec(&vault.record_history().unwrap()).unwrap(),
        previous
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir(temporary).unwrap();
    vault.save_changes().unwrap();
    let history = vault.record_history().unwrap();
    assert_eq!(serde_json::to_vec(&history[..count]).unwrap(), previous);
    let changes = &history[count..];
    assert_eq!(changes.len(), 4);
    assert_eq!(
        changes
            .iter()
            .map(|change| (change.id.as_str(), change.action.as_str()))
            .collect::<Vec<_>>(),
        [
            (restored.id.as_str(), "Restore"),
            (updated.id.as_str(), "Put"),
            (deleted.id.as_str(), "Delete"),
            (created.id.as_str(), "Put"),
        ]
    );
    assert!(changes[0].before.is_empty());
    assert_eq!(changes[0].after.as_ref(), Some(&restored));
    assert_eq!(changes[1].before, [original]);
    assert_eq!(changes[1].after.as_ref(), Some(&updated));
    assert_eq!(changes[2].before, [deleted]);
    assert!(changes[2].after.is_none());
    assert!(changes[3].before.is_empty());
    assert_eq!(changes[3].after.as_ref(), Some(&created));

    drop(vault);
    let reopened = Vault::open(&path, &password).unwrap();
    assert_eq!(reopened.entries(), desired);
    assert_eq!(
        serde_json::to_vec(&reopened.record_history().unwrap()).unwrap(),
        serde_json::to_vec(&history).unwrap()
    );
    reopened.save().unwrap();
    assert_eq!(reopened.record_history().unwrap().len(), count + 4);
}

#[test]
fn history_chunks_serialize_as_the_same_flat_json_array() {
    use crate::{records::history::RecordChange, vault::format::ContentsRef};
    let changes: Vec<_> = (0..3)
        .map(|index| {
            let mut value = entry(&index.to_string(), "Предыдущая запись");
            value
                .add_attachment("payload.bin".into(), vec![0, 255, index])
                .unwrap();
            let mut change = RecordChange::default();
            change.id = value.id.clone();
            change.revision = format!("revision-{index}");
            change.before = vec![value.clone()];
            change.after = Some(value);
            change
        })
        .collect();
    for history in [&changes[..0], changes.as_slice()] {
        let mut expected = b"{\"entries\":[],\"history\":".to_vec();
        expected.extend(serde_json::to_vec(history).unwrap());
        expected.push(b'}');
        for split in 0..=history.len() {
            let contents = ContentsRef {
                entries: &[],
                history: [&history[..split], &history[split..]],
                folders: &[],
                profile: None,
                generation: None,
            };
            assert_eq!(serde_json::to_vec(&contents).unwrap(), expected);
        }
    }
}

#[test]
fn legacy_duplicate_ids_keep_first_occurrence_history_semantics() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(
        directory.path().join("legacy.lbx"),
        &SecretString::from("test master".to_owned()),
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
    )
    .unwrap();
    let first = entry("1", "First occurrence");
    let mut second = entry("1", "Second occurrence");
    vault
        .append_records(&[], &[first.clone(), second.clone()])
        .unwrap();
    let history = vault.record_history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].after.as_ref(), Some(&first));

    second.title = "Changed second occurrence".into();
    vault.test_entries_mut()[1] = second.clone();
    vault.save_changes().unwrap();
    assert_eq!(vault.record_history().unwrap().len(), 1);

    vault.test_entries_mut().remove(0);
    vault.save_changes().unwrap();
    let history = vault.record_history().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].before, [first]);
    assert_eq!(history[1].after.as_ref(), Some(&second));
}
