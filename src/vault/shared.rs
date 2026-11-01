//! Shared-space application operations backed by the encrypted vault adapter.
//!
//! Every signing operation uses one durable intent boundary. The parent module
//! owns encryption and atomic file replacement; protocol decisions remain in
//! `sync::protocol::Machine`.

use crate::{
    sync::protocol::{DeviceSecret, PreparedChange, Replay, Result as ProtocolResult, SyncState},
    vault::{
        KdfParams, ProviderConfiguration, Result, Vault, VaultError, profile::BootstrapProfile,
    },
};
use secrecy::SecretString;
use std::path::Path;

impl Vault {
    /// Creates a new independent device profile and owner space. Credentials and
    /// private keys are stored only inside this locally encrypted file.
    pub fn create_shared(
        path: impl AsRef<Path>,
        password: &SecretString,
        recovery_password: &SecretString,
        kdf: KdfParams,
        username: String,
        space_name: String,
        provider: ProviderConfiguration,
    ) -> Result<Self> {
        let profile =
            BootstrapProfile::generate(username, space_name, provider, recovery_password, kdf)?;
        Self::create_profile(path, password, kdf, profile)
    }

    pub(super) fn ensure_sync_projection(&self) -> Result<()> {
        self.ensure_exchange_committed()?;

        if let Some(profile) = &self.profile
            && let Some(sync) = &profile.sync
        {
            if let Some(error) = sync.fault {
                return Err(error.into());
            }

            let replay =
                crate::sync::protocol::Machine::replay_state(sync, &profile.sync_secret())?;

            if !replay.projection.entry_values().eq(self.entries.iter())
                || !replay.projection.folder_values().eq(self.folders.iter())
            {
                return Err(crate::sync::protocol::Error::InvalidTransaction.into());
            }
        }

        Ok(())
    }

    /// Existing app workflows stage records first. Shared profiles convert the
    /// complete edit into one atomic transaction and durably reserve its exact
    /// encrypted signing body before signing or acknowledging the local save.
    pub fn save_changes(&mut self) -> Result<()> {
        self.ensure_exchange_committed()?;
        use crate::sync::protocol::Machine;
        use crate::sync::protocol::Transaction;
        let Some(profile) = self.profile.as_ref().filter(|p| p.sync.is_some()) else {
            return self.save();
        };

        let device = profile.sync_secret();
        let desired_entries = self.entries.clone();
        let desired_folders = self.folders.clone();
        self.recover_sync()?;
        let sync = self.profile.as_ref().unwrap().sync.as_ref().unwrap();
        let replay = Machine::replay_state(sync, &device)?;
        let transaction =
            Transaction::diff(&replay.projection, &desired_entries, &desired_folders)?;

        if transaction.mutations.is_empty() {
            return self.save();
        }

        let prepared = self.prepare_sync_command(|sync, device| {
            Machine::prepare_borrowed(sync, device, &transaction)
        })?;
        self.entries = replay.projection.entries();
        self.folders = replay.projection.folders();
        self.commit_prepared(prepared)
    }

    pub(super) fn recover_sync(&mut self) -> Result<()> {
        use crate::sync::protocol::Machine;
        if !matches!(self.journal_check, super::JournalCheck::Ready)
            && self
                .protection
                .as_ref()
                .is_some_and(|p| p.borrow().has_file())
        {
            self.ensure_profile_writable()?;
            if matches!(self.journal_check, super::JournalCheck::Unchecked) {
                let changed = self
                    .profile
                    .as_mut()
                    .and_then(|p| p.sync.as_mut())
                    .map(Machine::quarantine_unverified)
                    .transpose()?
                    .unwrap_or(false);
                self.journal_check = if changed {
                    super::JournalCheck::PendingCommit
                } else {
                    super::JournalCheck::Ready
                };
            }
            if matches!(self.journal_check, super::JournalCheck::PendingCommit) {
                // Retain the dirty state after a failed write. A retry must
                // commit cleanup before it can sign or publish anything new.
                self.save_inner(false)?;
                self.journal_check = super::JournalCheck::Ready;
            }
        }
        // An intent may exist only in memory after its first write failed.
        // Reserve it successfully before any retry is allowed to sign it.
        if self
            .sync_state()
            .is_some_and(|state| state.intent.is_some() && state.fault.is_none())
        {
            self.save_inner(false)?;
        }

        self.ensure_profile_writable()?;
        let Some(profile) = self.profile.as_mut().filter(|p| p.sync.is_some()) else {
            return Ok(());
        };

        let device = profile.sync_secret();
        let sync = profile.sync.as_mut().unwrap();
        // A frozen history remains readable. Recovery never erases a fault or
        // resumes publication merely because the provider is available again.
        if sync.fault.is_some() {
            return Ok(());
        }

        let recovering = sync.intent.is_some();

        if recovering {
            if Machine::pending_control_approval(sync).is_some() {
                return Ok(());
            }
            Machine::complete_intent(sync, &device)?;
        }

        let replay = match Machine::replay_state(sync, &device) {
            Ok(replay) => replay,
            Err(crate::sync::protocol::Error::MissingDependency) if !recovering => return Ok(()),
            Err(error) => return Err(error.into()),
        };

        if !replay.ready && !recovering {
            return Ok(());
        }

        self.entries = replay.projection.entries();
        self.folders = replay.projection.folders();

        if recovering {
            self.save_inner(false)?;
        }

        Ok(())
    }

