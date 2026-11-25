use super::*;
use crate::{
    records::entry::{Entry, Totp, TotpAlgorithm},
    sync::protocol::{Error, MAX_MUTATIONS, Role},
    vault::{KdfParams, ProfileStore, sharing_tests, witness::MemoryWitness},
};
use secrecy::SecretString;
use std::{fs, path::Path, sync::Arc};

fn local(path: &Path) -> Vault {
    Vault::create(
        path,
        &SecretString::from("fictional master password".to_owned()),
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
    )
    .unwrap()
}

fn entry(title: &str, parent: Option<&Folder>) -> Entry {
    let mut entry = Entry::new();
    entry.title = title.to_owned();
    entry.password = "fictional entry password".into();
    entry.folder_id = parent.map(|folder| folder.id.clone());
    entry
}

#[test]
fn nested_folders_and_individual_records_move_without_collapsing_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = local(&dir.path().join("source.lbx"));
    let mut destination = local(&dir.path().join("destination.lbx"));
    let root = Folder::new("Accounts".into(), None);
    let nested = Folder::new("Work".into(), Some(root.id.clone()));
    let empty = Folder::new("Empty".into(), Some(nested.id.clone()));
    let mut login = entry("Portal", Some(&nested));
    login.notes = "Fictional notes".into();
    login.totp = Some(Totp {
        secret: "JBSWY3DPEHPK3PXP".into(),
        algorithm: TotpAlgorithm::Sha1,
        digits: 6,
        period: 30,
    });
    login
        .add_attachment("test.bin".into(), vec![1, 2, 3])
        .unwrap();
    let mut duplicate = login.clone();
    duplicate.id = crate::records::id::new();
    let other_folder = Folder::new("Other".into(), None);
    let individual = entry("Selected", Some(&other_folder));
    let untouched = entry("Unselected", Some(&other_folder));
    source
        .append_records(
            &[root.clone(), nested.clone(), empty, other_folder.clone()],
            &[
                login.clone(),
                duplicate,
                individual.clone(),
                untouched.clone(),
            ],
        )
        .unwrap();
    let existing = Folder::new("accounts".into(), None);
    destination.save_folder(existing.clone()).unwrap();

    let selection = Selection {
        entries: BTreeSet::from([login.id.clone(), individual.id.clone()]),
        folders: BTreeSet::from([root.id.clone(), nested.id.clone()]),
        ..Selection::default()
    };
    let mut transfer = Move::new(&source, &destination, &selection).unwrap();
    assert_eq!(transfer.counts(), (3, 3));
    transfer.execute(&mut source, &mut destination).unwrap();

    assert_eq!(source.entries(), &[untouched]);
    assert_eq!(source.folders(), &[other_folder]);
    assert!(
        source
            .record_history()
            .unwrap()
            .iter()
            .any(|change| { change.id == login.id && change.action == "Delete" })
    );
    assert!(destination.folders().contains(&existing));
    let moved_root = destination
        .folders()
        .iter()
        .find(|f| f.name == "Accounts (2)")
        .unwrap();
    let moved_nested = destination
        .folders()
        .iter()
        .find(|f| f.name == "Work")
        .unwrap();
    let moved_empty = destination
        .folders()
        .iter()
        .find(|f| f.name == "Empty")
        .unwrap();
    assert_eq!(moved_nested.parent_id.as_ref(), Some(&moved_root.id));
    assert_eq!(moved_empty.parent_id.as_ref(), Some(&moved_nested.id));

    let records = Vault::read_records(
        destination.path(),
        &SecretString::from("fictional master password".to_owned()),
    )
    .unwrap();
    let logins: Vec<_> = records
        .entries
        .iter()
        .filter(|e| e.title == "Portal")
        .collect();
    assert_eq!(logins.len(), 2);
    assert_ne!(logins[0].id, logins[1].id);
    for actual in logins {
        assert_ne!(actual.id, login.id);
        let mut expected = login.clone();
        expected.id.clone_from(&actual.id);
        expected.folder_id = Some(moved_nested.id.clone());
        assert_eq!(actual, &expected);
    }
    assert!(
        records
            .entries
            .iter()
            .any(|e| e.title == "Selected" && e.folder_id.is_none())
    );
}

