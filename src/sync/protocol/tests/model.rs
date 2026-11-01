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

#[test]
fn diff_keeps_order_actions_bases_and_owned_payloads_for_mixed_changes() {
    let mut kept = entry(1, None);
    kept.add_attachment("keep.bin".into(), vec![0, 255, 42])
        .unwrap();
    let before = projection(vec![
        live(Value::Entry(kept.clone()), 1),
        live(Value::Entry(entry(2, None)), 2),
        live(Value::Entry(entry(3, None)), 3),
        deleted(EntityKind::Entry, 4, 4),
        conflict(
            Value::Entry(entry(5, None)),
            Value::Entry(entry(5, None)),
            5,
        ),
        live(Value::Folder(folder(11, "Before", None)), 11),
        live(Value::Folder(folder(12, "Removed", None)), 12),
        deleted(EntityKind::Folder, 13, 13),
        live(Value::Folder(folder(14, "Kept", None)), 14),
        conflict(
            Value::Folder(folder(15, "Conflict A", None)),
            Value::Folder(folder(15, "Conflict B", None)),
            15,
        ),
    ]);
    before.validate().unwrap();
    let mut changed = entry(2, None);
    changed.password = "edited password".into();
    changed
        .add_attachment("edited.bin".into(), vec![3, 2, 1])
        .unwrap();
    let mut entries = vec![entry(6, None), changed, entry(4, None), kept];
    let mut folders = vec![
        folder(16, "New", None),
        folder(14, "Kept", None),
        folder(11, "After", None),
        folder(13, "Restored", None),
    ];
    let transaction = Transaction::diff(&before, &entries, &folders).unwrap();
    transaction.validate(&before).unwrap();
    let expected = [
        (EntityKind::Entry, 2, Action::Put, Some(2)),
        (EntityKind::Entry, 3, Action::Delete, Some(3)),
        (EntityKind::Entry, 4, Action::Restore, Some(4)),
        (EntityKind::Entry, 6, Action::Put, None),
        (EntityKind::Folder, 11, Action::Put, Some(11)),
        (EntityKind::Folder, 12, Action::Delete, Some(12)),
        (EntityKind::Folder, 13, Action::Restore, Some(13)),
        (EntityKind::Folder, 16, Action::Put, None),
    ];
    assert_eq!(transaction.mutations.len(), expected.len());
    assert!(transaction.rebased_from.is_none());
    for (mutation, (kind, id, action, base)) in transaction.mutations.iter().zip(expected) {
        assert_eq!(mutation.entity, entity(kind, id));
        assert_eq!(mutation.action, action);
        assert_eq!(
            mutation.bases,
            base.map(|revision| version(revision, None).revision)
                .into_iter()
                .collect::<Vec<_>>()
        );
        match &mutation.value {
            Some(Value::Entry(actual)) => {
                assert_eq!(actual, entries.iter().find(|e| e.id == actual.id).unwrap());
            }
            Some(Value::Folder(actual)) => {
                assert_eq!(actual, folders.iter().find(|f| f.id == actual.id).unwrap());
            }
            None => assert_eq!(action, Action::Delete),
        }
    }
    let serialized = canonical(&transaction).unwrap();
    entries[1].password = "later edit".into();
    entries[1].attachments[0].data[0] = 9;
    folders[2].name = "Later folder edit".into();
    assert_eq!(canonical(&transaction).unwrap(), serialized);
}

#[test]
fn diff_rejects_duplicate_and_conflicted_entities_but_distinguishes_kinds() {
    let before = projection(vec![
        conflict(
            Value::Entry(entry(1, None)),
            Value::Entry(entry(1, None)),
            1,
        ),
        conflict(
            Value::Folder(folder(11, "First", None)),
            Value::Folder(folder(11, "Second", None)),
            3,
        ),
    ]);
    for (entries, folders) in [
        (vec![entry(1, None)], vec![]),
        (vec![], vec![folder(11, "First", None)]),
        (vec![entry(2, None), entry(2, None)], vec![]),
        (
            vec![],
            vec![folder(12, "Same", None), folder(12, "Same", None)],
        ),
    ] {
        assert!(matches!(
            Transaction::diff(&before, &entries, &folders),
            Err(Error::InvalidTransaction)
        ));
    }
    assert!(
        Transaction::diff(&before, &[], &[])
            .unwrap()
            .mutations
            .is_empty()
    );
    let transaction = Transaction::diff(
        &before,
        &[entry(2, None)],
        &[folder(2, "Different kind", None)],
    )
    .unwrap();
    assert_eq!(transaction.mutations.len(), 2);
    transaction.validate(&before).unwrap();
}