    /// Rebuild a verified view without changing the journal or writing files.
    /// Inspect `Replay::ready` before using it to replace a readable snapshot.
    pub fn sync_replay(&self) -> Result<Option<Replay>> {
        self.replay_using(crate::sync::protocol::Machine::replay)
    }

    /// Verify the complete journal with an empty display-history vector.
    pub(crate) fn sync_replay_state(&self) -> Result<Option<Replay>> {
        self.replay_using(crate::sync::protocol::Machine::replay_state)
    }

    pub(super) fn sync_replay_record(&self, record: Option<&str>) -> Result<Option<Replay>> {
        self.replay_using(|state, device| {
            crate::sync::protocol::Machine::replay_record_history(state, device, record)
        })
    }

    fn replay_using(
        &self,
        replay: impl FnOnce(&SyncState, &DeviceSecret) -> ProtocolResult<Replay>,
    ) -> Result<Option<Replay>> {
        self.ensure_exchange_committed()?;
        self.profile
            .as_ref()
            .and_then(|profile| profile.sync.as_ref().map(|sync| (profile, sync)))
            .map(|(profile, sync)| replay(sync, &profile.sync_secret()).map_err(VaultError::from))
            .transpose()
    }

    /// Choose a whole-record revision and durably resolve all currently visible
    /// versions. A later unseen concurrent edit can create another conflict.
    pub fn resolve_sync(
        &mut self,
        entity: crate::sync::protocol::Entity,
        revision: crate::sync::protocol::Revision,
    ) -> Result<()> {
        use crate::sync::protocol::Error;
        use crate::sync::protocol::Transaction;
        let replay = self.sync_replay_state()?.ok_or(Error::UnauthorizedDevice)?;
        let value = replay
            .projection
            .records
            .iter()
            .find(|r| r.entity == entity)
            .and_then(|record| record.versions.iter().find(|v| v.revision == revision))
            .ok_or(Error::InvalidTransaction)?
            .value
            .clone();
        self.commit_sync(Transaction::resolve(&replay.projection, entity, value))
    }

    pub fn excluded_edits(&self) -> Result<Vec<crate::sync::protocol::ExcludedEdit>> {
        use crate::sync::protocol::Error;
        use crate::sync::protocol::Machine;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let state = profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Ok(Machine::excluded_edits(state, &profile.sync_secret())?)
    }

    /// Explicit re-authoring after reviewing both the old edit and current state.
    pub fn rebase_edit(&mut self, hash: crate::sync::protocol::Hash) -> Result<()> {
        self.ensure_exchange_committed()?;
        use crate::sync::protocol::Machine;
        let prepared = self
            .prepare_sync_command(|state, device| Machine::prepare_rebase(state, device, hash))?;
        self.commit_prepared(prepared)
    }

    fn commit_sync(&mut self, transaction: crate::sync::protocol::Transaction) -> Result<()> {
        use crate::sync::protocol::Machine;
        let prepared = self.prepare_sync_command(|state, device| {
            Machine::prepare_borrowed(state, device, &transaction)
        })?;
        self.commit_prepared(prepared)
    }

    /// Changes existing roles, removes devices, or rotates keys. Profile 1
    /// requires `admit_invitation` and signed consent for adding certificates.
    pub fn set_space_members(
        &mut self,
        members: Vec<(
            crate::sync::protocol::Signed<crate::sync::protocol::Certificate>,
            crate::sync::protocol::Role,
        )>,
    ) -> Result<()> {
        self.ensure_exchange_committed()?;
        use crate::sync::protocol::Machine;
        let prepared = self.prepare_sync_command(|state, device| {
            Machine::prepare_membership(state, device, members.clone())
        })?;
        self.commit_prepared(prepared)
    }

    pub(super) fn prepare_sync_command(
        &mut self,
        command: impl Fn(
            &SyncState,
            &crate::sync::protocol::DeviceSecret,
        ) -> crate::sync::protocol::Result<PreparedChange>,
    ) -> Result<PreparedChange> {
        self.ensure_sync_commit_allowed()?;
        use crate::sync::protocol::Error;
        use crate::sync::protocol::Machine;
        loop {
            let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
            let state = profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;
            let device = profile.sync_secret();

            match command(state, &device) {
                Err(Error::FrontierTooWide) => {
                    let merge =
                        Machine::prepare_merge(state, &device)?.ok_or(Error::InvalidTransaction)?;
                    // save_changes may have staged a desired view. A causal
                    // merge reserves and commits only the verified prior view.
                    self.recover_sync()?;
                    self.commit_prepared(merge)?;
                }
                result => return result.map_err(VaultError::from),
            }
        }
    }

