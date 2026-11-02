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
