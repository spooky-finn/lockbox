//! Bounded public artifact files. Never overwrites a profile or existing path.

use crate::core::sync::MAX_INVITATION_BYTES;
use crate::infra::sync::Cancellation;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(crate) fn read(path: &Path, cancel: &Cancellation) -> Result<Vec<u8>, String> {
    cancel.check().map_err(|_| "File operation cancelled")?;
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