    pub fn space_descriptor(&self) -> Option<crate::sync::protocol::SpaceDescriptor> {
        self.profile
            .as_ref()?
            .sync
            .as_ref()
            .map(|sync| sync.descriptor())
    }

    /// Export a verified minimum control anchor from this installation's current
    /// durable state. Authenticate it independently before invitation or restore.
    pub fn space_anchor(&self) -> Result<crate::sync::protocol::Hash> {
        self.ensure_profile_writable()?;
        let replay = self
            .sync_replay_state()?
            .ok_or(crate::sync::protocol::Error::UnauthorizedDevice)?;

        if !replay.ready {
            return Err(crate::sync::protocol::Error::MissingDependency.into());
        }

        Ok(replay.membership)
    }

    pub fn device_certificate(
        &self,
    ) -> Option<crate::sync::protocol::Signed<crate::sync::protocol::Certificate>> {
        self.profile.as_ref()?.certificate.clone()
    }

    /// Certifies an older bootstrap-only device with its separately protected
    /// recovery authority, then journals its existing records as initial events.
    pub fn activate_sync(&mut self, recovery_password: &SecretString) -> Result<()> {
        self.ensure_exchange_committed()?;
        use crate::sync::protocol::Error;
        use crate::sync::protocol::Machine;
        use crate::sync::protocol::Projection;
        use crate::sync::protocol::Transaction;

        if self
            .profile
            .as_ref()
            .ok_or(Error::UnauthorizedDevice)?
            .sync
            .is_some()
        {
            return self.ensure_profile_writable();
        }

        let entries = self.entries.clone();
        let folders = self.folders.clone();
        self.profile
            .as_mut()
            .unwrap()
            .activate_sync(recovery_password)?;
        let result = (|| {
            let profile = self.profile.as_ref().unwrap();
            let device = profile.sync_secret();
            self.protection = Some(std::cell::RefCell::new(
                crate::vault::protection::Protection::create(
                    self.witnesses.clone(),
                    device.id(),
                    &self.salt,
                )?,
            ));
            let transaction = Transaction::diff(&Projection::default(), &entries, &folders)?;

            if transaction.mutations.is_empty() {
                return self.save_inner(false);
            }
            // The first protected file contains the complete import intent.
            // A crash cannot expose an empty genesis that discards local records.
            let prepared = Machine::prepare(
                self.profile.as_ref().unwrap().sync.as_ref().unwrap(),
                &device,
                transaction,
            )?;
            self.commit_prepared(prepared)
        })();

        if result.is_err() {
            let profile = self.profile.as_mut().unwrap();
            let state = profile.sync.as_ref().unwrap();

            if state.intent.is_none()
                && state.objects.len() == 1
                && self
                    .protection
                    .as_ref()
                    .is_none_or(|p| !p.borrow().has_file())
            {
                profile.cancel_activation();
                self.protection = None;
            }

            self.entries = entries;
            self.folders = folders;
        }
        result
    }

    /// Attach an empty device profile to independently authenticated space
    /// routing/identity metadata. No private key or shared secret is copied.
    pub fn join_space(&mut self, descriptor: crate::sync::protocol::SpaceDescriptor) -> Result<()> {
        self.ensure_exchange_committed()?;
        self.ensure_profile_writable()?;
        use crate::sync::protocol::Error;
        use crate::sync::protocol::Machine;

        if !self.entries.is_empty() || !self.folders.is_empty() {
            return Err(Error::InvalidTransaction.into());
        }

        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;

        if profile
            .sync
            .as_ref()
            .is_some_and(|s| s.intent.is_some() || s.objects.len() != 1)
        {
            return Err(Error::InvalidTransaction.into());
        }

        let previous = profile.sync.replace(Machine::join(
            descriptor.space,
            descriptor.namespace,
            descriptor.genesis,
            descriptor.owner_root,
        ));
        let old_space = profile.space.space_id;
        let old_namespace = profile.space.namespace;
        profile.space.space_id = descriptor.space.0;
        profile.space.namespace = descriptor.namespace.0;

        if let Err(error) = self.save_inner(false) {
            let profile = self.profile.as_mut().unwrap();
            profile.sync = previous;
            profile.space.space_id = old_space;
            profile.space.namespace = old_namespace;
            return Err(error);
        }

        Ok(())
    }

    /// Commit the exact reserved body before signing, then finish that same body.
    /// Errors retain the intent so recovery cannot silently reuse its sequence slot.
    pub(super) fn commit_prepared(&mut self, prepared: PreparedChange) -> Result<()> {
        self.ensure_exchange_committed()?;
        self.ensure_sync_commit_allowed()?;
        use crate::sync::protocol::Error;
        let state = self
            .profile
            .as_mut()
            .and_then(|profile| profile.sync.as_mut())
            .ok_or(Error::UnauthorizedDevice)?;

        if state.intent.is_some() {
            return Err(Error::PendingIntent.into());
        }
        state.intent = Some(prepared);
        self.recover_sync()
    }

    /// Ciphertext, discovery metadata and pinned anchors; no device secrets.
    pub(crate) fn sync_state(&self) -> Option<&SyncState> {
        self.profile.as_ref()?.sync.as_ref()
    }
}
