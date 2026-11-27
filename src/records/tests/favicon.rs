use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fs;
use store::MAX_CACHE_BYTES;

fn image() -> Option<String> {
    Some("data:image/png;base64,aWNvbg==".into())
}

/// Plain SHA-256 of `WEBSITE`, the name older versions gave its cache file.
const UNKEYED_FILE: &str = "d3d39abc88b126ca1df72306e7d15dba35623cc2197ad321867ca4b200d98b02.json";
const WEBSITE: &str = "https://example.com/favicon.ico";

/// The keyed cache path of `WEBSITE`; creates the directory and its key.
fn cache_file(directory: &std::path::Path) -> std::path::PathBuf {
    store::prepare_path(directory, WEBSITE).unwrap()
}

#[test]
fn cache_file_names_are_keyed_per_device_and_not_a_plain_site_hash() {
    let root = tempfile::tempdir().unwrap();
    let (first, second) = (root.path().join("a"), root.path().join("b"));
    for directory in [&first, &second] {
        assert_eq!(
            Cache::new(Some(directory.clone())).cached_fetch(WEBSITE, 100, image),
            image()
        );
        assert!(!directory.join(UNKEYED_FILE).exists());
    }
    let name = |directory: &std::path::Path| cache_file(directory).file_name().unwrap().to_owned();
    assert_ne!(name(&first), name(&second));
    assert!(first.join(name(&first)).is_file());
    // A damaged key is replaced, and entries made under the old key go with it.
    fs::write(first.join("key"), b"short").unwrap();
    let renamed = name(&first);
    assert_eq!(fs::metadata(first.join("key")).unwrap().len(), 32);
    assert!(!first.join(renamed).exists());
    assert_eq!(fs::read_dir(&first).unwrap().count(), 1);
}

#[test]
fn a_new_key_drops_unkeyed_entries_and_clearing_removes_every_version() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join("v1");
    let current = root.path().join("v2");
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&current).unwrap();
    let entry = serde_json::json!({"image":image(),"fetched_at":100}).to_string();
    fs::write(legacy.join(UNKEYED_FILE), &entry).unwrap();
    fs::write(current.join(UNKEYED_FILE), &entry).unwrap();
    let cache = Cache::new(Some(current.clone()));
    assert_eq!(cache.cached_fetch(WEBSITE, 101, image), image());
    assert!(!current.join(UNKEYED_FILE).exists());
    // Older versions' cache is dropped on the next session.
    remove_legacy(root.path());
    assert!(!legacy.exists());
    assert!(current.exists());
    clear(root.path());
    assert!(!root.path().exists());
    clear(root.path());
}

#[test]
fn disk_cache_survives_session_changes_and_keeps_original_expiry() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("icons");
    {
        let first = Cache::new(Some(directory.clone()));
        assert_eq!(first.cached_fetch(WEBSITE, 100, image), image());
    }
    let second = Cache::new(Some(directory.clone()));
    assert_eq!(
        second.cached_fetch(WEBSITE, 101, || panic!("disk hit must not download")),
        image()
    );
    let entry: CachedIcon =
        serde_json::from_slice(&fs::read(cache_file(&directory)).unwrap()).unwrap();
    assert_eq!(entry.fetched_at, 100);
    let mut downloads = 0;
    assert_eq!(
        second.cached_fetch(WEBSITE, 100 + IMAGE_TTL.as_secs(), || {
            downloads += 1;
            image()
        }),
        image()
    );
    assert_eq!(downloads, 1);
}

#[test]
fn negative_cache_survives_sessions_and_retries_after_five_minutes() {
    let root = tempfile::tempdir().unwrap();
    let first = Cache::new(Some(root.path().to_owned()));
    assert_eq!(first.cached_fetch(WEBSITE, 100, || None), None);
    let second = Cache::new(Some(root.path().to_owned()));
    assert_eq!(
        second.cached_fetch(WEBSITE, 101, || panic!("failure cached")),
        None
    );
    assert_eq!(
        second.cached_fetch(WEBSITE, 100 + FAILURE_TTL.as_secs(), image),
        image()
    );
}

#[test]
fn cache_io_failures_keep_downloaded_icons_usable_in_memory() {
    let root = tempfile::tempdir().unwrap();
    let blocked = root.path().join("blocked");
    fs::write(&blocked, "keep").unwrap();
    let cache = Cache::new(Some(blocked.clone()));
    assert_eq!(cache.cached_fetch(WEBSITE, 100, image), image());
    assert_eq!(
        cache.cached_fetch(WEBSITE, 101, || panic!(
            "memory cache must survive disk failure"
        )),
        image()
    );
    assert_eq!(fs::read(&blocked).unwrap(), b"keep");
    // A failed atomic replacement cleans its temporary file too.
    let directory = root.path().join("icons");
    fs::create_dir(cache_file(&directory)).unwrap();
    assert_eq!(
        Cache::new(Some(directory.clone())).cached_fetch(WEBSITE, 100, image),
        image()
    );
    // The blocking directory and the key.
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
}

