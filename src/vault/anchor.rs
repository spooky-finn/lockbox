//! Deterministic reconciliation of an encrypted profile and independent witness.
//! A witness is local rollback evidence, not a hardware monotonic counter.

use crate::sync::protocol::Hash;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("profile witness is unavailable; signing and exchange are paused")]
    Unavailable,
    #[error(
        "Lockbox cannot access its local security folder. Check folder permissions and restart Lockbox outside the restricted sandbox."
    )]
    StorageAccessDenied,
    #[error("this signing identity is already open on this installation")]
    InUse,
    #[error("profile has no binding on this installation; use a new admitted device")]
    MissingBinding,
    #[error("profile rollback or inconsistent witness detected; use a new admitted device")]
    Rollback,
    #[error("legacy signing profile requires a new admitted device")]
    Legacy,
    #[error("profile generation limit reached")]
    Exhausted,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Stored inside the authenticated, encrypted profile. No signing secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileGeneration {
    pub binding: Hash,
    pub generation: u64,
    pub previous: Option<Hash>,
}

impl FileGeneration {
    pub fn valid(&self) -> bool {
        self.generation > 0 && (self.generation == 1) == self.previous.is_none()
    }
}

/// Independently persisted installation record, addressed by device fingerprint.
/// Generation zero reserves a new identity before its first profile commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub profile: Hash,
    pub binding: Hash,
    pub generation: u64,
    pub digest: Option<Hash>,
}

impl Witness {
    pub fn reservation(profile: Hash, binding: Hash) -> Self {
        Self {
            profile,
            binding,
            generation: 0,
            digest: None,
        }
    }

    pub fn next(&self) -> Result<FileGeneration> {
        Ok(FileGeneration {
            binding: self.binding,
            generation: self.generation.checked_add(1).ok_or(Error::Exhausted)?,
            previous: self.digest,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconciliation {
    Current,
    /// Exactly one durable file commit happened before its witness update.
    Advance(Witness),
}

/// Only equality or the explicitly linked next generation can resume signing.
/// Missing evidence never authorizes enrollment of a copied signing identity.
pub fn reconcile(
    profile: Hash,
    file: FileGeneration,
    digest: Hash,
    witness: Option<Witness>,
) -> Result<Reconciliation> {
    let witness = witness.ok_or(Error::MissingBinding)?;

    if !file.valid()
        || witness.profile != profile
        || witness.binding != file.binding
        || (witness.generation == 0) != witness.digest.is_none()
    {
        return Err(Error::Rollback);
    }

    if witness.generation == file.generation && witness.digest == Some(digest) {
        return Ok(Reconciliation::Current);
    }

    if witness.generation.checked_add(1) == Some(file.generation) && file.previous == witness.digest
    {
        return Ok(Reconciliation::Advance(Witness {
            profile,
            binding: file.binding,
            generation: file.generation,
            digest: Some(digest),
        }));
    }
    Err(Error::Rollback)
}

#[cfg(test)]
mod tests {
    use crate::vault::anchor::*;

    #[test]
    fn only_the_current_or_exact_linked_next_generation_is_reconcilable() {
        let profile = Hash([1; 32]);
        let binding = Hash([2; 32]);
        let previous = Hash([3; 32]);
        let digest = Hash([4; 32]);
        let file = FileGeneration {
            binding,
            generation: 2,
            previous: Some(previous),
        };

        let witness = |generation, digest| Witness {
            profile,
            binding,
            generation,
            digest,
        };
        assert_eq!(
            reconcile(profile, file, digest, None),
            Err(Error::MissingBinding)
        );
        assert_eq!(
            reconcile(profile, file, digest, Some(witness(2, Some(digest)))),
            Ok(Reconciliation::Current)
        );
        assert_eq!(
            reconcile(profile, file, digest, Some(witness(1, Some(previous)))),
            Ok(Reconciliation::Advance(witness(2, Some(digest))))
        );

        for record in [
            witness(0, None),
            witness(1, Some(Hash([9; 32]))),
            witness(2, Some(previous)),
            witness(3, Some(digest)),
        ] {
            assert_eq!(
                reconcile(profile, file, digest, Some(record)),
                Err(Error::Rollback)
            );
        }

        let mut record = witness(2, Some(digest));
        record.profile = Hash([9; 32]);
        assert_eq!(
            reconcile(profile, file, digest, Some(record)),
            Err(Error::Rollback)
        );
        record = witness(2, Some(digest));
        record.binding = Hash([9; 32]);
        assert_eq!(
            reconcile(profile, file, digest, Some(record)),
            Err(Error::Rollback)
        );
    }

    #[test]
    fn enrollment_reservation_is_one_time_and_generation_cannot_wrap() {
        let reserved = Witness::reservation(Hash([1; 32]), Hash([2; 32]));
        let first = reserved.next().unwrap();
        assert!(first.valid());
        let digest = Hash([3; 32]);
        assert!(matches!(
            reconcile(reserved.profile, first, digest, Some(reserved)),
            Ok(Reconciliation::Advance(_))
        ));
        assert_eq!(
            reconcile(reserved.profile, first, digest, None),
            Err(Error::MissingBinding)
        );
        let exhausted = Witness {
            generation: u64::MAX,
            digest: Some(digest),
            ..reserved
        };
        assert_eq!(exhausted.next(), Err(Error::Exhausted));
    }
}
