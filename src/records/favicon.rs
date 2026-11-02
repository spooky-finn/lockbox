//! Opt-in website icons: public HTTPS origins only, pinned DNS, no redirects,
//! or proxies. Icons persist unencrypted with private, size-bounded cache entries.
use base64::{Engine, engine::general_purpose::STANDARD};
use kuchikiki::traits::TendrilSink;
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_URL_BYTES: usize = 4096;
const IMAGE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_CACHE_BYTES: u64 = 1_500_000;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedIcon {
    image: Option<String>,
    fetched_at: u64,
}

impl CachedIcon {
    fn fresh(&self, now: u64) -> bool {
        let ttl = if self.image.is_some() {
            IMAGE_TTL
        } else {
            FAILURE_TTL
        };
        now.checked_sub(self.fetched_at)
            .is_some_and(|age| age < ttl.as_secs())
    }
}
type CacheSlot = Arc<Mutex<Option<CachedIcon>>>;

#[derive(Clone)]
pub(crate) struct Cache {
    memory: Arc<Mutex<HashMap<String, CacheSlot>>>,
    directory: Option<PathBuf>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(dirs::cache_dir().map(|root| root.join("lockbox/favicons/v1")))
    }
}

impl PartialEq for Cache {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.memory, &other.memory)
    }
}

impl Cache {
    fn new(directory: Option<PathBuf>) -> Self {
        Self {
            memory: Default::default(),
            directory,
        }
    }

    pub(crate) fn fetch(&self, website: &str) -> Option<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        self.cached_fetch(website, now, || download(website))
    }

    fn cached_fetch(
        &self,
        website: &str,
        now: u64,
        download: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        if website.len() > MAX_URL_BYTES {
            return None;
        }
        let url = url::Url::parse(website).ok()?;
        if !same_origin_https(&url, &url) {
            return None;
        }
        let slot = {
            let mut cache = self.memory.lock().ok()?;
            if cache.len() >= 256 {
                cache.retain(|_, slot| Arc::strong_count(slot) > 1);
            }
            cache.entry(website.to_owned()).or_default().clone()
        };
        let mut cached = slot.lock().ok()?;
        if let Some(entry) = cached.as_ref().filter(|entry| entry.fresh(now)) {
            return entry.image.clone();
        }
        let path = self.directory.as_deref().and_then(|directory| {
            private_directory(directory).ok()?;
            Some(cache_path(directory, website))
        });
        if let Some(entry) = path.as_deref().and_then(|path| load(path, now)) {
            let image = entry.image.clone();
            *cached = Some(entry);
            return image;
        }
        let image = download();
        let entry = CachedIcon {
            image: image.clone(),
            fetched_at: now,
        };
        if let Some(path) = path {
            let _ = store(&path, &entry);
        }
        *cached = Some(entry);
        image
    }
}

fn cache_path(directory: &Path, website: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    directory.join(format!("{:x}.json", Sha256::digest(website.as_bytes())))
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
            .is_ok_and(|bytes| !bytes.is_empty() && bytes.len() <= 1024 * 1024)
}

fn load(path: &Path, now: u64) -> Option<CachedIcon> {
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

fn store(path: &Path, entry: &CachedIcon) -> std::io::Result<()> {
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
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        {
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
        }
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn raster_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/x-icon"
            | "image/vnd.microsoft.icon"
    )
}

fn same_origin_https(url: &url::Url, origin: &url::Url) -> bool {
    url.as_str().len() <= MAX_URL_BYTES
        && url.scheme() == "https"
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.origin() == origin.origin()
}

fn icon_links(html: &str, page: &url::Url) -> Vec<url::Url> {
    let document = kuchikiki::parse_html().one(html).document_node;
    let base = document
        .select_first("base[href]")
        .ok()
        .and_then(|node| page.join(node.attributes.borrow().get("href")?).ok())
        .filter(|base| same_origin_https(base, page))
        .unwrap_or_else(|| page.clone());
    let mut icons = Vec::new();
    if let Ok(links) = document.select("link[href][rel]") {
        for link in links {
            let attrs = link.attributes.borrow();
            if !attrs
                .get("rel")
                .unwrap_or_default()
                .split_ascii_whitespace()
                .any(|value| {
                    value.eq_ignore_ascii_case("icon")
                        || value.eq_ignore_ascii_case("apple-touch-icon")
                })
            {
                continue;
            }
            if let Ok(icon) = base.join(attrs.get("href").unwrap_or_default())
                && same_origin_https(&icon, page)
            {
                icons.push(icon);
            }
            if icons.len() == 7 {
                break;
            }
        }
    }
    icons
}

/// Conservative public-address policy. Deny special-use IPv4 ranges and IPv6
/// outside ordinary global unicast, including translation/tunnelling ranges.
fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            let special = matches!(a, 0 | 10 | 127 | 224..=255)
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192
                    && ((b == 0 && matches!(c, 0 | 2)) || b == 168 || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113);
            !special
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(ip));
            }
            let s = ip.segments();
            (s[0] & 0xe000 == 0x2000)
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] & 0xf000 == 0)
        }
    }
}

fn public_addresses(addresses: Vec<SocketAddr>) -> Option<Vec<SocketAddr>> {
    (!addresses.is_empty() && addresses.iter().all(|address| public_ip(address.ip())))
        .then_some(addresses)
}

