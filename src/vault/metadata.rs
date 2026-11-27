//! Public profile metadata, shared display names and local participant names.

use crate::{
    sync::{
        identity::Key,
        protocol::{
            Error, Hash, Role, SpaceIcon, SpaceIconChange, Transaction, pairing::valid_name,
        },
    },
    vault::{Result, Vault, VaultError},
};

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
    pub(crate) fn awaiting_member_storage(&self) -> bool {
        self.profile.as_ref().is_some_and(|profile| {
            let provider = &profile.provider;
            provider.base_url.is_empty()
                && provider.bucket.is_empty()
                && provider.access_key.is_empty()
                && provider.secret_key.is_empty()
        })
    }

    pub fn profile_summary(&self) -> Option<ProfileSummary<'_>> {
        if let Some(snapshot) = &self.editor_view {
            return snapshot.profile_summary();
        }
        self.profile.as_ref().map(|profile| ProfileSummary {
            username: &profile.username,
            space_name: profile
                .shared_name
                .as_deref()
                .unwrap_or(&profile.space_name),
            user_id: profile.identity.user_id,
            device_id: profile.identity.device_id,
            space_id: profile.space.space_id,
            sync_enabled: profile.sync.is_some(),
        })
    }

    pub(crate) fn can_edit_shared_metadata(&self) -> bool {
        self.signing_error().is_none()
            && self.sync_replay_state().is_ok_and(|replay| {
                replay.is_some_and(|replay| replay.ready && replay.role == Some(Role::Owner))
            })
    }

    pub fn rename_shared(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(VaultError::InvalidDisplayName);
        }
        self.ensure_exchange_committed()?;
        self.recover_sync()?;
        if self
            .profile_summary()
            .is_some_and(|profile| profile.space_name == name)
        {
            let replay = self.sync_replay_state()?.ok_or(Error::UnauthorizedDevice)?;
            replay.require_writer()?;
            if replay.role != Some(Role::Owner) {
                return Err(Error::InsufficientRole.into());
            }
            return self.save();
        }
        self.commit_sync(Transaction {
            mutations: Vec::new(),
            rebased_from: None,
            authored_at: None,
            space_name: Some(name.to_owned()),
            space_icon: None,
        })
    }

    pub fn icon(&self) -> Option<&SpaceIcon> {
        if let Some(snapshot) = &self.editor_view {
            return snapshot.icon.as_ref();
        }
        match self.profile.as_ref() {
            Some(profile) => profile.shared_icon.as_ref(),
            None => self.local_icon.as_ref(),
        }
    }

    pub fn set_icon(&mut self, icon: Option<SpaceIcon>) -> Result<()> {
        self.ensure_not_display_snapshot()?;
        if icon.as_ref().is_some_and(|icon| !icon.valid()) {
            return Err(Error::InvalidTransaction.into());
        }

        if self.profile.as_ref().is_some() {
            self.ensure_exchange_committed()?;
            self.recover_sync()?;
            if self.icon() == icon.as_ref() {
                let replay = self.sync_replay_state()?.ok_or(Error::UnauthorizedDevice)?;
                replay.require_writer()?;
                if replay.role != Some(Role::Owner) {
                    return Err(Error::InsufficientRole.into());
                }
                return self.save();
            }
            return self.commit_sync(Transaction {
                mutations: Vec::new(),
                rebased_from: None,
                authored_at: None,
                space_name: None,
                space_icon: Some(icon.map_or(SpaceIconChange::Clear, SpaceIconChange::Set)),
            });
        }

        if self.icon() == icon.as_ref() {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.local_icon, icon);
        // Once the file is replaced it holds the new icon, so memory keeps it.
        match self.save_checked() {
            Ok(confirmed) => confirmed,
            Err(error) => {
                self.local_icon = previous;
                Err(error)
            }
        }
    }

    /// Changes the personal name used by this profile for future exchanges.
    pub fn set_username(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(VaultError::InvalidDisplayName);
        }
        self.ensure_not_display_snapshot()?;
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        if profile.username == name {
            return Ok(());
        }
        let previous = std::mem::replace(&mut profile.username, name.to_owned());
        match self.save_checked() {
            Ok(confirmed) => confirmed,
            Err(error) => {
                self.profile.as_mut().unwrap().username = previous;
                Err(error)
            }
        }
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
        let previous = if name.is_empty() {
            profile.participant_names.remove(&user)
        } else {
            profile.participant_names.insert(user, name.to_owned())
        };
        match self.save_checked() {
            Ok(confirmed) => confirmed,
            Err(error) => {
                let profile = self.profile.as_mut().expect("profile retained during save");
                if let Some(previous) = previous {
                    profile.participant_names.insert(user, previous);
                } else {
                    profile.participant_names.remove(&user);
                }
                Err(error)
            }
        }
    }
}

// Fault injection and serialized legacy-profile fixtures stay test-only.
#[cfg(test)]
impl Vault {
    pub(crate) fn test_profile(&self) -> Option<&crate::vault::BootstrapProfile> {
        self.profile.as_ref()
    }

    pub(crate) fn test_profile_mut(&mut self) -> Option<&mut crate::vault::BootstrapProfile> {
        self.profile.as_mut()
    }
}
