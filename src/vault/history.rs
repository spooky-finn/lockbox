//! Local revision persistence and shared verified-history projection.
use crate::{
    records::{entry::Entry, history::RecordChange},
    sync::protocol::EntityKind,
    vault::{Result, Vault, VaultError},
};
use std::collections::{BTreeMap, BTreeSet};

impl Vault {
    pub(super) fn next_local_history(&self) -> Vec<RecordChange> {
        let mut history = self.history.borrow().clone();
        if self.sync_state().is_some() {
            return history;
        }
        let saved = self.saved_entries.borrow();
        let ids: std::collections::BTreeSet<_> =
            saved.iter().chain(&self.entries).map(|e| &e.id).collect();
        for id in ids {
            let before = saved.iter().find(|e| &e.id == id);
            let after = self.entries.iter().find(|e| &e.id == id);
            if before == after {
                continue;
            }
            let restored = before.is_none() && history.iter().any(|h| &h.id == id);
            history.push(RecordChange {
                id: id.clone(),
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
        history
    }

    /// Shared revisions are included only after signature, membership and causal validation.
    pub fn record_history(&self) -> Result<Vec<RecordChange>> {
        if let Some(replay) = self.sync_replay()? {
            return Ok(replay.history);
        }
        Ok(self.history.borrow().clone())
    }

    /// Deleted records with an accessible previous value. Never hides history failures.
    pub fn trash(&self) -> Result<Vec<Entry>> {
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
        Ok(trash.into_values().cloned().collect())
    }

    pub fn restore_record(&mut self, id: &str) -> Result<()> {
        let mut entry = self
            .trash()?
            .into_iter()
            .find(|e| e.id == id)
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
