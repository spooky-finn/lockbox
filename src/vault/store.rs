//! Explicit witness-store injection for CLI hosts, tests and alternate platforms.

use crate::{
    sync::{
        identity::Key,
        protocol::{Error, Hash, Machine, RecoveryAnchor, SpaceDescriptor},
    },
    vault::{
        KdfParams, ProviderConfiguration, Result, Unlock, Vault,
        profile::BootstrapProfile,
        witness::{self, WitnessStore},
    },
};
use secrecy::SecretString;
use std::{path::Path, sync::Arc};

pub struct SharedProfile {
    pub username: String,
    pub space_name: String,
    pub provider: ProviderConfiguration,
}

/// Recovery preserves user authority, generates new device keys and waits for
/// explicit admission. Supply a current descriptor/head through an authenticated
/// channel independently of the backup and provider. Credentials stay local.
pub struct DeviceRecovery<'a> {
    pub source: &'a Vault,
    pub recovery_password: &'a SecretString,
    pub descriptor: SpaceDescriptor,
    pub control_anchor: Hash,
    pub configuration: SharedProfile,
}

/// Recover identity authority and current data separately. Both sources remain
/// unchanged; the recovered controller receives fresh device keys and a new file.
pub struct ControllerRecovery<'a> {
    pub authority: &'a Vault,
    pub content: &'a Vault,
    pub recovery_password: &'a SecretString,
    pub anchor: RecoveryAnchor,
    pub configuration: SharedProfile,
}

/// Production defaults use the OS installation store. Test hosts inject their
/// own store and keep the same instance across profile reopening.
#[derive(Clone)]
pub struct ProfileStore {
    pub(super) witnesses: Arc<dyn WitnessStore>,
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::new(witness::system())
    }
}

impl ProfileStore {
    pub fn recover_controller(
        &self,
        path: impl AsRef<Path>,
        password: &SecretString,
        kdf: KdfParams,
        recovery: ControllerRecovery<'_>,
    ) -> Result<Vault> {
        recovery.content.ensure_sync_projection()?;
        let source = recovery
            .authority
            .profile
            .as_ref()
            .ok_or(Error::UnauthorizedDevice)?;
        let content = recovery
            .content
            .profile
            .as_ref()
            .ok_or(Error::UnauthorizedDevice)?;
        let state = content.sync.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let configuration = recovery.configuration;
        let mut profile = source.recover_identity(
            recovery.recovery_password,
            kdf,
            configuration.username,
            configuration.space_name,
            configuration.provider,
        )?;
        let prepared = Machine::prepare_control_recovery(
            state,
            &content.sync_secret(),
            profile
                .certificate
                .clone()
                .ok_or(Error::UnauthorizedDevice)?,
            &recovery.anchor,
        )?;
        let projection = Machine::replay_state(state, &content.sync_secret())?.projection;
        let mut journal = state.clone();
        journal.outbox.clear();
        journal.last_exchange = None;
        journal.required_control = Some(recovery.anchor.control);
        journal.intent = Some(prepared);
        journal.outbox = Machine::control_intent_dependencies(&journal)?;
        profile.space.space_id = journal.space.0;
        profile.space.namespace = journal.namespace.0;
        profile.sync = Some(journal);
        profile.enable_authority();
        let mut vault = Vault::create_profile_snapshot(
            path.as_ref(),
            password,
            kdf,
            profile,
            self.witnesses.clone(),
            (projection.entries(), projection.folders(), None),
        )?;
        vault.approve_controller_recovery(recovery.recovery_password)?;
        Ok(vault)
    }

    pub fn new(witnesses: Arc<dyn WitnessStore>) -> Self {
        Self { witnesses }
    }

    pub fn create_shared(
        &self,
        path: impl AsRef<Path>,
        password: &SecretString,
        recovery_password: &SecretString,
        kdf: KdfParams,
        configuration: SharedProfile,
    ) -> Result<Vault> {
        let profile = BootstrapProfile::generate(
            configuration.username,
            configuration.space_name,
            configuration.provider,
            recovery_password,
            kdf,
        )?;
        self.create_profile(path, password, kdf, profile)
    }

