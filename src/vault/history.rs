//! Local revision persistence and shared verified-history projection.
use crate::{
    records::{entry::Entry, history::RecordChange},
    sync::protocol::EntityKind,
    vault::{Result, Vault, VaultError},
};
use std::collections::{BTreeMap, BTreeSet};

impl Vault {
    pub(super) fn local_history_changes(&self) -> Vec<RecordChange> {
        if self.sync_state().is_some() {
            return Vec::new();
        }
        let history = self.history.borrow();
        let saved = self.saved_entries.borrow();
        let mut changes = BTreeMap::new();
        // Legacy local files may repeat IDs; history uses the first value.
        for entry in saved.iter() {
            changes
                .entry(entry.id.as_str())
                .or_insert((Some(entry), None));
        }
        for entry in &self.entries {
            let (_, after) = changes.entry(entry.id.as_str()).or_insert((None, None));
            after.get_or_insert(entry);
        }
        let mut revisions = Vec::new();
        for (id, (before, after)) in changes {
            if before == after {
                continue;
            }
            let restored = before.is_none() && history.iter().any(|h| h.id == id);
            revisions.push(RecordChange {
                id: id.to_owned(),
                revision: crate::records::id::new(),
                actor: "Local user".into(),
                device: "Local vault".into(),
                at: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                ),
                action: if after.is_none() {
                    "Delete"
                } else if restored {
                    "Restore"
                } else {
                    "Put"
                }
                .into(),
                before: before.cloned().into_iter().collect(),
                after: after.cloned(),
            });
        }
        revisions
    }

    /// Shared revisions are included only after signature, membership and causal validation.
    pub fn record_history(&self) -> Result<Vec<RecordChange>> {
        if let Some(replay) = self.sync_replay()? {
            return Ok(replay.history);
        }
        Ok(self.history.borrow().clone())
    }

    /// Verify the complete journal before selecting display revisions, including no selection.
    pub(crate) fn record_history_for(&self, record: Option<&str>) -> Result<Vec<RecordChange>> {
        if let Some(replay) = self.sync_replay_record(record)? {
            return Ok(replay.history);
        }
        Ok(self
            .history
            .borrow()
            .iter()
            .filter(|change| Some(change.id.as_str()) == record)
            .cloned()
            .collect())
    }

    /// Deleted records with an accessible previous value. Never hides history failures.
    pub fn trash(&self) -> Result<Vec<Entry>> {
        self.select_trash(|trash| trash.into_values().cloned().collect())
    }

    fn select_trash<T>(&self, select: impl FnOnce(BTreeMap<&str, &Entry>) -> T) -> Result<T> {
        let replay = self.sync_replay()?;
        let local_history = self.history.borrow();
        let history = replay
            .as_ref()
            .map_or(local_history.as_slice(), |replay| replay.history.as_slice());
        let deleted: Option<BTreeSet<_>> = replay.as_ref().map(|replay| {
            replay
                .projection
                .records
                .iter()
                .filter(|record| {
                    record.entity.kind == EntityKind::Entry
                        && record.versions.len() == 1
                        && record.versions[0].value.is_none()
                })
                .map(|record| record.entity.id.as_str())
                .collect()
        });
        let present: BTreeSet<_> = self.entries.iter().map(|entry| entry.id.as_str()).collect();
        let mut trash = BTreeMap::new();

        for change in history.iter().rev() {
            if present.contains(change.id.as_str())
                || deleted
                    .as_ref()
                    .is_some_and(|ids| !ids.contains(change.id.as_str()))
            {
                continue;
            }
            if let Some(entry) = change.after.as_ref().or_else(|| change.before.first()) {
                trash.entry(change.id.as_str()).or_insert(entry);
            }
        }
        Ok(select(trash))
    }

    pub fn restore_record(&mut self, id: &str) -> Result<()> {
        let mut entry = self
            .select_trash(|trash| trash.into_values().find(|entry| entry.id == id).cloned())?
            .ok_or(VaultError::Corrupted)?;
        if entry
            .folder_id
            .as_ref()
            .is_some_and(|id| !self.folders.iter().any(|f| &f.id == id))
        {
            entry.folder_id = None;
        }
        self.upsert_entry(entry)
    }
}

#[cfg(test)]
#[path = "tests/history.rs"]
mod tests;
