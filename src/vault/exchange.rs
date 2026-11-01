//! Remote exchange commits. Failed writes retain a separate verified candidate
//! and the last committed view. Authenticated history faults freeze the session.

use crate::{
    records::{entry::Entry, folder::Folder},
    sync::protocol::{Error, Machine, Projection, SyncState},
    vault::{Result, Vault, VaultError},
};

/// Verified receive candidate retained separately from the committed journal.
/// A failed file write must not forget a known successor and resume old uploads.
pub(super) struct PendingExchange {
    journal: SyncState,
    view: Option<ReceivedView>,
}

struct ReceivedView {
    entries: Vec<Entry>,
    folders: Vec<Folder>,
}

impl ReceivedView {
    fn install(self, vault: &mut Vault) -> Self {
        Self {
            entries: std::mem::replace(&mut vault.entries, self.entries),
            folders: std::mem::replace(&mut vault.folders, self.folders),
        }
    }
}

impl Vault {
    /// Verify into a candidate journal. Ordinary failed commits restore both
    /// journal and view, including when the new history is incomplete.
    pub(crate) fn receive_sync(&mut self, batch: crate::sync::transport::Download) -> Result<()> {
        self.recover_exchange()?;
        self.recover_sync()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let device = profile.sync_secret();
        let mut candidate = profile
            .sync
            .as_ref()
            .ok_or(Error::UnauthorizedDevice)?
            .clone();

        for (hash, bytes) in batch.objects {
            Machine::receive(&mut candidate, hash, bytes)?;
        }

        if candidate
            .objects
            .values()
            .map(|blob| blob.0.len())
            .sum::<usize>()
            > crate::sync::transport::MAX_CACHE_BYTES
        {
            return Err(Error::ResourceBudgetExceeded.into());
        }

        match Machine::replay(&candidate, &device) {
            Ok(replay) => {
                self.prepare_received(candidate, replay.ready.then_some(replay.projection))
            }
            Err(Error::MissingDependency) => self.prepare_received(candidate, None),
            Err(error) if error.is_history_fault() => {
                // Integrity evidence remains in memory even if its disk write
                // fails: a storage fault must never resume unsafe publication.
                candidate.fault = Some(error);
                self.profile.as_mut().unwrap().sync = Some(candidate);
                self.save_inner(false)?;
                Err(error.into())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Install only a fully verified projection; otherwise retain readable data.
    fn prepare_received(
        &mut self,
        candidate: SyncState,
        projection: Option<Projection>,
    ) -> Result<()> {
        let view = projection.map(|projection| ReceivedView {
            entries: projection.entries(),
            folders: projection.folders(),
        });
        self.commit_received(PendingExchange {
            journal: candidate,
            view,
        })
    }

    fn commit_received(&mut self, pending: PendingExchange) -> Result<()> {
        let previous = self
            .profile
            .as_mut()
            .ok_or(Error::UnauthorizedDevice)?
            .sync
            .replace(pending.journal);
        let previous_view = pending.view.map(|view| view.install(self));

        if let Err(error) = self.save_inner(false) {
            let journal =
                std::mem::replace(&mut self.profile.as_mut().unwrap().sync, previous).unwrap();
            let view = previous_view.map(|view| view.install(self));
            self.pending_exchange = Some(PendingExchange { journal, view });
            return Err(error);
        }

        Ok(())
    }

    pub(crate) fn has_pending_exchange(&self) -> bool {
        self.pending_exchange.is_some()
    }

    pub(super) fn ensure_exchange_committed(&self) -> Result<()> {
        if self.has_pending_exchange() {
            Err(VaultError::PendingSyncCommit)
        } else {
            Ok(())
        }
    }

    /// Retry the retained candidate before fetching or publishing any more data.
    pub(crate) fn recover_exchange(&mut self) -> Result<()> {
        if let Some(pending) = self.pending_exchange.take() {
            self.commit_received(pending)?;
        }

        Ok(())
    }

    pub(crate) fn acknowledge_sync(
        &mut self,
        hashes: &[crate::sync::protocol::Hash],
        completed: bool,
    ) -> Result<()> {
        self.ensure_exchange_committed()?;
        self.ensure_profile_writable()?;
        let Some(sync) = self.profile.as_mut().and_then(|p| p.sync.as_mut()) else {
            return Ok(());
        };

        let previous_exchange = sync.last_exchange;
        let removed: Vec<_> = hashes
            .iter()
            .filter(|hash| sync.outbox.remove(*hash))
            .collect();

        if completed {
            sync.last_exchange = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
        }

        if let Err(error) = self.save_inner(false) {
            let state = self.profile.as_mut().unwrap().sync.as_mut().unwrap();
            state.outbox.extend(removed);
            state.last_exchange = previous_exchange;
            return Err(error);
        }

        Ok(())
    }

    /// Blocking adapter for services/CLI callers. Desktop uses session-scoped
    /// ciphertext workers instead. Downloaded history is committed before any
    /// upload is authorized, and publication never sends the profile file.
    pub fn synchronize(&mut self, storage: &mut dyn crate::sync::transport::Storage) -> Result<()> {
        use crate::sync::protocol::Error;
        use crate::sync::protocol::Machine;
        use crate::sync::transport as sync;

        self.recover_exchange()?;
        self.recover_sync()?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let state = profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;

        if let Some(error) = state.fault {
            return Err(error.into());
        }

        let namespace = state.namespace;
        let cancel = sync::Cancellation::default();
        let batch = sync::Discovery::for_state(state)?.download(storage, &cancel)?;
        let incomplete = batch.incomplete;
        self.receive_sync(batch)?;
        let profile = self.profile.as_ref().unwrap();
        let state = profile.sync.as_ref().unwrap();
        let replay = Machine::replay(state, &profile.sync_secret())?;

        if replay.role.is_none() {
            return Err(Error::UnauthorizedDevice.into());
        }

        if incomplete || !replay.ready {
            return Err(Error::MissingDependency.into());
        }

        let plan = sync::publication(state, &replay)?;
        let receipts = sync::upload(storage, namespace, &plan, &cancel)?;
        self.acknowledge_sync(&receipts, true)
    }

    /// Provider composition stays in infrastructure. The returned factory has
    /// credentials and routing only; it cannot access this vault's private keys.
    pub(crate) fn storage_factory(
        &self,
    ) -> Result<std::sync::Arc<dyn crate::sync::transport::StorageFactory>> {
        let profile = self
            .profile
            .as_ref()
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;
        let configuration = &profile.provider;
        Ok(std::sync::Arc::new(
            crate::sync::transport::storj::StorjFactory::new(
                &configuration.base_url,
                &configuration.bucket,
                &profile.space_name,
                &configuration.access_key,
                &configuration.secret_key,
            ),
        ))
    }
}