fn request(url: &url::Url, origin: &url::Url) -> Option<reqwest::blocking::Response> {
    if !same_origin_https(url, origin) {
        return None;
    }
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    let addresses = match url.host()? {
        url::Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        url::Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        url::Host::Domain(domain) => (domain, port).to_socket_addrs().ok()?.take(32).collect(),
    };
    let addresses = public_addresses(addresses)?;
    // A fresh client pins every connection/retry to precisely the vetted IPs.
    // Preserve the URL's hostname for TLS/SNI; never disable certificate checks.
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .no_proxy()
        .referer(false)
        .resolve_to_addrs(host, &addresses)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let response = client.get(url.clone()).send().ok()?;
    response.status().is_success().then_some(response)
}

fn download(fallback: &str) -> Option<String> {
    if fallback.len() > MAX_URL_BYTES {
        return None;
    }
    let fallback = url::Url::parse(fallback).ok()?;
    if !same_origin_https(&fallback, &fallback) {
        return None;
    }
    let home = fallback.join("/").ok()?;
    let mut candidates = Vec::new();
    if let Some(response) = request(&home, &home) {
        let mut body = String::new();
        if response.take(512 * 1024).read_to_string(&mut body).is_ok() {
            candidates = icon_links(&body, &home);
        }
    }
    candidates.push(fallback);
    candidates.dedup();
    for candidate in candidates.into_iter().take(8) {
        let Some(response) = request(&candidate, &home) else {
            continue;
        };
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)?
            .to_str()
            .ok()?
            .split(';')
            .next()?
            .trim()
            .to_ascii_lowercase();
        // Exclude active/vector formats. Browser image decoders receive only
        // bounded raster data and never website HTML or SVG documents.
        if !raster_mime(&mime) {
            continue;
        }
        let mut bytes = Vec::new();
        if response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_ok()
            && !bytes.is_empty()
            && bytes.len() <= 1024 * 1024
        {
            return Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_local_special_and_translated_addresses() {
        for value in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "64:ff9b::a00:1",
            "2001:db8::1",
            "2002:7f00:1::",
            "3fff::1",
        ] {
            assert!(!public_ip(value.parse().unwrap()), "{value}");
        }
        for value in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(public_ip(value.parse().unwrap()), "{value}");
        }
        assert!(
            public_addresses(vec![
                "1.1.1.1:443".parse().unwrap(),
                "127.0.0.1:443".parse().unwrap()
            ])
            .is_none()
        );
    }
    #[test]
    fn website_html_cannot_select_another_origin_http_or_credentials() {
        let page = url::Url::parse("https://example.com/").unwrap();
        let links = icon_links(
            r#"<base href='https://evil.test/'><link rel=icon href='/safe.png'><link rel=icon href='http://example.com/no.png'><link rel=icon href='https://other.test/no.png'><link rel=icon href='https://user:pass@example.com/no.png'><link rel=icon href='https://example.com:444/no.png'>"#,
            &page,
        );
        assert_eq!(links, vec![page.join("safe.png").unwrap()]);
    }
    #[test]
    fn private_destinations_and_http_are_rejected_before_connecting() {
        for url in [
            "http://127.0.0.1:1/favicon.ico",
            "https://127.0.0.1:1/favicon.ico",
            "https://[::1]:1/favicon.ico",
            "https://192.168.1.1:1/favicon.ico",
        ] {
            assert!(download(url).is_none());
        }
    }
    fn image() -> Option<String> {
        Some("data:image/png;base64,aWNvbg==".into())
    }
    const WEBSITE: &str = "https://example.com/favicon.ico";

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
            serde_json::from_slice(&fs::read(cache_path(&directory, WEBSITE)).unwrap()).unwrap();
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
        fs::create_dir(&directory).unwrap();
        fs::create_dir(cache_path(&directory, WEBSITE)).unwrap();
        assert_eq!(
            Cache::new(Some(directory.clone())).cached_fetch(WEBSITE, 100, image),
            image()
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_active_oversized_and_future_cache_entries_are_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = cache_path(root.path(), WEBSITE);
        for content in [
            "not json".to_owned(),
            serde_json::json!({"image":"https://attacker.test/icon.png","fetched_at":100}).to_string(),
            serde_json::json!({"image":"data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=","fetched_at":100}).to_string(),
            serde_json::json!({"image":"data:image/png;base64,invalid!","fetched_at":100}).to_string(),
            serde_json::json!({"image":image(),"fetched_at":u64::MAX}).to_string(),
        ] {
            fs::write(&path,content).unwrap();
            assert_eq!(Cache::new(Some(root.path().to_owned())).cached_fetch(WEBSITE,101,image),image());
            assert_eq!(load(&path,101).unwrap().image,image());
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
        fs::write(&path,serde_json::json!({"image":format!("data:image/png;base64,{payload}"),"fetched_at":100}).to_string()).unwrap();
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
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        let path = cache_path(&directory, WEBSITE);
        let outside = root.path().join("outside.json");
        let other =
            serde_json::json!({"image":"data:image/png;base64,b3V0c2lkZQ==","fetched_at":100})
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
                let cache = &cache;
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
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