#[test]
fn failed_destination_keeps_source_and_failed_cleanup_reuses_destination_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = local(&dir.path().join("source.lbx"));
    let mut destination = local(&dir.path().join("destination.lbx"));
    let record = entry("Portal", None);
    source.upsert_entry(record.clone()).unwrap();
    let selection = Selection {
        entries: BTreeSet::from([record.id.clone()]),
        ..Selection::default()
    };
    let mut transfer = Move::new(&source, &destination, &selection).unwrap();
    let destination_block = destination.path().with_extension("lbx.tmp");
    fs::create_dir(&destination_block).unwrap();
    assert!(transfer.execute(&mut source, &mut destination).is_err());
    assert_eq!(source.entries(), std::slice::from_ref(&record));
    assert!(destination.entries().is_empty());

    fs::remove_dir(destination_block).unwrap();
    let source_block = source.path().with_extension("lbx.tmp");
    fs::create_dir(&source_block).unwrap();
    assert!(transfer.execute(&mut source, &mut destination).is_err());
    assert_eq!(source.entries(), &[record]);
    let copied = destination.entries()[0].clone();
    fs::remove_dir(source_block).unwrap();

    let mut edited = copied.clone();
    edited.password = "changed in destination".into();
    destination.upsert_entry(edited).unwrap();
    assert!(matches!(
        transfer.execute(&mut source, &mut destination),
        Err(MoveError::DestinationChanged)
    ));
    assert_eq!(source.entries().len(), 1);
    destination.upsert_entry(copied.clone()).unwrap();
    transfer.execute(&mut source, &mut destination).unwrap();
    assert!(source.entries().is_empty());
    assert_eq!(destination.entries(), &[copied]);
}

#[test]
fn moves_a_selection_larger_than_one_shared_transaction_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut local_vault = local(&dir.path().join("local.lbx"));
    let shared_path = dir.path().join("shared.lbx");
    let mut shared = sharing_tests::create(&store, &shared_path);
    let folder = Folder::new("Batch".into(), None);
    let entries: Vec<_> = (0..=MAX_MUTATIONS)
        .map(|i| entry(&format!("Login {i}"), Some(&folder)))
        .collect();
    local_vault
        .append_records(std::slice::from_ref(&folder), &entries)
        .unwrap();
    let selection = Selection {
        folders: BTreeSet::from([folder.id.clone()]),
        entire_vault: true,
        ..Selection::default()
    };

    Move::new(&local_vault, &shared, &selection)
        .unwrap()
        .execute(&mut local_vault, &mut shared)
        .unwrap();
    assert!(local_vault.entries().is_empty());
    assert_eq!(shared.entries().len(), entries.len());
    assert!(shared.sync_state().unwrap().outbox.len() > 2);
    drop(shared);

    let mut shared = store
        .open(&shared_path, &SecretString::from("test master".to_owned()))
        .unwrap();
    assert_eq!(shared.entries().len(), entries.len());
    let selection = Selection {
        folders: BTreeSet::from([shared.folders()[0].id.clone()]),
        ..Selection::default()
    };
    Move::new(&shared, &local_vault, &selection)
        .unwrap()
        .execute(&mut shared, &mut local_vault)
        .unwrap();
    assert!(shared.entries().is_empty());
    assert!(shared.folders().is_empty());
    assert_eq!(local_vault.entries().len(), entries.len());
    assert_eq!(local_vault.folders().len(), 1);
    assert!(
        shared
            .record_history()
            .unwrap()
            .iter()
            .any(|change| change.action == "Delete")
    );
}

#[test]
fn rejects_read_only_destination_and_two_profiles_of_the_same_space() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let mut source = local(&dir.path().join("local.lbx"));
    let mut owner = sharing_tests::create(&store, &dir.path().join("owner.lbx"));
    let mut reader = sharing_tests::create(&store, &dir.path().join("reader.lbx"));
    let record = entry("Portal", None);
    source.upsert_entry(record.clone()).unwrap();
    let selection = Selection {
        entries: BTreeSet::from([record.id.clone()]),
        ..Selection::default()
    };
    sharing_tests::share(&mut owner, &mut reader, Role::Reader);
    let objects = owner
        .sync_state()
        .unwrap()
        .objects
        .iter()
        .map(|(hash, blob)| (*hash, blob.0.clone()))
        .collect();
    reader
        .receive_sync(crate::sync::transport::Download {
            objects,
            ..Default::default()
        })
        .unwrap();

    assert!(matches!(
        Move::new(&owner, &reader, &selection),
        Err(MoveError::SameSpace)
    ));
    let mut transfer = Move::new(&source, &reader, &selection).unwrap();
    assert!(matches!(
        transfer.execute(&mut source, &mut reader),
        Err(MoveError::Vault(VaultError::Sync(Error::InsufficientRole)))
    ));
    assert_eq!(source.entries(), &[record]);
    assert!(reader.entries().is_empty());
}
