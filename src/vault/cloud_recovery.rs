//! Cloud-only owner recovery. Provider freshness is an explicit caller policy;
//! it is never silently promoted to independently authenticated evidence.

use crate::{
    sync::{
        protocol::{
            self, Error, Hash, Machine, RecoveryAnchor,
            cloud_recovery::{self as capsule, CloudSpace, RecoveryUpload},
        },
        transport::{Cancellation, Discovery, Storage, TransportError},
    },
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, Result, Vault, VaultError,
        profile::BootstrapProfile,
    },
};
use ed25519_dalek::SigningKey;
use secrecy::SecretString;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Independent anchors preserve the existing freshness contract. Trusting the
/// provider snapshot explicitly accepts undetectable replay after total loss.
pub enum CloudRecoveryTrust {
    Independent(RecoveryAnchor),
    TrustProviderSnapshot,
}

pub struct CloudRestore<'a> {
    pub root: &'a SigningKey,
    pub space: CloudSpace,
    pub provider: ProviderConfiguration,
    pub trust: CloudRecoveryTrust,
    pub phrase: Option<&'a crate::access::mnemonic::RecoveryPhrase>,
    pub cancel: Cancellation,
}

impl ProfileStore {
    /// Bounded discovery across all owned spaces in this bucket. Neither LIST
    /// order nor timestamps select a winner. Missing predecessors are fetched
    /// directly; competing authenticated capsule heads require explicit repair.
    pub fn discover_cloud_spaces(
        root: &SigningKey,
        storage: &mut dyn Storage,
        cancel: &Cancellation,
    ) -> Result<Vec<CloudSpace>> {
        let directory = capsule::directory(root);
        let mut queue = Vec::new();
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        loop {
            cancel.check()?;
            let page = match storage.list_recovery(directory, cursor.as_deref()) {
                Err(TransportError::NotFound) => return Err(VaultError::CloudRecoveryMissing),
                result => result?,
            };
            if page.objects.len() > 1000 || queue.len() + page.objects.len() > protocol::MAX_OBJECTS
            {
                return Err(Error::ResourceBudgetExceeded.into());
            }
            queue.extend(page.objects);
            match page.next {
                Some(next)
                    if next.len() <= 8192 && cursors.len() < 64 && cursors.insert(next.clone()) =>
                {
                    cursor = Some(next)
                }
                Some(_) => return Err(TransportError::InvalidResponse.into()),
                None => break,
            }
        }
        let mut found = BTreeMap::new();
        let mut attempted = BTreeSet::new();
        let mut bytes_read = 0usize;
        while let Some(hash) = queue.pop() {
            cancel.check()?;
            if !attempted.insert(hash) {
                continue;
            }
            if attempted.len() > protocol::MAX_OBJECTS {
                return Err(Error::ResourceBudgetExceeded.into());
            }
            let bytes = storage.get_recovery(directory, hash)?;
            bytes_read = bytes_read.saturating_add(bytes.len());
            if bytes.len() > protocol::MAX_OBJECT_BYTES || bytes_read > protocol::MAX_CACHE_BYTES {
                return Err(Error::ResourceBudgetExceeded.into());
            }
            // A provider can add arbitrary bytes; only root-decryptable capsules
            // signed by their verified controller participate in selection.
            if let Ok(space) = capsule::open(root, hash, &bytes) {
                if let Some(previous) = space.previous() {
                    queue.push(previous);
                }
                found.insert(hash, space);
            }
        }
        let mut parents = BTreeSet::new();
        for space in found.values() {
            if let Some(previous) = space.previous() {
                let parent = found.get(&previous).ok_or(Error::MissingDependency)?;
                if !space.follows(parent)? {
                    return Err(Error::InvalidSignature.into());
                }
                parents.insert(previous);
            }
        }
        let candidates: Vec<_> = found
            .iter()
            .filter(|(hash, _)| !parents.contains(hash))
            .map(|(_, space)| space)
            .collect();
        let mut heads = BTreeMap::new();
        for space in &candidates {
            // A legitimate controller handoff may start a new local capsule
            // chain. Its signed control ancestry dominates the earlier device;
            // unrelated heads in the same epoch still have no automatic winner.
            let mut superseded = false;
            for other in &candidates {
                if other.anchor().control != space.anchor().control && other.follows(space)? {
                    superseded = true;
                    break;
                }
            }
            if superseded {
                continue;
            }
            let mut selected = (*space).clone();
            let id = selected.id();
            selected.inherit_history(found.values().filter(|candidate| candidate.id() == id))?;
            if heads.insert(selected.id(), selected).is_some() {
                return Err(VaultError::CloudRecoveryConflict);
            }
        }
        if heads.is_empty() {
            return Err(VaultError::CloudRecoveryMissing);
        }
        Ok(heads.into_values().collect())
    }

