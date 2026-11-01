//! Private files, exclusive process locks and durable encrypted-file replacement.
//! Directory synchronization remains explicit in the Vault witness orchestration.

use super::{
    Result, VaultError,
    format::{Header, PROFILE_VERSION},
};
use crate::infra::lease::ExclusiveFile;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(super) fn sibling(path: &Path, extension: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(extension);
    path.with_file_name(name)
}

pub(super) fn refresh_profile_snapshot(
    path: &Path,
    data: Vec<u8>,
    header: Header,
    lock: &Option<ExclusiveFile>,
) -> Result<(Vec<u8>, Header)> {
    if lock.is_none() {
        return Ok((data, header));
    }

    let data = fs::read(path)?;
    let header = Header::parse(&data)?;

    if !matches!(header.version, 3 | 5 | PROFILE_VERSION) {
        return Err(VaultError::Corrupted);
    }

    Ok((data, header))
}

/// Stable sidecar: replacing the encrypted file must not release the process lock.
pub(super) fn lock_profile(path: &Path) -> Result<ExclusiveFile> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canonical =
        parent.canonicalize()?.join(path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "profile filename required")
        })?);
    let canonical = if path.exists() {
        path.canonicalize()?
    } else {
        canonical
    };

    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let file = options.open(sibling(&canonical, "lock"))?;
    ExclusiveFile::acquire(file).map_err(|error| match error {
        fs::TryLockError::WouldBlock => VaultError::ProfileInUse,
        fs::TryLockError::Error(error) => VaultError::Io(error),
    })
}

/// Readable and writable by the owner only.
fn private_file(path: &Path) -> io::Result<File> {
    // Unlink stale sidecars themselves, then exclusively create the new file.
    // Never truncate a symlink target through a predictable temporary pathname.
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Makes the rename itself durable, not only the file contents.
pub(super) fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
        File::open(dir.unwrap_or(Path::new(".")))?.sync_all()?;
    }

    Ok(())
}

pub(super) fn create(path: &Path, header: &[u8], ciphertext: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            VaultError::AlreadyExists(path.to_path_buf())
        } else {
            VaultError::Io(error)
        }
    })?;
    let result = (|| {
        file.write_all(header)?;
        file.write_all(ciphertext)?;
        file.sync_all()
    })();

    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result?;
    Ok(())
}

pub(super) fn replace(path: &Path, header: &[u8], ciphertext: &[u8]) -> Result<()> {
    let tmp = sibling(path, "tmp");
    {
        let mut file = private_file(&tmp)?;
        file.write_all(header)?;
        file.write_all(ciphertext)?;
        file.sync_all()?;
    }

    if path.exists() {
        let backup = sibling(path, "bak");
        let backup_tmp = sibling(&backup, "tmp");
        let mut source = File::open(path)?;
        let mut target = private_file(&backup_tmp)?;
        io::copy(&mut source, &mut target)?;
        target.sync_all()?;
        fs::rename(&backup_tmp, &backup)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
