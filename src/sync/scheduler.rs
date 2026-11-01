//! Coordinate typed exchange phases without owning protocol or HTTP policy.
//! Workers return ciphertext or receipts; every local commit belongs to Vault.

use crate::{
    platform::jobs::Worker,
    sync::{
        protocol::{Hash, SyncState},
        transport as sync,
        transport::{Cancellation, Download, StorageFactory, TransportError},
    },
    vault::{Result as VaultResult, Vault, VaultError},
};
use std::{sync::Arc, time::Instant};

mod schedule;
use schedule::{INCOMPLETE_INTERVAL, RECOVERY_INTERVAL, Schedule};

/// A successful provider operation. Errors travel once through the worker result.
enum Reply {
    Downloaded(Download),
    Uploaded(Vec<Hash>),
}

/// A reply can only be consumed by the space and trust anchor that started it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Scope {
    space: Hash,
    namespace: Hash,
    genesis: Hash,
}

impl From<&SyncState> for Scope {
    fn from(state: &SyncState) -> Self {
        Self {
            space: state.space,
            namespace: state.namespace,
            genesis: state.genesis,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Status {
    #[default]
    Saved,
    Syncing,
    Synced,
    Offline,
    Incomplete,
    Conflict,
    Removed,
    Waiting,
    Failed,
    Recovery,
}

pub(crate) struct SpaceSync {
    worker: Option<Worker<sync::Result<Reply>>>,
    factory: Option<Arc<dyn StorageFactory>>,
    scope: Option<Scope>,
    schedule: Schedule,
    pub status: Status,
    stopped: bool,
}

impl Default for SpaceSync {
    fn default() -> Self {
        Self::new(None)
    }
}

impl SpaceSync {
    /// `None` selects infrastructure's configured provider at the vault boundary.
    pub fn new(factory: Option<Arc<dyn StorageFactory>>) -> Self {
        Self {
            worker: None,
            factory,
            scope: None,
            schedule: Schedule::new(Instant::now()),
            status: Status::Saved,
            stopped: false,
        }
    }

    pub fn retry(&mut self) {
        self.schedule.retry_now(Instant::now());
    }

    pub fn saved(&mut self) {
        if !self.stopped {
            self.status = Status::Saved;
            self.retry();
        }
    }

    /// A membership action must discard replies and pending publication plans
    /// prepared for the previous roster before committing the new epoch.
    pub fn suspend(&mut self) {
        self.worker = None;
    }

    pub fn needs_tick(&self, editing: bool) -> bool {
        !editing
            && !self.stopped
            && match &self.worker {
                Some(worker) => worker.is_finished(),
                None => self.schedule.is_due(Instant::now()),
            }
    }

    /// Observe a new scope or persisted fault even if an old worker is pending.
    pub fn needs_tick_for(&self, vault: &Vault, editing: bool) -> bool {
        vault.sync_state().is_some_and(|state| {
            self.scope != Some(Scope::from(state))
                || (state.fault.is_some() && !self.stopped)
                || (vault.has_pending_exchange() && self.worker.is_some())
                || (vault.signing_error().is_some()
                    && (self.worker.is_some() || self.status != Status::Recovery))
                || self.needs_tick(editing)
        })
    }

    /// Defers incoming history while editing. Space changes cancel stale jobs;
    /// verified faults cancel every pending job before further exchange.
    pub fn tick(&mut self, vault: &mut Vault, editing: bool) -> Option<String> {
        let state = vault.sync_state()?;
        let scope = Scope::from(state);

        if self.scope != Some(scope) {
            if self.scope.is_some() {
                self.worker = None;
                self.stopped = false;
                self.status = Status::Saved;
            }

            self.scope = Some(scope);
            self.retry();
        }

        if state.fault.is_some() {
            self.freeze();
            return None;
        }

        if vault.has_pending_exchange() {
            self.worker = None;
            self.status = Status::Incomplete;
        }

        if vault.signing_error().is_some() {
            self.worker = None;
            self.status = Status::Recovery;
        }

        if editing {
            return None;
        }

        self.advance(vault)
            .err()
            .map(|error| self.on_failure(vault, error))
    }

    fn advance(&mut self, vault: &mut Vault) -> VaultResult<()> {
        vault.ensure_profile_writable()?;
        vault.recover_exchange()?;

        if let Some(worker) = &mut self.worker {
            let Some(result) = worker.take() else {
                return Ok(());
            };

            self.worker = None;
            let reply = result.map_err(|_| TransportError::Unavailable)??;
            return match reply {
                Reply::Downloaded(batch) => self.downloaded(vault, batch),
                Reply::Uploaded(receipts) => self.uploaded(vault, receipts),
            };
        }

        if self.schedule.is_due(Instant::now()) {
            self.start_download(vault)?;
        }

        Ok(())
    }

    fn downloaded(&mut self, vault: &mut Vault, batch: Download) -> VaultResult<()> {
        let incomplete = batch.incomplete;
        vault.receive_sync(batch)?;
        let replay = vault
            .sync_replay_state()?
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;

        if replay.role.is_none() {
            let state = vault
                .sync_state()
                .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;
            let device = vault
                .device_certificate()
                .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?
                .body
                .device;
            self.status = if crate::sync::protocol::Machine::was_member(state, device)? {
                Status::Removed
            } else {
                Status::Waiting
            };
            self.schedule.defer(Instant::now(), RECOVERY_INTERVAL);
            return Ok(());
        }

        if !replay.ready || incomplete {
            self.status = Status::Incomplete;
            self.schedule.defer(Instant::now(), INCOMPLETE_INTERVAL);
            return Ok(());
        }

        let state = vault
            .sync_state()
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;
        let plan = sync::publication(state, &replay)?;
        let namespace = state.namespace;
        self.start_exchange(vault, move |storage, cancel| {
            sync::upload(storage, namespace, &plan, cancel).map(Reply::Uploaded)
        })
    }

    fn uploaded(&mut self, vault: &mut Vault, receipts: Vec<Hash>) -> VaultResult<()> {
        let replay = vault
            .sync_replay_state()?
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;

        if replay.role.is_none() {
            return Err(crate::sync::protocol::Error::UnauthorizedDevice.into());
        }

        if !replay.ready {
            return Err(crate::sync::protocol::Error::MissingDependency.into());
        }

        let state = vault
            .sync_state()
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;
        let plan = sync::publication(state, &replay)?;
        let confirmed: std::collections::BTreeSet<_> = receipts.iter().copied().collect();
        let pending = plan.iter().any(|(hash, _)| !confirmed.contains(hash));

        // Acknowledgements and exchange time are saved only after fresh replay and
        // publication validation. A validation failure cannot become "Synced".
        vault.acknowledge_sync(&receipts, true)?;
        self.schedule.succeeded(Instant::now());
        self.status = if pending {
            self.retry();
            Status::Saved
        } else if replay.projection.conflicts().next().is_some()
            || !replay.projection.structural_conflicts().is_empty()
            || !replay.excluded.is_empty()
        {
            Status::Conflict
        } else {
            Status::Synced
        };

        Ok(())
    }

    fn start_download(&mut self, vault: &Vault) -> VaultResult<()> {
        let state = vault
            .sync_state()
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;
        let request = sync::Discovery::for_state(state)?;
        self.start_exchange(vault, move |storage, cancel| {
            request.download(storage, cancel).map(Reply::Downloaded)
        })
    }

    /// Provider connections stay inside the worker; vault access stays with the caller.
    fn start_exchange(
        &mut self,
        vault: &Vault,
        task: impl FnOnce(&mut dyn sync::Storage, &Cancellation) -> sync::Result<Reply> + Send + 'static,
    ) -> VaultResult<()> {
        let factory = match &self.factory {
            Some(factory) => factory.clone(),
            None => vault.storage_factory()?,
        };
        self.start(move |cancel| {
            let mut storage = factory.connect()?;
            task(&mut *storage, &cancel)
        });
        Ok(())
    }

    fn start(&mut self, task: impl FnOnce(Cancellation) -> sync::Result<Reply> + Send + 'static) {
        self.worker = Some(Worker::spawn(task));
        self.status = Status::Syncing;
    }

    fn freeze(&mut self) {
        self.worker = None;
        self.stopped = true;
        self.status = Status::Failed;
    }

    fn on_failure(&mut self, vault: &Vault, error: VaultError) -> String {
        let persisted_fault = vault
            .sync_state()
            .is_some_and(|state| state.fault.is_some());

        if persisted_fault || matches!(&error, VaultError::Sync(fault) if fault.is_history_fault())
        {
            self.freeze();
        } else if matches!(error, VaultError::Transport(_)) {
            let mut jitter = [0];
            let _ = getrandom::fill(&mut jitter);
            self.schedule.failed(Instant::now(), jitter[0]);
            self.status = match error {
                VaultError::Transport(TransportError::Verification(_)) => Status::Failed,
                _ => Status::Offline,
            };
        } else if matches!(error, VaultError::ProfileProtection(_)) {
            self.worker = None;
            self.status = Status::Recovery;
            self.schedule.defer(Instant::now(), RECOVERY_INTERVAL);
        } else {
            self.status = Status::Incomplete;
            self.schedule.defer(Instant::now(), RECOVERY_INTERVAL);
        }
        error.to_string()
    }
}

#[cfg(test)]
#[path = "tests/scheduler.rs"]
mod tests;
