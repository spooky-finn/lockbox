//! Portable witness storage outside the vault's backup location. macOS uses the
//! Keychain backend instead; both use the same installation-wide file lease.

use super::{Error, Hash, Result, Witness, WitnessLease, WitnessStore};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub struct DirectoryWitness {
    root: PathBuf,
}

impl DirectoryWitness {
    /// The directory must be installation-local and backed up separately from
    /// profiles. Restoring both together cannot be detected by a local witness.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl WitnessStore for DirectoryWitness {
    fn acquire(&self, device: Hash) -> Result<Box<dyn WitnessLease>> {
        let lock = lock(&self.root, device)?;
        Ok(Box::new(Lease {
            path: self.root.join(format!("{device}.json")),
            lock,
        }))
    }
}

struct Lease {
    path: PathBuf,
    lock: SignerLock,
}

impl WitnessLease for Lease {
    fn load(&self) -> Result<Option<Witness>> {
        self.lock.check()?;
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Error::Unavailable),
        };

        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Unavailable)?;
        decode(&bytes).map(Some)
    }

    fn replace(&mut self, expected: Option<Witness>, next: Witness) -> Result<()> {
        if self.load()? != expected {
            return Err(Error::Rollback);
        }

        let bytes = serde_json::to_vec(&next).map_err(|_| Error::Unavailable)?;
        self.lock.check()?;
        let temporary = self.path.with_extension("tmp");

        match fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Unavailable),
        }

        let mut file = options()
            .create_new(true)
            .open(&temporary)
            .map_err(|_| Error::Unavailable)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| Error::Unavailable)?;
        fs::rename(&temporary, &self.path).map_err(|_| Error::Unavailable)?;
        sync_directory(self.path.parent().ok_or(Error::Unavailable)?)
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<Witness> {
    if bytes.len() > 4096 {
        return Err(Error::Unavailable);
    }
    serde_json::from_slice(bytes).map_err(|_| Error::Unavailable)
}

pub(super) struct SignerLock {
    path: PathBuf,
    file: File,
}

impl SignerLock {
    /// A removed/replaced lock path must not leave an old lease usable while a
    /// different process locks a new inode at the same pathname.
    pub(super) fn check(&self) -> Result<()> {
        let current = fs::symlink_metadata(&self.path).map_err(|_| Error::Unavailable)?;
        let held = self.file.metadata().map_err(|_| Error::Unavailable)?;

        if !current.is_file() {
            return Err(Error::Unavailable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            if current.dev() != held.dev() || current.ino() != held.ino() {
                return Err(Error::Unavailable);
            }
        }
        #[cfg(not(unix))]
        let _ = held;
        Ok(())
    }
}

pub(super) fn lock(root: &Path, device: Hash) -> Result<SignerLock> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(root).map_err(|_| Error::Unavailable)?;
    let path = root.join(format!("{device}.lock"));
    let mut lease_options = options();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE; the lock alone excludes deletion.
        lease_options.share_mode(0x1 | 0x2);
    }

    let file = lease_options
        .truncate(false)
        .open(&path)
        .map_err(|_| Error::Unavailable)?;
    file.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => Error::InUse,
        fs::TryLockError::Error(_) => Error::Unavailable,
    })?;
    let lock = SignerLock { path, file };
    lock.check()?;
    Ok(lock)
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::Unavailable)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
