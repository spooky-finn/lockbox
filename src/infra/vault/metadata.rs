//! Read-only public profile metadata and durable local participant aliases.

use super::{Result, Vault, VaultError};
use crate::core::identity::Key;
use crate::core::sync::Hash;

/// Public identity metadata, without provider credentials or private keys.
pub struct ProfileSummary<'a> {
    pub username: &'a str,
    pub space_name: &'a str,
    pub user_id: Key,
    pub device_id: Key,
    pub space_id: Key,
    pub sync_enabled: bool,
}

impl Vault {
    pub fn profile_summary(&self) -> Option<ProfileSummary<'_>> {
        self.profile.as_ref().map(|profile| ProfileSummary {
            username: &profile.username,
            space_name: &profile.space_name,
            user_id: profile.identity.user_id,
            device_id: profile.identity.device_id,
            space_id: profile.space.space_id,
            sync_enabled: profile.sync.is_some(),
        })
    }

    pub fn participant_alias(&self, user: Hash) -> Option<&str> {
        self.profile
            .as_ref()?
            .participant_names
            .get(&user)
            .map(String::as_str)
    }

    /// Uses a local alias first, then this profile's own user name if applicable.
    pub fn participant_name(&self, user: Hash) -> Option<&str> {
        self.participant_alias(user).or_else(|| {
            let profile = self.profile.as_ref()?;
            profile
                .certificate
                .as_ref()
                .filter(|certificate| certificate.body.user == user)
                .map(|_| profile.username.as_str())
        })
    }

    /// Stores a local display alias. Empty input removes the alias.
    /// This metadata never creates a replicated membership event.
    pub fn set_participant_name(&mut self, user: Hash, name: &str) -> Result<()> {
        let name = name.trim();
        if name.len() > 256 || name.chars().any(char::is_control) {
            return Err(VaultError::InvalidParticipantName);
        }
        let Some(profile) = self.profile.as_mut() else {
            return Ok(());
        };
        let previous = profile.participant_names.get(&user).cloned();
        if name.is_empty() {
            profile.participant_names.remove(&user);
        } else {
            profile.participant_names.insert(user, name.to_owned());
        }
        if let Err(error) = self.save() {
            let profile = self.profile.as_mut().expect("profile retained during save");
            if let Some(previous) = previous {
                profile.participant_names.insert(user, previous);
            } else {
                profile.participant_names.remove(&user);
            }
            return Err(error);
        }
        Ok(())
    }
}

// Fault injection and serialized legacy-profile fixtures stay test-only.
#[cfg(test)]
impl Vault {
    pub(crate) fn test_profile(&self) -> Option<&super::BootstrapProfile> {
        self.profile.as_ref()
    }

    pub(crate) fn test_profile_mut(&mut self) -> Option<&mut super::BootstrapProfile> {
        self.profile.as_mut()
    }
}
