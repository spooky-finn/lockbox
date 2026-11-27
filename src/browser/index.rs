//! Advisory HTTPS origin index for the browser popup. Never used to authorize credential release.
//!
//! Files hold HMAC digests of origins under a device key. Only the browser relay creates
//! that key, so a device that never ran the extension keeps no index at all. The key sits
//! beside the digests because the relay answers with Lockbox closed and locked: this keeps
//! the list out of plain sight, not away from a same-user process that tests guessed sites.
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const KEY: &str = "key";
const KEY_BYTES: usize = 32;
const INDEX: &str = "sites";
/// Plaintext origin lists written by older versions.
const LEGACY: &str = "json";

type Key = Zeroizing<[u8; KEY_BYTES]>;

fn directory() -> io::Result<PathBuf> {
    #[cfg(not(test))]
    let root = crate::vault::paths::private_root(
        &dirs::home_dir().ok_or_else(|| io::Error::other("no home directory"))?,
    )?;
    #[cfg(test)]
    let root = {
        static ROOT: std::sync::LazyLock<tempfile::TempDir> =
            std::sync::LazyLock::new(|| tempfile::tempdir().unwrap());
        ROOT.path().to_owned()
    };
    fs::create_dir_all(&root)?;
    let directory = root.join("browser-sites");
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() {
        return Err(io::Error::other("unsafe site index directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::other("unsafe site index permissions"));
        }
    }
    Ok(directory)
}

fn origins(entries: &[crate::records::entry::Entry]) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|entry| {
            !entry.password.is_empty()
                && entry.password.len() <= 4096
                && entry.username.len() <= 4096
        })
        .filter_map(|entry| {
            super::origin(&entry.url)
                .ok()
                .map(|o| o.ascii_serialization())
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn digest(key: &Key, label: &[u8], value: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_ref()).expect("HMAC accepts any key length");
    mac.update(label);
    mac.update(&[0]);
    mac.update(value.as_bytes());
    hex(&mac.finalize().into_bytes())
}

fn open_regular(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
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
            "damaged site index key",
        ));
    }
    let mut key = Zeroizing::new([0; KEY_BYTES]);
    file.read_exact(key.as_mut())?;
    Ok(Some(key))
}

/// Private temporary file; `.tmp` names are never read as an index or a key.
fn stage(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let temporary = directory.join(format!("{name}-{}.tmp", hex(&random)));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    match result {
        Ok(()) => Ok(temporary),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

/// Keeps going past failures so one stuck file does not shield the rest.
fn remove_all(directory: &Path, extension: &str) -> io::Result<()> {
    let mut result = Ok(());
    for entry in fs::read_dir(directory)?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == extension)
            && let Err(error) = fs::remove_file(&path)
            && error.kind() != io::ErrorKind::NotFound
        {
            result = Err(error);
        }
    }
    result
}

/// The relay's first query creates the key and so opts this device in to the index.
fn create_key(directory: &Path) -> io::Result<Key> {
    let path = directory.join(KEY);
    match read_key(directory) {
        Ok(Some(key)) => return Ok(key),
        Ok(None) => {}
        // Another relay may have removed the damaged key first.
        Err(error) if error.kind() == io::ErrorKind::InvalidData => match fs::remove_file(&path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        },
        Err(error) => return Err(error),
    }
    let mut key = Zeroizing::new([0; KEY_BYTES]);
    getrandom::fill(key.as_mut()).map_err(io::Error::other)?;
    let temporary = stage(directory, KEY, key.as_ref())?;
    // A link never replaces the key another relay published meanwhile.
    let linked = fs::hard_link(&temporary, &path);
    let _ = fs::remove_file(&temporary);
    match linked {
        Ok(()) => {
            // Digests made under an earlier key can never match again.
            let _ = remove_all(directory, INDEX);
            Ok(key)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            read_key(directory)?.ok_or_else(|| io::Error::other("site index key disappeared"))
        }
        Err(error) => Err(error),
    }
}

fn index_path(directory: &Path, key: &Key, id: &str) -> PathBuf {
    directory.join(format!("{}.{INDEX}", digest(key, b"vault", id)))
}

fn publish(directory: &Path, key: &Key, id: &str, origins: &BTreeSet<String>) -> io::Result<()> {
    let destination = index_path(directory, key, id);
    let sites: BTreeSet<String> = origins
        .iter()
        .map(|origin| digest(key, b"origin", origin))
        .collect();
    let bytes = serde_json::to_vec(&sites)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("site index too large"));
    }
    if fs::read(&destination).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    let temporary = stage(directory, INDEX, &bytes)?;
    let result = fs::rename(&temporary, &destination);
    let _ = fs::remove_file(temporary);
    result
}

fn update(directory: &Path, id: &str, origins: &BTreeSet<String>) -> io::Result<()> {
    // Plaintext lists from older versions go on the first refresh of any vault.
    let legacy = remove_all(directory, LEGACY);
    let published = read_key(directory).and_then(|key| match key {
        Some(key) => publish(directory, &key, id, origins),
        // The extension never asked on this device; leftover digests cannot match.
        None => remove_all(directory, INDEX),
    });
    legacy.and(published)
}

