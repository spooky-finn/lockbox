use crate::{
    records::{entry::Entry, folder::Folder, totp},
    sync::protocol::{Error, Hash, MAX_MUTATIONS, Result, canonical},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Editor,
    Reader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Entry,
    Folder,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub kind: EntityKind,
    pub id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Value {
    Entry(Entry),
    Folder(Folder),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueRef<'a> {
    Entry(&'a Entry),
    Folder(&'a Folder),
}

impl ValueRef<'_> {
    fn entity(self) -> Entity {
        match self {
            Self::Entry(entry) => Entity {
                kind: EntityKind::Entry,
                id: entry.id.clone(),
            },
            Self::Folder(folder) => Entity {
                kind: EntityKind::Folder,
                id: folder.id.clone(),
            },
        }
    }

    fn into_owned(self) -> Value {
        match self {
            Self::Entry(entry) => Value::Entry(entry.clone()),
            Self::Folder(folder) => Value::Folder(folder.clone()),
        }
    }
}

impl Value {
    pub fn entity(&self) -> Entity {
        self.as_ref().entity()
    }

    fn as_ref(&self) -> ValueRef<'_> {
        match self {
            Self::Entry(entry) => ValueRef::Entry(entry),
            Self::Folder(folder) => ValueRef::Folder(folder),
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::Entry(entry) => {
                entry
                    .validate_attachments()
                    .map_err(|_| Error::InvalidTransaction)?;

                if trim_whitespace(&entry.title).is_empty()
                    || entry.title.len() > 1024
                    || entry.username.len() > 4096
                    || entry.password.len() > 65536
                    || entry.url.len() > 8192
                    || entry.notes.len() > 1024 * 1024
                    || entry.icon.as_ref().is_some_and(|icon| icon.len() > 256)
                {
                    return Err(Error::InvalidTransaction);
                }

                if let Some(totp) = &entry.totp
                    && (totp.secret.len() > 4096
                        || !totp
                            .secret
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b))
                        || totp::validate(totp).is_err())
                {
                    return Err(Error::InvalidTransaction);
                }

                if let Some(parent) = &entry.folder_id {
                    validate_id(parent)?;
                }
            }
            Self::Folder(folder) => {
                if trim_whitespace(&folder.name).is_empty() || folder.name.chars().count() > 128 {
                    return Err(Error::InvalidTransaction);
                }

                if let Some(parent) = &folder.parent_id {
                    validate_id(parent)?;
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub change: Hash,
    pub index: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Put,
    Delete,
    Restore,
    Resolve,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub entity: Entity,
    pub bases: Vec<Revision>,
    pub action: Action,
    pub value: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub mutations: Vec<Mutation>,
    /// Advisory provenance only; authorship always belongs to this event's signer.
    pub rebased_from: Option<Hash>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub revision: Revision,
    pub author: Hash,
    pub value: Option<Value>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub entity: Entity,
    pub versions: Vec<Version>,
}

/// Maximal whole-record versions, including tombstones. No timestamp winner.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub records: Vec<Record>,
}

impl Projection {
    pub fn heads(&self, entity: &Entity) -> Vec<Revision> {
        self.records
            .iter()
            .find(|r| &r.entity == entity)
            .map(|r| r.versions.iter().map(|v| v.revision).collect())
            .unwrap_or_default()
    }

    pub fn conflicts(&self) -> impl Iterator<Item = &Record> {
        self.records
            .iter()
            .filter(|record| record.versions.len() > 1)
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.entry_values().cloned().collect()
    }

    pub fn folders(&self) -> Vec<Folder> {
        self.folder_values().cloned().collect()
    }

    /// Only a single live head belongs in these views; conflicts retain every version.
    fn values(&self) -> impl Iterator<Item = &Value> {
        self.records
            .iter()
            .filter_map(|record| match record.versions.as_slice() {
                [version] => version.value.as_ref(),
                _ => None,
            })
    }

    fn entry_values(&self) -> impl Iterator<Item = &Entry> {
        self.values().filter_map(|value| match value {
            Value::Entry(entry) => Some(entry),
            _ => None,
        })
    }

    fn folder_values(&self) -> impl Iterator<Item = &Folder> {
        self.values().filter_map(|value| match value {
            Value::Folder(folder) => Some(folder),
            _ => None,
        })
    }

    /// Evidence uses entity IDs and stable ASCII name comparison. The experimental
    /// profile intentionally does not claim the draft's pinned Unicode rules.
    pub fn structural_conflicts(&self) -> BTreeSet<Entity> {
        let by_id: BTreeMap<_, _> = self.folder_values().map(|f| (f.id.as_str(), f)).collect();
        let entity = |kind, id: &str| Entity {
            kind,
            id: id.to_owned(),
        };

        let mut bad = BTreeSet::new();
        let mut names = BTreeMap::new();

        for folder in self.folder_values() {
            let name = trim_whitespace(&folder.name).to_ascii_lowercase();
            let key = (folder.parent_id.clone(), name);

            if let Some(previous) = names.insert(key, folder.id.as_str()) {
                bad.insert(entity(EntityKind::Folder, previous));
                bad.insert(entity(EntityKind::Folder, &folder.id));
            }

            let mut path = BTreeSet::new();
            let mut current = Some(folder.id.as_str());

            while let Some(id) = current {
                if !path.insert(id) || path.len() > 64 {
                    bad.insert(entity(EntityKind::Folder, &folder.id));
                    break;
                }

                let Some(next) = by_id.get(id) else {
                    bad.insert(entity(EntityKind::Folder, &folder.id));
                    break;
                };
                current = next.parent_id.as_deref();
            }
        }

        for entry in self.entry_values() {
            if entry.folder_id.as_ref().is_some_and(|id| {
                !by_id.contains_key(id.as_str()) || bad.contains(&entity(EntityKind::Folder, id))
            }) {
                bad.insert(entity(EntityKind::Entry, &entry.id));
            }
        }
        bad
    }

    pub(super) fn validate(&self) -> Result<()> {
        if self.records.len() > 100_000 {
            return Err(Error::ResourceBudgetExceeded);
        }

        if !crate::sync::protocol::wire::sorted_unique(
            &self.records.iter().map(|r| &r.entity).collect::<Vec<_>>(),
        ) {
            return Err(Error::InvalidTransaction);
        }

        let mut revisions = BTreeSet::new();

        for record in &self.records {
            validate_entity(&record.entity)?;

            if record.versions.is_empty()
                || record.versions.len() > crate::sync::protocol::MAX_OBJECTS
                || !crate::sync::protocol::wire::sorted_unique(
                    &record
                        .versions
                        .iter()
                        .map(|v| v.revision)
                        .collect::<Vec<_>>(),
                )
            {
                return Err(Error::InvalidTransaction);
            }

            for version in &record.versions {
                if !revisions.insert(version.revision) {
                    return Err(Error::InvalidTransaction);
                }

                if let Some(value) = &version.value {
                    if value.entity() != record.entity {
                        return Err(Error::InvalidTransaction);
                    }
                    value.validate()?;
                }
            }
        }

        Ok(())
    }

    pub fn digest(&self) -> Result<Hash> {
        Ok(Hash::digest(
            b"lockbox.sync.projection\0",
            &canonical(self)?,
        ))
    }
}

fn validate_entity(entity: &Entity) -> Result<()> {
    validate_id(&entity.id)
}

fn validate_id(id: &str) -> Result<()> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidTransaction);
    }

    Ok(())
}

