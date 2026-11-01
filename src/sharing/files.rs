//! Bounded public artifact files. Never overwrites a profile or existing path.

use crate::sync::{protocol::MAX_INVITATION_BYTES, transport::Cancellation};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(crate) fn read(path: &Path, cancel: &Cancellation) -> Result<Vec<u8>, String> {
    cancel.check().map_err(|_| "File operation cancelled")?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "Could not read sharing file")?;
    if !metadata.is_file() {
        return Err("Choose a regular sharing file".into());
    }
    if metadata.len() > MAX_INVITATION_BYTES as u64 {
        return Err("Sharing file is too large".into());
    }
    let file = File::open(path).map_err(|_| "Could not read sharing file")?;
    if !file
        .metadata()
        .map_err(|_| "Could not read sharing file")?
        .is_file()
    {
        return Err("Choose a regular sharing file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_INVITATION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read sharing file")?;
    cancel.check().map_err(|_| "File operation cancelled")?;
    if bytes.len() > MAX_INVITATION_BYTES {
        return Err("Sharing file is too large".into());
    }
    Ok(bytes)
}

pub(crate) fn write(path: &Path, bytes: &[u8], cancel: &Cancellation) -> Result<(), String> {
    cancel.check().map_err(|_| "File operation cancelled")?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Choose a new file name for sharing")?;
    let result = (|| {
        for chunk in bytes.chunks(64 * 1024) {
            cancel.check().map_err(|_| "File operation cancelled")?;
            file.write_all(chunk)
                .map_err(|_| "Could not save sharing file")?;
        }
        file.sync_all().map_err(|_| "Could not save sharing file")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use crate::sharing::files::*;

    #[test]
    fn public_export_never_overwrites_a_vault_or_an_existing_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.lbx");
        std::fs::write(&path, b"private encrypted profile").unwrap();
        assert!(write(&path, b"public artifact", &Cancellation::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"private encrypted profile");
        #[cfg(unix)]
        {
            let link = directory.path().join("artifact.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(write(&link, b"public artifact", &Cancellation::default()).is_err());
            assert!(read(&link, &Cancellation::default()).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"private encrypted profile");
        }
    }

    #[test]
    fn oversized_files_and_cancelled_operations_stop_before_import_or_export() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.json");
        File::create(&path)
            .unwrap()
            .set_len(MAX_INVITATION_BYTES as u64 + 1)
            .unwrap();
        assert!(read(&path, &Cancellation::default()).is_err());
        let cancel = Cancellation::default();
        cancel.cancel();
        let path = directory.path().join("cancelled.json");
        assert!(write(&path, b"public artifact", &cancel).is_err());
        assert!(!path.exists());
    }
}
