//! Explicit file import/export; vault persistence never writes plaintext attachments.
use crate::{platform::files::create_new_private, records::entry::MAX_ATTACHMENT_BYTES};
use std::{
    fs::OpenOptions,
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
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Opening a FIFO must not block before the regular-file check.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;

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

/// Suggested save-panel name. Another member chose the stored name, so bidi
/// overrides and invisible characters that could disguise the extension are
/// dropped here; the stored name itself is never rewritten.
pub(crate) fn export_file_name(name: &str) -> String {
    let name = crate::sync::protocol::pairing::display_name(name);
    if matches!(name.trim(), "" | "." | "..") {
        return "attachment".into();
    }
    name
}

pub(crate) fn export(path: &Path, data: &[u8]) -> Result<(), String> {
    // Never follow a symlink or silently overwrite an existing file.
    let mut file = create_new_private(path).map_err(|e| e.to_string())?;
    quarantine(&file);

    if let Err(error) = file.write_all(data).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(path);
        return Err(error.to_string());
    }

    Ok(())
}

/// Mark the export as downloaded so Gatekeeper checks it before it runs.
/// Best effort: a filesystem without extended attributes still gets the file.
#[cfg(target_os = "macos")]
fn quarantine(file: &std::fs::File) {
    use std::os::fd::AsRawFd;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let value = format!("0081;{now:08x};lockbox;");
    // The descriptor is open, the name is NUL terminated and `value` outlives the call.
    unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            c"com.apple.quarantine".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn quarantine(_: &std::fs::File) {}

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

    #[test]
    fn export_name_cannot_hide_the_real_extension() {
        assert_eq!(export_file_name("invoice\u{202E}fdp.exe"), "invoicefdp.exe");
        assert_eq!(
            export_file_name("\u{2066}report\u{2069}\u{200B}.pdf.command"),
            "report.pdf.command"
        );
        assert_eq!(export_file_name("\u{202E}\u{200F}"), "attachment");
        assert_eq!(export_file_name(".\u{200B}"), "attachment");
        assert_eq!(export_file_name("Отчёт 2026.pdf"), "Отчёт 2026.pdf");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn exported_attachment_is_quarantined() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("saved.bin");
        export(&destination, b"payload").unwrap();
        let path = std::ffi::CString::new(destination.as_os_str().as_bytes()).unwrap();
        let mut value = [0u8; 256];
        // Both pointers stay valid for the call; the buffer length is passed.
        let len = unsafe {
            libc::getxattr(
                path.as_ptr(),
                c"com.apple.quarantine".as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        assert!(len > 0, "no quarantine attribute");
        assert!(value[..len as usize].starts_with(b"0081;"));
        assert_eq!(std::fs::read(destination).unwrap(), b"payload");
    }
}