fn remove(directory: &Path, id: &str) -> io::Result<()> {
    let legacy = remove_all(directory, LEGACY);
    let own = read_key(directory).and_then(|key| {
        let Some(key) = key else { return Ok(()) };
        match fs::remove_file(index_path(directory, &key, id)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    });
    legacy.and(own)
}

/// Cache failure must not turn a successful encrypted vault save into a failure.
pub(crate) fn refresh(vault: &crate::vault::Vault) {
    if let Err(error) =
        directory().and_then(|directory| update(&directory, &vault.id(), &origins(vault.entries())))
    {
        eprintln!("Could not update browser site index: {error}");
    }
}

/// A deleted vault must not leave its sites behind; failure must not undo the deletion.
pub(crate) fn forget(id: &str) {
    if let Err(error) = directory().and_then(|directory| remove(&directory, id)) {
        eprintln!("Could not remove browser site index: {error}");
    }
}

fn contains(directory: &Path, origin: &str) -> bool {
    let Ok(origin) = super::origin(origin).map(|o| o.ascii_serialization()) else {
        return false;
    };
    let Ok(key) = create_key(directory) else {
        return false;
    };
    let wanted = digest(&key, b"origin", &origin);
    let Ok(files) = fs::read_dir(directory) else {
        return false;
    };
    files.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != INDEX) {
            return false;
        }
        let read = || -> io::Result<bool> {
            let file = open_regular(&path)?;
            if file.metadata()?.len() > MAX_BYTES {
                return Ok(false);
            }
            let mut bytes = Vec::new();
            file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Ok(false);
            }
            let sites: BTreeSet<String> = serde_json::from_slice(&bytes)?;
            Ok(sites.contains(&wanted))
        };
        read().unwrap_or(false)
    })
}