#[test]
fn corrupt_active_oversized_and_future_cache_entries_are_replaced() {
    let root = tempfile::tempdir().unwrap();
    let path = cache_file(root.path());
    for content in [
        "not json".to_owned(),
        serde_json::json!({"image":"https://attacker.test/icon.png","fetched_at":100}).to_string(),
        serde_json::json!({"image":"data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=","fetched_at":100})
            .to_string(),
        serde_json::json!({"image":"data:image/png;base64,invalid!","fetched_at":100}).to_string(),
        serde_json::json!({"image":image(),"fetched_at":u64::MAX}).to_string(),
    ] {
        fs::write(&path, content).unwrap();
        assert_eq!(
            Cache::new(Some(root.path().to_owned())).cached_fetch(WEBSITE, 101, image),
            image()
        );
        assert_eq!(store::load(&path, 101).unwrap().image, image());
    }
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_CACHE_BYTES + 1)
        .unwrap();
    assert_eq!(
        Cache::new(Some(root.path().to_owned())).cached_fetch(WEBSITE, 101, image),
        image()
    );
    let payload = STANDARD.encode(vec![0u8; 1024 * 1024 + 1]);
    fs::write(
        &path,
        serde_json::json!({"image":format!("data:image/png;base64,{payload}"),"fetched_at":100})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        Cache::new(Some(root.path().to_owned())).cached_fetch(WEBSITE, 101, image),
        image()
    );
}

#[cfg(unix)]
#[test]
fn cache_permissions_are_private_and_symlinks_are_never_followed() {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    };
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("icons");
    let path = cache_file(&directory);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    let outside = root.path().join("outside.json");
    let other = serde_json::json!({"image":"data:image/png;base64,b3V0c2lkZQ==","fetched_at":100})
        .to_string();
    fs::write(&outside, &other).unwrap();
    symlink(&outside, &path).unwrap();
    assert_eq!(
        Cache::new(Some(directory.clone())).cached_fetch(WEBSITE, 101, image),
        image()
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), other);
    assert!(
        !fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let linked = root.path().join("linked");
    symlink(&directory, &linked).unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        Cache::new(Some(linked)).cached_fetch(WEBSITE, 102, image),
        image()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    // A pre-existing FIFO cannot block the cache reader.
    fs::remove_file(&path).unwrap();
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(
        Cache::new(Some(directory)).cached_fetch(WEBSITE, 101, image),
        image()
    );
    assert!(fs::metadata(&path).unwrap().is_file());
}

#[test]
fn concurrent_requests_share_one_download_and_disk_entry() {
    let root = tempfile::tempdir().unwrap();
    let cache = Cache::new(Some(root.path().to_owned()));
    let count = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let cache = cache.clone();
            let count = &count;
            scope.spawn(move || {
                assert_eq!(
                    cache.cached_fetch(WEBSITE, 100, || {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        image()
                    }),
                    image()
                );
            });
        }
    });
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    // One entry and the key.
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn invalid_websites_never_reach_disk_or_download() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("icons");
    let cache = Cache::new(Some(directory.clone()));
    for website in [
        "not a URL".to_owned(),
        "http://example.com/favicon.ico".to_owned(),
        "https://user:secret@example.com/favicon.ico".to_owned(),
        format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES)),
    ] {
        assert_eq!(
            cache.cached_fetch(&website, 100, || panic!(
                "invalid website must not download"
            )),
            None
        );
    }
    assert!(!directory.exists());
}

/// Locking or turning website icons off drops the session's cache. A fetch that
/// is still running must neither download afterwards nor write the site to disk.
#[test]
fn downloads_stop_and_never_reach_disk_once_the_session_cache_is_gone() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("favicons");
    let session = SessionCache(Cache::new(Some(directory.clone())));
    let handle = session.handle();
    let during = session.handle();
    let mut session = Some(session);

    // Opted out while the request was on the wire.
    let image_after_opt_out = during.cached_fetch(WEBSITE, 100, || {
        session.take();
        image()
    });
    assert_eq!(image_after_opt_out, None);
    assert!(!cache_file(&directory).exists());

    // Any later fetch from a stale handle never reaches the network.
    let fetched = handle.cached_fetch(WEBSITE, 101, || panic!("downloaded after lock"));
    assert_eq!(fetched, None);
    assert!(!cache_file(&directory).exists());
}

#[test]
fn icons_are_fetched_only_from_public_hostnames_on_the_default_https_port() {
    let cache = Cache::new(None);
    // A shared-space member chooses these; none may be contacted.
    for website in [
        "https://tracker.example:8443/favicon.ico",
        "https://tracker.example:80/favicon.ico",
        "https://93.184.216.34/favicon.ico",
        "https://2130706433/favicon.ico",
        "https://[2606:4700:4700::1111]/favicon.ico",
        "https://localhost/favicon.ico",
        "https://printer.localhost/favicon.ico",
        "https://nas.local./favicon.ico",
        "https://router.home.arpa/favicon.ico",
        "https://intranet/favicon.ico",
    ] {
        assert_eq!(
            cache.cached_fetch(website, 100, || panic!("{website} must not be fetched")),
            None,
            "{website}"
        );
        assert!(download::download(website).is_none(), "{website}");
    }
    for website in [
        "https://example.com/favicon.ico",
        "https://example.com:443/favicon.ico",
        "https://www.example.org./favicon.ico",
    ] {
        assert_eq!(
            cache.cached_fetch(website, 100, image),
            image(),
            "{website}"
        );
    }
}
