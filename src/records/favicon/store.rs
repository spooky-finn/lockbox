//! Bounded plaintext cache files with private permissions and atomic replacement.

use super::{CachedIcon, MAX_IMAGE_BYTES, raster_mime};
use crate::platform::files::create_new_private;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const MAX_CACHE_BYTES: u64 = 1_500_000;

/// Prepare a private cache directory before returning a hashed entry path.
pub(super) fn prepare_path(directory: &Path, website: &str) -> Option<PathBuf> {
    private_directory(directory).ok()?;
    Some(directory.join(format!("{:x}.json", Sha256::digest(website.as_bytes()))))
}

fn private_directory(path: &Path) -> std::io::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return Err(std::io::Error::other("favicon cache is not a directory"));
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other("favicon cache is not a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn valid_cached_image(image: &str) -> bool {
    let Some((prefix, data)) = image.split_once(',') else {
        return false;
    };
    let Some(mime) = prefix
        .strip_prefix("data:")
        .and_then(|prefix| prefix.strip_suffix(";base64"))
    else {
        return false;
    };
    raster_mime(mime)
        && STANDARD
            .decode(data)
            .is_ok_and(|bytes| !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES)
}

pub(super) fn load(path: &Path, now: u64) -> Option<CachedIcon> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_CACHE_BYTES {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .ok()?;
    }
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return None;
    }
    let entry: CachedIcon = serde_json::from_slice(&bytes).ok()?;
    (entry.fresh(now) && entry.image.as_deref().is_none_or(valid_cached_image)).then_some(entry)
}

pub(super) fn store(path: &Path, entry: &CachedIcon) -> std::io::Result<()> {
    if !entry.image.as_deref().is_none_or(valid_cached_image) {
        return Ok(());
    }
    let bytes = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Ok(());
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(std::io::Error::other)?;
    let temporary = path.with_extension(format!("{:x}.tmp", u128::from_le_bytes(random)));
    let result = (|| {
        {
            let mut file = create_new_private(&temporary)?;
            file.write_all(&bytes)?;
        }
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