    pub fn restore_cloud_space(
        &self,
        path: impl AsRef<Path>,
        password: &SecretString,
        kdf: KdfParams,
        request: CloudRestore<'_>,
        storage: &mut dyn Storage,
    ) -> Result<Vault> {
        let CloudRestore {
            root,
            space,
            provider,
            trust,
            phrase,
            cancel,
        } = request;
        if root.verifying_key().to_bytes() != space.anchor().descriptor.owner_root
            || phrase
                .is_some_and(|phrase| phrase.signing_key().verifying_key() != root.verifying_key())
        {
            return Err(Error::UnauthorizedDevice.into());
        }
        let mut state = space.state()?;
        let mut required: BTreeSet<_> = space.anchor().frontier.iter().copied().collect();
        if let CloudRecoveryTrust::Independent(anchor) = &trust {
            if anchor.descriptor != state.descriptor() || anchor.frontier.len() > 512 {
                return Err(Error::InvalidEncoding.into());
            }
            state.required_control = Some(anchor.control);
            required.extend(anchor.frontier.iter().copied());
        }
        cancel.check()?;
        let mut downloaded_bytes = 0usize;
        for hash in required {
            cancel.check()?;
            let bytes = storage.get(state.namespace, hash)?;
            downloaded_bytes = downloaded_bytes.saturating_add(bytes.len());
            if downloaded_bytes > protocol::MAX_CACHE_BYTES {
                return Err(Error::ResourceBudgetExceeded.into());
            }
            Machine::receive(&mut state, hash, bytes)?;
        }
        let mut complete = false;
        for _ in 0..64 {
            let batch = Discovery::for_state(&state)?.download(storage, &cancel)?;
            let incomplete = batch.incomplete;
            let count = batch.objects.len();
            for (hash, bytes) in batch.objects {
                downloaded_bytes = downloaded_bytes.saturating_add(bytes.len());
                if downloaded_bytes > protocol::MAX_CACHE_BYTES {
                    return Err(Error::ResourceBudgetExceeded.into());
                }
                Machine::receive(&mut state, hash, bytes)?;
            }
            if state
                .objects
                .values()
                .map(|bytes| bytes.0.len())
                .sum::<usize>()
                > protocol::MAX_CACHE_BYTES
            {
                return Err(Error::ResourceBudgetExceeded.into());
            }
            if !incomplete {
                complete = true;
                break;
            }
            if count == 0 {
                break;
            }
        }
        if !complete {
            return Err(Error::MissingDependency.into());
        }
        let replay = Machine::replay_recovery(&state, space.keys())?;
        if !replay.ready
            || !replay.pending.is_empty()
            || space
                .anchor()
                .frontier
                .iter()
                .any(|hash| !replay.accepted.contains(hash))
        {
            return Err(Error::MissingDependency.into());
        }
        let anchor = match trust {
            CloudRecoveryTrust::Independent(anchor) => anchor,
            CloudRecoveryTrust::TrustProviderSnapshot => RecoveryAnchor {
                descriptor: state.descriptor(),
                control: replay.membership,
                frontier: replay.frontier.iter().copied().collect(),
            },
        };
        cancel.check()?;
        let mut profile = BootstrapProfile::generate_for_root(
            root,
            space.username().into(),
            space.name().into(),
            provider,
            password,
            kdf,
        )?;
        cancel.check()?;
        let prepared = Machine::prepare_recovery_from_replay(
            &state,
            replay,
            profile
                .certificate
                .clone()
                .ok_or(Error::UnauthorizedDevice)?,
            &anchor,
        )?;
        let projection = Machine::replay_recovery(&state, space.keys())?.projection;
        state.outbox.clear();
        state.last_exchange = None;
        state.required_control = Some(anchor.control);
        state.intent = Some(prepared);
        state.outbox = Machine::control_intent_dependencies(&state)?;
        profile.space.space_id = state.space.0;
        profile.space.namespace = state.namespace.0;
        profile.sync = Some(state);
        profile.enable_cloud_backup(root)?;
        let backup = profile.cloud_backup.as_mut().unwrap();
        backup.target = space.target();
        backup.archive = space.keys().to_vec();
        backup.upload = Some(RecoveryUpload {
            fingerprint: Hash([0; 32]),
            hash: space.hash,
            bytes: space.bytes.clone(),
            acknowledged: true,
        });
        cancel.check()?;
        let mut vault = Vault::create_profile_snapshot(
            path.as_ref(),
            password,
            kdf,
            profile,
            self.witnesses.clone(),
            (projection.entries(), projection.folders(), phrase),
        )?;
        cancel.check()?;
        vault.approve_controller_recovery(password)?;
        Ok(vault)
    }
}