pub(crate) fn has_match(origin: &str) -> bool {
    directory().is_ok_and(|directory| contains(&directory, origin))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(directory: &Path) -> BTreeSet<String> {
        fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    #[test]
    fn saved_origins_never_reach_disk_in_plaintext_and_deletion_drops_the_index() {
        let vault_directory = tempfile::tempdir().unwrap();
        let mut unlocked = crate::desktop::test_support::unlocked(&vault_directory);
        let host = format!("probe-{}.example", unlocked.vault.id());
        let mut entry = crate::records::entry::Entry::new();
        entry.url = format!("https://{host}/login");
        entry.password = "fictional-password".into();
        unlocked.vault.test_entries_mut().push(entry);
        has_match("https://unrelated.example");
        unlocked.vault.save().unwrap();
        assert!(has_match(&format!("https://{host}")));
        for file in fs::read_dir(directory().unwrap()).unwrap() {
            let bytes = fs::read(file.unwrap().path()).unwrap_or_default();
            assert!(!String::from_utf8_lossy(&bytes).contains(&host));
        }
        unlocked.vault.delete_local_files().unwrap();
        assert!(!has_match(&format!("https://{host}")));
    }

    #[test]
    fn vault_save_and_unlock_refresh_index_and_drop_preserves_it() {
        // The relay's first query opts this device in.
        has_match("https://unrelated.example");
        let vault_directory = tempfile::tempdir().unwrap();
        let mut unlocked = crate::desktop::test_support::unlocked(&vault_directory);
        let id = unlocked.vault.id();
        let website = format!("https://vault-{id}.example");
        let mut entry = crate::records::entry::Entry::new();
        entry.url = format!("{website}/private");
        entry.password = "fictional-password".into();
        unlocked.vault.test_entries_mut().push(entry);
        unlocked.vault.save().unwrap();
        let key = unlocked.vault.export_key();
        let path = unlocked.vault.path().to_owned();
        drop(unlocked);
        assert!(has_match(&website), "closed vault remains discoverable");

        // Simulate an existing vault whose browser index has not been populated.
        update(&directory().unwrap(), &id, &BTreeSet::new()).unwrap();
        assert!(!has_match(&website));
        let mut vault = crate::vault::Vault::open_with_key(path, &key).unwrap();
        assert!(has_match(&website), "unlock migrates existing entries");
        vault.test_entries_mut().clear();
        vault.save().unwrap();
        drop(vault);
        assert!(
            !has_match(&website),
            "deleted password disappears after save"
        );
    }

    #[test]
    fn no_index_until_the_relay_asks_and_plaintext_lists_are_removed() {
        let directory = tempfile::tempdir().unwrap();
        let sites = BTreeSet::from(["https://saved.example".to_owned()]);
        // An older version left plaintext lists, one of them for a vault deleted long ago.
        fs::write(
            directory.path().join("old.json"),
            "[\"https://saved.example\"]",
        )
        .unwrap();
        fs::write(
            directory.path().join("gone.json"),
            "[\"https://gone.example\"]",
        )
        .unwrap();

        update(directory.path(), "vault", &sites).unwrap();
        assert!(
            files(directory.path()).is_empty(),
            "nothing without the extension"
        );

        assert!(!contains(directory.path(), "https://saved.example"));
        assert_eq!(files(directory.path()), BTreeSet::from([KEY.to_owned()]));
        update(directory.path(), "vault", &sites).unwrap();
        assert!(contains(directory.path(), "https://saved.example"));
        assert_eq!(files(directory.path()).len(), 2);

        // Plaintext from an older build is never consulted and goes on the next save.
        fs::write(
            directory.path().join("old.json"),
            "[\"https://gone.example\"]",
        )
        .unwrap();
        assert!(!contains(directory.path(), "https://gone.example"));
        update(directory.path(), "vault", &sites).unwrap();
        assert!(!directory.path().join("old.json").exists());
    }

    #[test]
    fn persists_without_vault_and_replaces_each_vault_independently() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!contains(directory.path(), "https://saved.example"));
        let first = BTreeSet::from(["https://saved.example".to_owned()]);
        let other = BTreeSet::from(["https://other.example:444".to_owned()]);
        update(directory.path(), "first", &first).unwrap();
        update(directory.path(), "second", &other).unwrap();
        assert!(contains(
            directory.path(),
            "https://saved.example/login?private=1"
        ));
        assert!(contains(directory.path(), "https://other.example:444"));
        for url in [
            "http://saved.example",
            "https://saved.example:444",
            "https://saved.example.attacker.test",
        ] {
            assert!(!contains(directory.path(), url));
        }
        update(directory.path(), "first", &BTreeSet::new()).unwrap();
        assert!(!contains(directory.path(), "https://saved.example"));
        assert!(contains(directory.path(), "https://other.example:444"));
        assert_eq!(files(directory.path()).len(), 3);

        remove(directory.path(), "first").unwrap();
        remove(directory.path(), "missing").unwrap();
        assert!(contains(directory.path(), "https://other.example:444"));
        assert_eq!(files(directory.path()).len(), 2);
    }

    #[test]
    fn lost_or_damaged_key_is_replaced_and_old_digests_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let sites = BTreeSet::from(["https://saved.example".to_owned()]);
        assert!(!contains(directory.path(), "https://saved.example"));
        update(directory.path(), "vault", &sites).unwrap();
        let first = read_key(directory.path()).unwrap().unwrap();

        fs::remove_file(directory.path().join(KEY)).unwrap();
        assert!(!contains(directory.path(), "https://saved.example"));
        assert_eq!(files(directory.path()), BTreeSet::from([KEY.to_owned()]));
        assert_ne!(*read_key(directory.path()).unwrap().unwrap(), *first);
        update(directory.path(), "vault", &sites).unwrap();
        assert!(contains(directory.path(), "https://saved.example"));

        fs::write(directory.path().join(KEY), b"short").unwrap();
        assert!(update(directory.path(), "vault", &sites).is_err());
        assert!(!contains(directory.path(), "https://saved.example"));
        update(directory.path(), "vault", &sites).unwrap();
        assert!(contains(directory.path(), "https://saved.example"));
    }

    #[test]
    fn strips_private_url_parts_and_excludes_passwordless_entries() {
        let mut entries = Vec::new();
        for (url, password) in [
            (
                "https://SAVED.example:443/private?token=secret#fragment",
                "password",
            ),
            ("https://saved.example/other", "password"),
            ("https://empty.example", ""),
            ("http://unsafe.example", "password"),
            ("https://user:secret@embedded.example", "password"),
        ] {
            let mut entry = crate::records::entry::Entry::new();
            entry.url = url.into();
            entry.password = password.into();
            entry.username = "private-user".into();
            entries.push(entry);
        }
        let sites = origins(&entries);
        assert_eq!(sites, BTreeSet::from(["https://saved.example".to_owned()]));
        let directory = tempfile::tempdir().unwrap();
        let key = create_key(directory.path()).unwrap();
        update(directory.path(), "vault", &sites).unwrap();
        let path = index_path(directory.path(), &key, "vault");
        let bytes = fs::read(&path).unwrap();
        assert_eq!(
            serde_json::from_slice::<BTreeSet<String>>(&bytes).unwrap(),
            BTreeSet::from([digest(&key, b"origin", "https://saved.example")])
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("saved.example"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [path, directory.path().join(KEY)] {
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn corrupt_index_does_not_hide_other_vault_matches() {
        let directory = tempfile::tempdir().unwrap();
        let key = create_key(directory.path()).unwrap();
        fs::write(directory.path().join("broken.sites"), "{").unwrap();
        fs::write(
            directory.path().join("unfinished.tmp"),
            serde_json::to_vec(&[digest(&key, b"origin", "https://unfinished.example")]).unwrap(),
        )
        .unwrap();
        update(
            directory.path(),
            "valid",
            &BTreeSet::from(["https://saved.example".into()]),
        )
        .unwrap();
        assert!(contains(directory.path(), "https://saved.example"));
        assert!(!contains(directory.path(), "https://unfinished.example"));
    }
}
