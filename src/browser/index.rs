//! Advisory plaintext HTTPS origins. Never used to authorize credential release.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 8 * 1024 * 1024;

fn directory() -> io::Result<PathBuf> {
    #[cfg(not(test))]
    let root = dirs::home_dir()
        .ok_or_else(|| io::Error::other("no home directory"))?
        .join(".lockbox");
    #[cfg(test)]
    let root = {
        static ROOT: std::sync::LazyLock<tempfile::TempDir> =
            std::sync::LazyLock::new(|| tempfile::tempdir().unwrap());
        ROOT.path().to_owned()
    };
    fs::create_dir_all(&root)?;
    let directory = root.join("browser-sites");
    let mut builder = fs::DirBuilder::new();
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

fn publish(directory: &Path, id: &str, origins: &BTreeSet<String>) -> io::Result<()> {
    let name: String = Sha256::digest(id.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let destination = directory.join(format!("{name}.json"));
    let bytes = serde_json::to_vec_pretty(origins)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("site index too large"));
    }
    if fs::read(&destination).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temporary = directory.join(format!("{name}-{suffix}.tmp"));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

/// Cache failure must not turn a successful encrypted vault save into a failure.
pub(crate) fn refresh(vault: &crate::vault::Vault) {
    if let Err(error) = directory()
        .and_then(|directory| publish(&directory, &vault.id(), &origins(vault.entries())))
    {
        eprintln!("Could not update browser site index: {error}");
    }
}

fn contains(directory: &Path, origin: &str) -> bool {
    let Ok(origin) = super::origin(origin).map(|o| o.ascii_serialization()) else {
        return false;
    };
    let Ok(files) = fs::read_dir(directory) else {
        return false;
    };
    files.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            return false;
        }
        let read = || -> io::Result<bool> {
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.len() > MAX_BYTES {
                return Ok(false);
            }
            let mut bytes = Vec::new();
            file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Ok(false);
            }
            let sites: BTreeSet<String> = serde_json::from_slice(&bytes)?;
            Ok(sites.contains(&origin))
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

    #[test]
    fn vault_save_and_unlock_refresh_index_and_drop_preserves_it() {
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
        publish(&directory().unwrap(), &id, &BTreeSet::new()).unwrap();
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
    fn persists_without_vault_and_replaces_each_vault_independently() {
        let directory = tempfile::tempdir().unwrap();
        let first = BTreeSet::from(["https://saved.example".to_owned()]);
        let other = BTreeSet::from(["https://other.example:444".to_owned()]);
        publish(directory.path(), "first", &first).unwrap();
        publish(directory.path(), "second", &other).unwrap();
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
        publish(directory.path(), "first", &BTreeSet::new()).unwrap();
        assert!(!contains(directory.path(), "https://saved.example"));
        assert!(contains(directory.path(), "https://other.example:444"));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
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
        publish(directory.path(), "vault", &sites).unwrap();
        let path = fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            serde_json::from_slice::<BTreeSet<String>>(&fs::read(&path).unwrap()).unwrap(),
            sites
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn corrupt_index_does_not_hide_other_vault_matches() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("broken.json"), "{").unwrap();
        fs::write(
            directory.path().join("unfinished.tmp"),
            "[\"https://unfinished.example\"]",
        )
        .unwrap();
        publish(
            directory.path(),
            "valid",
            &BTreeSet::from(["https://saved.example".into()]),
        )
        .unwrap();
        assert!(contains(directory.path(), "https://saved.example"));
        assert!(!contains(directory.path(), "https://unfinished.example"));
    }
}
