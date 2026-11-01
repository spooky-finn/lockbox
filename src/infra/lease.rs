//! The owning session releases its advisory lock explicitly. A duplicated or
//! briefly inherited descriptor must not extend a finished session's lease.

use std::{
    fs::{File, Metadata, TryLockError},
    io,
};

pub(crate) struct ExclusiveFile(File);

impl ExclusiveFile {
    pub(crate) fn acquire(file: File) -> Result<Self, TryLockError> {
        file.try_lock()?;
        Ok(Self(file))
    }

    pub(crate) fn metadata(&self) -> io::Result<Metadata> {
        self.0.metadata()
    }
}

impl Drop for ExclusiveFile {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_descriptors_do_not_extend_the_owning_sessions_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("signer.lock");
        let lock = ExclusiveFile::acquire(File::create(&path).unwrap()).unwrap();
        let inherited = lock.0.try_clone().unwrap();
        assert!(ExclusiveFile::acquire(File::open(&path).unwrap()).is_err());
        drop(lock);
        let next = ExclusiveFile::acquire(File::open(&path).unwrap()).unwrap();
        drop(inherited);
        assert!(ExclusiveFile::acquire(File::open(&path).unwrap()).is_err());
        drop(next);
        assert!(ExclusiveFile::acquire(File::open(&path).unwrap()).is_ok());
    }
}
