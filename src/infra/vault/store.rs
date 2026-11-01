//! Explicit witness-store injection for CLI hosts, tests and alternate platforms.

use super::{KdfParams, ProviderConfiguration, Result, Unlock, Vault};
use crate::core::sync::{Error, Hash, Machine, SpaceDescriptor};
use crate::infra::{
    profile::BootstrapProfile,
    witness::{self, WitnessStore},
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

    /// Never imports the old signing identity, sequence counters, outbox, epoch
    /// keys or readable snapshot as authoritative current content.
    ///
    /// ```no_run
    /// use lockbox::{core::sync::{Hash, SpaceDescriptor}, infra::vault::{
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

    pub fn open_with_key(&self, path: impl AsRef<Path>, key: &[u8; 32]) -> Result<Vault> {
        Vault::open_using(path.as_ref(), Unlock::Key(key), self.witnesses.clone())
    }
}
