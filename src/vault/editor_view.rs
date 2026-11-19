//! The editor can read committed search metadata while the writer owns the vault.

use crate::{
    records::entry::Entry,
    sync::identity::Key,
    vault::{ProfileSummary, Result, Vault, VaultError, anchor},
};
use zeroize::Zeroizing;

/// Search and public status while the writer owns the actual vault. No keys,
/// journal, record secrets, attachments, or leases are copied into the view.
pub(super) struct EditorView {
    profile: Option<EditorMetadata>,
    signing_error: Option<anchor::Error>,
    pending_uploads: Option<usize>,
    has_certificate: bool,
}

impl EditorView {
    pub(super) fn profile_summary(&self) -> Option<ProfileSummary<'_>> {
        self.profile.as_ref().map(EditorMetadata::summary)
    }

    pub(super) fn signing_error(&self) -> Option<anchor::Error> {
        self.signing_error
    }
}

struct EditorMetadata {
    username: String,
    space_name: String,
    user_id: Key,
    device_id: Key,
    space_id: Key,
    sync_enabled: bool,
}

impl EditorMetadata {
    fn new(profile: ProfileSummary<'_>) -> Self {
        Self {
            username: profile.username.into(),
            space_name: profile.space_name.into(),
            user_id: profile.user_id,
            device_id: profile.device_id,
            space_id: profile.space_id,
            sync_enabled: profile.sync_enabled,
        }
    }

    fn summary(&self) -> ProfileSummary<'_> {
        ProfileSummary {
            username: &self.username,
            space_name: &self.space_name,
            user_id: self.user_id,
            device_id: self.device_id,
            space_id: self.space_id,
            sync_enabled: self.sync_enabled,
        }
    }
}

impl Vault {
    pub(super) fn ensure_not_display_snapshot(&self) -> Result<()> {
        if self.editor_view.is_some() {
            return Err(VaultError::ProfileInUse);
        }
        Ok(())
    }

    /// The UI sees the last committed records while the actual vault is on
    /// the writer. This value cannot save, enroll a signer, or acquire a lease.
    pub(crate) fn editor_snapshot(&self) -> Self {
        Self {
            records_revision: self.records_revision,
            journal_check: Default::default(),
            path: self.path.clone(),
            kdf: self.kdf,
            salt: self.salt,
            check: self.check,
            key: Zeroizing::new([0; 32]),
            recovery: None,
            entries: self
                .entries
                .iter()
                .map(|entry| {
                    let mut row = Entry::default();
                    row.id.clone_from(&entry.id);
                    row.title.clone_from(&entry.title);
                    row.username.clone_from(&entry.username);
                    row.url.clone_from(&entry.url);
                    row.folder_id.clone_from(&entry.folder_id);
                    row
                })
                .collect(),
            history: Default::default(),
            saved_entries: Default::default(),
            folders: self.folders.clone(),
            profile: None.into(),
            pending_exchange: None,
            editor_view: Some(EditorView {
                profile: self.profile_summary().map(EditorMetadata::new),
                signing_error: self.signing_error(),
                pending_uploads: self.pending_uploads(),
                has_certificate: self.has_device_certificate(),
            }),
            _profile_lock: None,
            witnesses: self.witnesses.clone(),
            protection: None,
        }
    }

    pub(crate) fn has_device_certificate(&self) -> bool {
        if let Some(snapshot) = &self.editor_view {
            snapshot.has_certificate
        } else {
            self.profile
                .as_ref()
                .is_some_and(|profile| profile.certificate.is_some())
        }
    }

    pub(crate) fn pending_uploads(&self) -> Option<usize> {
        if let Some(snapshot) = &self.editor_view {
            snapshot.pending_uploads
        } else {
            self.sync_state().map(|sync| sync.outbox.len())
        }
    }
}
