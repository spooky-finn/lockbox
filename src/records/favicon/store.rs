//! Bounded plaintext cache files with private permissions and atomic replacement.
//!
//! File names are HMAC digests of the website under a random key kept in the cache
//! directory, so a list of names cannot be matched against hashes of guessed sites.
//! The key sits beside the entries: this keeps the sites out of plain sight, not away
//! from someone who can read the whole directory.

use super::{CachedIcon, MAX_IMAGE_BYTES, raster_mime};
use crate::platform::files::create_new_private;
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub(super) const MAX_CACHE_BYTES: u64 = 1_500_000;
const KEY: &str = "key";
const KEY_BYTES: usize = 32;
const ENTRY: &str = "json";

type Key = Zeroizing<[u8; KEY_BYTES]>;

/// Prepare a private cache directory and its key before returning a keyed entry path.
pub(super) fn prepare_path(directory: &Path, website: &str) -> Option<PathBuf> {
    private_directory(directory).ok()?;
    let key = key(directory).ok()?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_ref()).expect("HMAC accepts any key length");
    mac.update(website.as_bytes());
    let name: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Some(directory.join(format!("{name}.{ENTRY}")))
}

fn open_regular(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(file)
}

/// `None` only when no key exists; a damaged key is an `InvalidData` error.
fn read_key(directory: &Path) -> io::Result<Option<Key>> {
    let mut file = match open_regular(&directory.join(KEY)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() != KEY_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "damaged favicon cache key",
        ));
    }
    let mut key = Zeroizing::new([0; KEY_BYTES]);
    file.read_exact(key.as_mut())?;
    Ok(Some(key))
}

fn key(directory: &Path) -> io::Result<Key> {
    let path = directory.join(KEY);
    match read_key(directory) {
        Ok(Some(key)) => return Ok(key),
        Ok(None) => {}
        // Another process may have removed the damaged key first.
        Err(error) if error.kind() == io::ErrorKind::InvalidData => match fs::remove_file(&path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        },
        Err(error) => return Err(error),
    }
    let mut key = Zeroizing::new([0; KEY_BYTES]);
    getrandom::fill(key.as_mut()).map_err(io::Error::other)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let temporary = directory.join(format!("{KEY}.{:x}.tmp", u128::from_le_bytes(random)));
    let staged = (|| {
        let mut file = create_new_private(&temporary)?;
        file.write_all(key.as_ref())?;
        file.sync_all()
    })();
    // A link never replaces the key another process published meanwhile.
    let linked = staged.and_then(|()| fs::hard_link(&temporary, &path));
    let _ = fs::remove_file(&temporary);
    match linked {
        Ok(()) => {
            // Entries named under an earlier key, or unkeyed by older versions, never match again.
            remove_entries(directory);
            Ok(key)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            read_key(directory)?.ok_or_else(|| io::Error::other("favicon cache key disappeared"))
        }
        Err(error) => Err(error),
    }
}

fn remove_entries(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == ENTRY) {
            let _ = fs::remove_file(path);
        }
    }
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
