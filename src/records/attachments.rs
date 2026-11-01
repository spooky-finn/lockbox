//! Explicit file import/export; vault persistence never writes plaintext attachments.
use crate::records::entry::MAX_ATTACHMENT_BYTES;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;

pub(crate) fn read(path: &Path) -> Result<(String, Zeroizing<Vec<u8>>), String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid attachment file name")?
        .to_owned();
    let file = File::open(path).map_err(|e| e.to_string())?;

    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Select a regular file".into());
    }

    let mut data = Zeroizing::new(Vec::new());
    file.take(MAX_ATTACHMENT_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;

    if data.len() > MAX_ATTACHMENT_BYTES {
        return Err("Attachments must total at most 1 MiB per entry".into());
    }

    Ok((name, data))
}

pub(crate) fn export(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Never follow a symlink or silently overwrite an existing file.
    let mut file = options.open(path).map_err(|e| e.to_string())?;

    if let Err(error) = file.write_all(data).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(path);
        return Err(error.to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::records::attachments::*;
    #[test]
    fn import_binary_export_and_refuse_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        std::fs::write(&source, [0, 255, 42]).unwrap();
        let (name, data) = read(&source).unwrap();
        assert_eq!(name, "source.bin");
        assert_eq!(*data, [0, 255, 42]);
        let destination = dir.path().join("saved.bin");
        export(&destination, &data).unwrap();
        assert!(export(&destination, b"replacement").is_err());
        assert_eq!(std::fs::read(destination).unwrap(), *data);
        std::fs::write(source.clone(), vec![0; MAX_ATTACHMENT_BYTES + 1]).unwrap();
        assert!(read(&source).is_err());
    }
}