/// Unicode 15.1 White_Space, fixed explicitly instead of compiler Unicode tables.
fn trim_whitespace(value: &str) -> &str {
    value.trim_matches(|c| matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{0085}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'))
}

impl Transaction {
    pub(super) fn validate(&self, before: &Projection) -> Result<()> {
        if self.mutations.len() > MAX_MUTATIONS {
            return Err(Error::ResourceBudgetExceeded);
        }

        if !crate::sync::protocol::wire::sorted_unique(
            &self.mutations.iter().map(|m| &m.entity).collect::<Vec<_>>(),
        ) {
            return Err(Error::InvalidTransaction);
        }

        for mutation in &self.mutations {
            validate_entity(&mutation.entity)?;
            let heads = before.heads(&mutation.entity);

            if heads != mutation.bases {
                return Err(Error::InvalidTransaction);
            }

            let old = before.records.iter().find(|r| r.entity == mutation.entity);
            let live = old.is_some_and(|r| r.versions.len() == 1 && r.versions[0].value.is_some());
            let valid = match mutation.action {
                Action::Put => {
                    (heads.is_empty() || (heads.len() == 1 && live)) && mutation.value.is_some()
                }
                Action::Delete => heads.len() == 1 && live && mutation.value.is_none(),
                Action::Restore => heads.len() == 1 && !live && mutation.value.is_some(),
                Action::Resolve => heads.len() >= 2,
            };

            if !valid {
                return Err(Error::InvalidTransaction);
            }

            if let Some(value) = &mutation.value {
                if value.entity() != mutation.entity {
                    return Err(Error::InvalidTransaction);
                }
                value.validate()?;
            }
        }

        let after = project(before, [(Hash::default(), Hash::default(), self)]);
        let after_conflicts = after.structural_conflicts();

        if !after_conflicts.is_subset(&before.structural_conflicts())
            || self
                .mutations
                .iter()
                .any(|mutation| after_conflicts.contains(&mutation.entity))
        {
            return Err(Error::InvalidTransaction);
        }

        Ok(())
    }

    /// Used by existing record/folder workflows. Conflicts require explicit resolve().
    pub fn diff(before: &Projection, entries: &[Entry], folders: &[Folder]) -> Result<Self> {
        let mut desired = BTreeMap::new();

        for value in entries
            .iter()
            .map(ValueRef::Entry)
            .chain(folders.iter().map(ValueRef::Folder))
        {
            if desired.insert(value.entity(), value).is_some() {
                return Err(Error::InvalidTransaction);
            }
        }

        let mut mutations = Vec::new();

        for record in &before.records {
            if record.versions.len() > 1 {
                if desired.remove(&record.entity).is_some() {
                    return Err(Error::InvalidTransaction);
                }
                continue;
            }

            let previous = record.versions[0].value.as_ref().map(Value::as_ref);
            let next = desired.remove(&record.entity);

            if previous == next {
                continue;
            }
            mutations.push(Mutation {
                entity: record.entity.clone(),
                bases: before.heads(&record.entity),
                action: match (previous, next) {
                    (Some(_), Some(_)) => Action::Put,
                    (Some(_), None) => Action::Delete,
                    (None, Some(_)) => Action::Restore,
                    (None, None) => continue,
                },
                value: next.map(ValueRef::into_owned),
            });
        }

        for (entity, value) in desired {
            mutations.push(Mutation {
                entity,
                bases: vec![],
                action: Action::Put,
                value: Some(value.into_owned()),
            });
        }
        mutations.sort_by(|a, b| a.entity.cmp(&b.entity));
        Ok(Self {
            mutations,
            rebased_from: None,
        })
    }

    pub fn resolve(before: &Projection, entity: Entity, value: Option<Value>) -> Self {
        Self {
            mutations: vec![Mutation {
                bases: before.heads(&entity),
                entity,
                action: Action::Resolve,
                value,
            }],
            rebased_from: None,
        }
    }
}

/// Reference projection is set-based: discard explicitly superseded revisions,
/// then union new versions. Input order and duplicate delivery have no effect.
pub(super) fn project<'a>(
    baseline: &Projection,
    changes: impl IntoIterator<Item = (Hash, Hash, &'a Transaction)>,
) -> Projection {
    let mut versions: BTreeMap<Entity, BTreeMap<Revision, Version>> = baseline
        .records
        .iter()
        .map(|r| {
            (
                r.entity.clone(),
                r.versions.iter().map(|v| (v.revision, v.clone())).collect(),
            )
        })
        .collect();
    let mut removed: BTreeMap<Entity, BTreeSet<Revision>> = BTreeMap::new();

    for (change, author, transaction) in changes {
        for (index, mutation) in transaction.mutations.iter().enumerate() {
            removed
                .entry(mutation.entity.clone())
                .or_default()
                .extend(&mutation.bases);
            let revision = Revision {
                change,
                index: index as u16,
            };
            versions.entry(mutation.entity.clone()).or_default().insert(
                revision,
                Version {
                    revision,
                    author,
                    value: mutation.value.clone(),
                },
            );
        }
    }
    Projection {
        records: versions
            .into_iter()
            .filter_map(|(entity, mut versions)| {
                if let Some(removed) = removed.get(&entity) {
                    versions.retain(|revision, _| !removed.contains(revision));
                }
                (!versions.is_empty()).then(|| Record {
                    entity,
                    versions: versions.into_values().collect(),
                })
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "tests/model.rs"]
mod tests;
