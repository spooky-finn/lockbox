use super::*;

fn entity(kind: EntityKind, id: u8) -> Entity {
    Entity {
        kind,
        id: format!("{id:032x}"),
    }
}

fn entry(id: u8, parent: Option<u8>) -> Entry {
    let mut entry = Entry::default();
    entry.id = format!("{id:032x}");
    entry.title = format!("Account {id}");
    entry.password = "fictional password".into();
    entry.folder_id = parent.map(|id| format!("{id:032x}"));
    entry
}

fn folder(id: u8, name: &str, parent: Option<u8>) -> Folder {
    Folder {
        id: format!("{id:032x}"),
        name: name.into(),
        parent_id: parent.map(|id| format!("{id:032x}")),
    }
}

fn version(revision: u8, value: Option<Value>) -> Version {
    Version {
        revision: Revision {
            change: Hash([revision; 32]),
            index: 0,
        },
        author: Hash([250; 32]),
        value,
    }
}

fn live(value: Value, revision: u8) -> Record {
    Record {
        entity: value.entity(),
        versions: vec![version(revision, Some(value))],
    }
}

fn conflict(first: Value, second: Value, revision: u8) -> Record {
    Record {
        entity: first.entity(),
        versions: vec![
            version(revision, Some(first)),
            version(revision + 1, Some(second)),
        ],
    }
}

fn deleted(kind: EntityKind, id: u8, revision: u8) -> Record {
    Record {
        entity: entity(kind, id),
        versions: vec![version(revision, None)],
    }
}

fn projection(mut records: Vec<Record>) -> Projection {
    records.sort_by(|a, b| a.entity.cmp(&b.entity));
    Projection { records }
}

#[test]
fn owned_views_keep_single_live_heads_and_preserve_decrypted_payloads() {
    let mut account = entry(1, Some(11));
    account
        .add_attachment("fictional.bin".into(), vec![0, 255, 42])
        .unwrap();
    let unfiled = entry(4, None);
    let work = folder(11, "Work", None);
    let personal = folder(14, "Personal", None);
    let expected_entries = vec![account.clone(), unfiled.clone()];
    let expected_folders = vec![work.clone(), personal.clone()];
    let projection = projection(vec![
        live(Value::Folder(personal), 10),
        live(Value::Entry(unfiled), 5),
        deleted(EntityKind::Entry, 2, 2),
        conflict(
            Value::Entry(entry(3, None)),
            Value::Entry(entry(3, Some(11))),
            3,
        ),
        live(Value::Entry(account), 1),
        live(Value::Folder(work), 6),
        conflict(
            Value::Folder(folder(12, "Before", None)),
            Value::Folder(folder(12, "After", None)),
            7,
        ),
        deleted(EntityKind::Folder, 13, 9),
    ]);
    projection.validate().unwrap();
    assert_eq!(projection.entries(), expected_entries);
    assert_eq!(projection.folders(), expected_folders);
    assert_eq!(projection.conflicts().count(), 2);

    let mut snapshot = projection.entries();
    snapshot[0].password = "changed snapshot".into();
    snapshot[0].attachments[0].data[0] = 7;
    assert_eq!(projection.entries(), expected_entries);
}

#[test]
fn structural_evidence_preserves_name_cycle_and_orphan_rules() {
    let folders = [
        folder(10, " Work ", None),
        folder(11, "work", None),
        folder(12, "Child", Some(10)),
        folder(20, "Cycle A", Some(21)),
        folder(21, "Cycle B", Some(20)),
        folder(30, "Orphan", Some(99)),
        folder(31, "Orphan child", Some(30)),
        folder(40, "Сервис", None),
        folder(41, "сервис", None),
    ];
    let entries = [
        entry(1, Some(10)),
        entry(2, Some(11)),
        entry(3, Some(12)),
        entry(4, Some(20)),
        entry(5, Some(30)),
        entry(6, Some(99)),
        entry(7, None),
        entry(8, Some(40)),
        entry(9, Some(41)),
        entry(15, Some(31)),
    ];
    let projection = projection(
        folders
            .into_iter()
            .map(Value::Folder)
            .chain(entries.into_iter().map(Value::Entry))
            .enumerate()
            .map(|(index, value)| live(value, index as u8 + 1))
            .collect(),
    );
    projection.validate().unwrap();
    let expected: BTreeSet<_> = [10, 11, 20, 21, 30, 31]
        .into_iter()
        .map(|id| entity(EntityKind::Folder, id))
        .chain(
            [1, 2, 4, 5, 6, 15]
                .into_iter()
                .map(|id| entity(EntityKind::Entry, id)),
        )
        .collect();
    assert_eq!(projection.structural_conflicts(), expected);
}

#[test]
fn deleted_or_conflicted_folders_are_missing_for_live_entry_evidence() {
    let projection = projection(vec![
        live(Value::Entry(entry(1, Some(10))), 1),
        live(Value::Entry(entry(2, Some(11))), 2),
        live(Value::Entry(entry(3, Some(12))), 3),
        conflict(
            Value::Entry(entry(4, Some(99))),
            Value::Entry(entry(4, None)),
            4,
        ),
        deleted(EntityKind::Entry, 5, 6),
        conflict(
            Value::Folder(folder(10, "Before", None)),
            Value::Folder(folder(10, "After", None)),
            7,
        ),
        deleted(EntityKind::Folder, 11, 9),
        live(Value::Folder(folder(12, "Visible", None)), 10),
    ]);
    projection.validate().unwrap();
    assert_eq!(projection.entries().len(), 3);
    assert_eq!(projection.folders().len(), 1);
    assert_eq!(projection.conflicts().count(), 2);
    assert_eq!(
        projection.structural_conflicts(),
        BTreeSet::from([entity(EntityKind::Entry, 1), entity(EntityKind::Entry, 2)])
    );
}

#[test]
fn read_views_ignore_empty_head_sets() {
    let projection = projection(vec![
        Record {
            entity: entity(EntityKind::Entry, 1),
            versions: vec![],
        },
        Record {
            entity: entity(EntityKind::Folder, 11),
            versions: vec![],
        },
    ]);
    assert!(matches!(
        projection.validate(),
        Err(Error::InvalidTransaction)
    ));
    assert!(projection.entries().is_empty());
    assert!(projection.folders().is_empty());
    assert!(projection.structural_conflicts().is_empty());
}
