//! File creation shared by native exports and encrypted persistence.

use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

/// Create a write-only file with owner-only Unix permissions, refusing existing
/// paths, including symlinks. Callers own writing, synchronization and cleanup.
pub(crate) fn create_new_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Rename within a directory without replacing another directory entry.
pub(crate) fn rename_new(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};

        let source = CString::new(source.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        // Both C strings remain valid for the duration of the call.
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        if result == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileW(source: *const u16, destination: *const u16) -> i32;
        }

        let wide_path = |path: &Path| {
            let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
            if wide.contains(&0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid filename",
                ));
            }
            wide.push(0);
            Ok(wide)
        };
        let source = wide_path(source)?;
        let destination = wide_path(destination)?;
        // Both UTF-16 paths are NUL terminated and live across the call.
        if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } != 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        std::fs::hard_link(source, destination)?;
        if let Err(error) = std::fs::remove_file(source) {
            let _ = std::fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }
}