    /// Creates a v8 profile with independent password and recovery-word key slots.
    pub fn create_shared_with_phrase(
        &self,
        path: impl AsRef<Path>,
        password: &SecretString,
        kdf: KdfParams,
        configuration: SharedProfile,
        phrase: &crate::access::mnemonic::RecoveryPhrase,
    ) -> Result<Vault> {
        let mut profile = BootstrapProfile::from_recovery_phrase(
            configuration.username,
            configuration.space_name,
            configuration.provider,
            password,
            kdf,
            phrase,
        )?;
        profile.enable_cloud_backup(&phrase.signing_key())?;
        Vault::create_profile_snapshot(
            path.as_ref(),
            password,
            kdf,
            profile,
            self.witnesses.clone(),
            (Vec::new(), Vec::new(), Some(phrase)),
        )
    }

    pub(crate) fn create_shared_with_identity(
        &self,
        path: &Path,
        password: &SecretString,
        kdf: KdfParams,
        configuration: SharedProfile,
        identity: &crate::vault::identity::Identity,
    ) -> Result<Vault> {
        let mut profile = BootstrapProfile::generate_for_root(
            &identity.signing_key(),
            configuration.username,
            configuration.space_name,
            configuration.provider,
            password,
            kdf,
        )?;
        profile.enable_cloud_backup(&identity.signing_key())?;
        let phrase = identity.phrase()?;
        Vault::create_profile_snapshot(
            path,
            password,
            kdf,
            profile,
            self.witnesses.clone(),
            (Vec::new(), Vec::new(), phrase.as_ref()),
        )
    }

    /// Recovers readable access without bypassing installation/rollback protection.
    pub fn open_with_recovery_phrase(
        &self,
        path: impl AsRef<Path>,
        phrase: &crate::access::mnemonic::RecoveryPhrase,
    ) -> Result<Vault> {
        Vault::open_using(
            path.as_ref(),
            Unlock::Recovery(phrase),
            self.witnesses.clone(),
        )
    }

    /// Never imports the old signing identity, sequence counters, outbox, epoch
    /// keys or readable snapshot as authoritative current content.
    ///
    /// ```no_run
    /// use lockbox::{sync::protocol::{Hash, SpaceDescriptor}, vault::{
    ///     DeviceRecovery, KdfParams, ProfileStore, Result, SharedProfile, Vault,
    /// }};
    /// use secrecy::SecretString;
    ///
    /// fn recover(
    ///     backup: &Vault,
    ///     descriptor: SpaceDescriptor,
    ///     authenticated_head: Hash,
    ///     configuration: SharedProfile,
    ///     new_master: &SecretString,
    ///     recovery_password: &SecretString,
    /// ) -> Result<Vault> {
    ///     ProfileStore::default().restore_device(
    ///         "new-device.lbx", new_master, KdfParams::default(),
    ///         DeviceRecovery {
    ///             source: backup, recovery_password, descriptor,
    ///             control_anchor: authenticated_head, configuration,
    ///         },
    ///     )
    /// }
    /// ```
    pub fn restore_device(
        &self,
        path: impl AsRef<Path>,
        password: &SecretString,
        kdf: KdfParams,
        recovery: DeviceRecovery<'_>,
    ) -> Result<Vault> {
        let source = recovery
            .source
            .profile
            .as_ref()
            .ok_or(Error::UnauthorizedDevice)?;
        let configuration = recovery.configuration;
        let mut profile = source.recover_identity(
            recovery.recovery_password,
            kdf,
            configuration.username,
            configuration.space_name,
            configuration.provider,
        )?;
        profile.space.space_id = recovery.descriptor.space.0;
        profile.space.namespace = recovery.descriptor.namespace.0;
        profile.sync = Some(Machine::join_at(
            recovery.descriptor,
            recovery.control_anchor,
        ));
        self.create_profile(path, password, kdf, profile)
    }

    pub(super) fn create_profile(
        &self,
        path: impl AsRef<Path>,
        password: &SecretString,
        kdf: KdfParams,
        profile: BootstrapProfile,
    ) -> Result<Vault> {
        Vault::create_profile_using(
            path.as_ref(),
            password,
            kdf,
            profile,
            self.witnesses.clone(),
        )
    }

    pub fn open(&self, path: impl AsRef<Path>, password: &SecretString) -> Result<Vault> {
        Vault::open_using(
            path.as_ref(),
            Unlock::Password(password),
            self.witnesses.clone(),
        )
    }

    pub fn open_with_key(&self, path: impl AsRef<Path>, key: &Key) -> Result<Vault> {
        Vault::open_using(path.as_ref(), Unlock::Key(key), self.witnesses.clone())
    }
}
