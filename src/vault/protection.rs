//! Bind file commits to independent installation evidence. The signer lease is
//! held for the unlocked session, including when the profile is read-only.

use crate::{
    sync::protocol::Hash,
    vault::{
        anchor::{self, Error, FileGeneration, Reconciliation, Result, Witness},
        witness::{WitnessLease, WitnessStore},
    },
};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path, sync::Arc};

pub(super) struct Protection {
    store: Arc<dyn WitnessStore>,
    device: Hash,
    profile: Hash,
    binding: Hash,
    lease: Option<Box<dyn WitnessLease>>,
    file: Option<(FileGeneration, Hash)>,
    pub(super) error: Option<Error>,
}

impl Protection {
    pub(super) fn create(store: Arc<dyn WitnessStore>, device: Hash, salt: &[u8]) -> Result<Self> {
        let profile = profile_id(salt);
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| Error::Unavailable)?;
        let binding = Hash(bytes);
        let mut lease = store.acquire(device)?;
        lease.replace(None, Witness::reservation(profile, binding))?;
        Ok(Self {
            store,
            device,
            profile,
            binding,
            lease: Some(lease),
            file: None,
            error: None,
        })
    }

    pub(super) fn open(
        store: Arc<dyn WitnessStore>,
        device: Hash,
        salt: &[u8],
        generation: FileGeneration,
        digest: Hash,
        path: &Path,
    ) -> Result<Self> {
        let mut protection = Self {
            store,
            device,
            profile: profile_id(salt),
            binding: generation.binding,
            lease: None,
            file: Some((generation, digest)),
            error: None,
        };
        // A second live copy cannot acquire a second signer. Other witness
        // failures preserve readable contents and are retried explicitly later.
        if let Err(Error::InUse) = protection.ensure_ready(path) {
            return Err(Error::InUse);
        }

        Ok(protection)
    }

    pub(super) fn has_file(&self) -> bool {
        self.file.is_some()
    }

    pub(super) fn ensure_ready(&mut self, path: &Path) -> Result<()> {
        if self.file.is_none() {
            self.error = Some(Error::Unavailable);
            return Err(Error::Unavailable);
        }

        self.checked(path).map(|_| ())
    }

    pub(super) fn prepare_save(&mut self, path: &Path) -> Result<FileGeneration> {
        let result = self.checked(path).and_then(|witness| witness.next());
        self.error = result.as_ref().err().copied();
        result
    }

    fn checked(&mut self, path: &Path) -> Result<Witness> {
        let result = self.reconcile(path);
        self.error = result.as_ref().err().copied();
        result
    }

    fn reconcile(&mut self, path: &Path) -> Result<Witness> {
        if self.lease.is_none() {
            self.lease = Some(self.store.acquire(self.device)?);
        }

        let lease = self.lease.as_mut().ok_or(Error::Unavailable)?;
        let saved = lease.load()?;
        let Some((file, digest)) = self.file else {
            let reservation = Witness::reservation(self.profile, self.binding);
            return match saved {
                Some(witness) if witness == reservation => Ok(witness),
                Some(_) => Err(Error::Rollback),
                None => Err(Error::MissingBinding),
            };
        };

        if digest_file(path)? != digest {
            return Err(Error::Rollback);
        }

        match anchor::reconcile(self.profile, file, digest, saved)? {
            Reconciliation::Current => saved.ok_or(Error::MissingBinding),
            Reconciliation::Advance(next) => {
                // Synchronize the exact file and containing rename before making
                // the witness prohibit recovery of the previous generation.
                File::open(path)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| Error::Unavailable)?;
                crate::vault::sync_dir(path).map_err(|_| Error::Unavailable)?;
                lease.replace(saved, next)?;
                Ok(next)
            }
        }
    }

    /// Called immediately after rename, even if directory sync or witness update
    /// subsequently fails. Never allow another save to reuse that generation.
    pub(super) fn committed(&mut self, generation: FileGeneration, digest: Hash) {
        self.file = Some((generation, digest));
        self.error = Some(Error::Unavailable);
    }
}

fn profile_id(salt: &[u8]) -> Hash {
    Hash::digest(b"lockbox.profile-id.v1\0", salt)
}

pub(super) fn digest_parts(header: &[u8], ciphertext: &[u8]) -> Hash {
    let mut digest = Sha256::new();
    digest.update(b"lockbox.profile-file.v1\0");
    digest.update(header);
    digest.update(ciphertext);
    Hash(digest.finalize().into())
}

pub(super) fn digest_file(path: &Path) -> Result<Hash> {
    let mut file = File::open(path).map_err(|_| Error::Unavailable)?;
    let mut digest = Sha256::new();
    digest.update(b"lockbox.profile-file.v1\0");
    let mut bytes = [0; 32 * 1024];
    loop {
        let count = file.read(&mut bytes).map_err(|_| Error::Unavailable)?;

        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }

    Ok(Hash(digest.finalize().into()))
}