impl Vault {
    /// Upgrade an existing writable controller while its root is available.
    pub fn enable_cloud_recovery(&mut self, password: &SecretString) -> Result<bool> {
        self.ensure_profile_writable()?;
        let Some(profile) = self.profile.as_ref() else {
            return Ok(false);
        };
        let Some(state) = &profile.sync else {
            return Ok(false);
        };
        if profile
            .cloud_backup
            .as_ref()
            .is_some_and(|backup| backup.target.body.descriptor == state.descriptor())
        {
            return Ok(false);
        }
        if state.owner_root != profile.identity.root_key {
            return Ok(false);
        }
        let certificate = profile
            .certificate
            .as_ref()
            .ok_or(Error::UnauthorizedDevice)?;
        if Machine::controller(state)? != certificate.body.device {
            return Ok(false);
        }
        let root = profile.unlock_root(password)?;
        self.profile.as_mut().unwrap().enable_cloud_backup(&root)?;
        self.save_inner(false)?;
        Ok(true)
    }

    pub(crate) fn prepare_cloud_backup(&mut self) -> Result<Option<(Hash, RecoveryUpload)>> {
        self.ensure_profile_writable()?;
        let Some(profile) = self.profile.as_ref() else {
            return Ok(None);
        };
        let Some(backup) = &profile.cloud_backup else {
            return Ok(None);
        };
        let state = profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;
        if state.owner_root != profile.identity.root_key
            || backup.target.body.descriptor != state.descriptor()
        {
            return Ok(None);
        }
        let upload = match capsule::prepare(
            state,
            &profile.sync_secret(),
            backup,
            &profile.username,
            &profile.space_name,
        ) {
            Err(Error::InsufficientRole) => return Ok(None),
            result => result?,
        };
        let Some(upload) = upload else {
            return Ok(None);
        };
        let directory = backup.target.body.directory;
        self.profile
            .as_mut()
            .unwrap()
            .cloud_backup
            .as_mut()
            .unwrap()
            .upload = Some(upload.clone());
        self.save_inner(false)?;
        Ok(Some((directory, upload)))
    }

    pub(crate) fn acknowledge_cloud_backup(&mut self, hash: Hash) -> Result<()> {
        let upload = self
            .profile
            .as_mut()
            .and_then(|p| p.cloud_backup.as_mut())
            .and_then(|b| b.upload.as_mut())
            .ok_or(Error::InvalidTransaction)?;
        if upload.hash != hash {
            return Err(Error::InvalidTransaction.into());
        }
        upload.acknowledged = true;
        if let Err(error) = self.save_inner(false) {
            self.profile
                .as_mut()
                .unwrap()
                .cloud_backup
                .as_mut()
                .unwrap()
                .upload
                .as_mut()
                .unwrap()
                .acknowledged = false;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn cloud_backup_current(&self) -> Result<bool> {
        let Some(profile) = &self.profile else {
            return Ok(true);
        };
        let Some(backup) = &profile.cloud_backup else {
            return Ok(true);
        };
        let state = profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;
        if state.owner_root != profile.identity.root_key
            || backup.target.body.descriptor != state.descriptor()
        {
            return Ok(true);
        }
        match capsule::is_current(
            state,
            &profile.sync_secret(),
            backup,
            &profile.username,
            &profile.space_name,
        ) {
            Err(Error::InsufficientRole) => Ok(true),
            result => result.map_err(Into::into),
        }
    }
}

#[cfg(test)]
#[path = "tests/cloud_recovery.rs"]
mod tests;