#[test]
fn projection_requires_strict_record_and_revision_order() {
    Projection::default().validate().unwrap();
    let ordered = projection(vec![
        live(Value::Entry(entry(1, None)), 1),
        live(Value::Entry(entry(2, None)), 2),
    ]);
    ordered.validate().unwrap();
    let mut reversed = ordered.clone();
    reversed.records.reverse();
    assert!(matches!(
        reversed.validate(),
        Err(Error::InvalidTransaction)
    ));
    let mut duplicate = ordered;
    duplicate.records[1] = duplicate.records[0].clone();
    assert!(matches!(
        duplicate.validate(),
        Err(Error::InvalidTransaction)
    ));

    let ordered = projection(vec![conflict(
        Value::Entry(entry(1, None)),
        Value::Entry(entry(1, None)),
        1,
    )]);
    ordered.validate().unwrap();
    let mut reversed = ordered.clone();
    reversed.records[0].versions.reverse();
    assert!(matches!(
        reversed.validate(),
        Err(Error::InvalidTransaction)
    ));
    let mut duplicate = ordered;
    duplicate.records[0].versions[1] = duplicate.records[0].versions[0].clone();
    assert!(matches!(
        duplicate.validate(),
        Err(Error::InvalidTransaction)
    ));
}

#[test]
fn transaction_requires_strict_entity_order() {
    let before = Projection::default();
    Transaction::diff(&before, &[], &[])
        .unwrap()
        .validate(&before)
        .unwrap();
    let ordered = Transaction::diff(&before, &[entry(1, None), entry(2, None)], &[]).unwrap();
    ordered.validate(&before).unwrap();
    let mut reversed = ordered.clone();
    reversed.mutations.reverse();
    assert!(matches!(
        reversed.validate(&before),
        Err(Error::InvalidTransaction)
    ));
    let mut duplicate = ordered;
    duplicate.mutations[1] = duplicate.mutations[0].clone();
    assert!(matches!(
        duplicate.validate(&before),
        Err(Error::InvalidTransaction)
    ));
}

#[test]
fn transaction_bases_require_every_head_in_exact_order() {
    let before = projection(vec![
        live(Value::Entry(entry(1, None)), 1),
        deleted(EntityKind::Entry, 2, 2),
        conflict(
            Value::Entry(entry(3, None)),
            Value::Entry(entry(3, None)),
            3,
        ),
    ]);
    before.validate().unwrap();
    let transaction = Transaction {
        mutations: [
            (1, Action::Delete),
            (2, Action::Restore),
            (3, Action::Resolve),
            (4, Action::Put),
        ]
        .into_iter()
        .map(|(id, action)| {
            let entity = entity(EntityKind::Entry, id);
            Mutation {
                bases: before.heads(&entity),
                entity,
                action,
                value: (action != Action::Delete).then(|| Value::Entry(entry(id, None))),
            }
        })
        .collect(),
        rebased_from: None,
    };
    transaction.validate(&before).unwrap();
    for (index, mutation) in transaction.mutations.iter().enumerate() {
        let mut reversed = mutation.bases.clone();
        reversed.reverse();
        let mut extra = mutation.bases.clone();
        extra.push(version(99, None).revision);
        let truncated = mutation.bases[..mutation.bases.len().saturating_sub(1)].to_vec();
        for bases in [reversed, extra, truncated] {
            if bases == mutation.bases {
                continue;
            }
            let mut invalid = transaction.clone();
            invalid.mutations[index].bases = bases;
            assert!(matches!(
                invalid.validate(&before),
                Err(Error::InvalidTransaction)
            ));
        }
    }
}

#[test]
fn shared_entry_and_folder_ids_require_canonical_hex() {
    let before = Projection::default();
    for invalid in [
        String::new(),
        "a".repeat(31),
        "a".repeat(33),
        "A".repeat(32),
        "g".repeat(32),
        "é".repeat(16),
    ] {
        let mut account = entry(1, None);
        account.id = invalid.clone();
        let mut directory = folder(1, "Folder", None);
        directory.id = invalid;
        for transaction in [
            Transaction::diff(&before, &[account], &[]).unwrap(),
            Transaction::diff(&before, &[], &[directory]).unwrap(),
        ] {
            assert!(matches!(
                transaction.validate(&before),
                Err(Error::InvalidTransaction)
            ));
        }
    }
    let mut account = entry(1, None);
    account.id = "0123456789abcdef0123456789abcdef".into();
    Transaction::diff(&before, &[account], &[])
        .unwrap()
        .validate(&before)
        .unwrap();
}
