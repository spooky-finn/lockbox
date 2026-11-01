//! Opt-in website icons: public HTTPS origins only, pinned DNS, no redirects,
//! proxies or persistent cache. Each unlocked workspace owns its memory cache.
use base64::{Engine, engine::general_purpose::STANDARD};
use kuchikiki::traits::TendrilSink;
use std::{
    collections::HashMap,
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_URL_BYTES: usize = 4096;
const IMAGE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(5 * 60);

struct CachedIcon {
    image: Option<String>,
    fetched_at: Instant,
}
type CacheSlot = Arc<Mutex<Option<CachedIcon>>>;

#[derive(Clone, Default)]
pub(crate) struct Cache(Arc<Mutex<HashMap<String, CacheSlot>>>);
impl PartialEq for Cache {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Cache {
    pub(crate) fn fetch(&self, website: &str) -> Option<String> {
        if website.len() > MAX_URL_BYTES {
            return None;
        }
        let slot = {
            let mut cache = self.0.lock().ok()?;
            if cache.len() >= 256 {
                cache.retain(|_, slot| Arc::strong_count(slot) > 1);
            }
            cache.entry(website.to_owned()).or_default().clone()
        };
        let mut cached = slot.lock().ok()?;
        if let Some(entry) = cached.as_ref() {
            let ttl = if entry.image.is_some() {
                IMAGE_TTL
            } else {
                FAILURE_TTL
            };
            if entry.fetched_at.elapsed() < ttl {
                return entry.image.clone();
            }
        }
        let image = download(website);
        *cached = Some(CachedIcon {
            image: image.clone(),
            fetched_at: Instant::now(),
        });
        image
    }
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
        if !matches!(
            mime.as_str(),
            "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "image/x-icon"
                | "image/vnd.microsoft.icon"
        ) {
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

/// Remove only the obsolete application-owned plaintext cache. Never traverse
/// symlinked cache components into some other directory.
pub(crate) fn cleanup_legacy_cache() {
    if let Some(root) = dirs::cache_dir() {
        let _ = cleanup_at(&root);
    }
}
fn cleanup_at(root: &Path) -> std::io::Result<()> {
    let mut path = root.to_path_buf();
    for component in ["lockbox", "favicons", "v1"] {
        path.push(component);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Ok(());
        }
    }
    std::fs::remove_dir_all(path)
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
    #[test]
    fn legacy_cache_cleanup_is_scoped_and_refuses_symlink_components() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("lockbox/favicons/v1");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("domain.json"), "old metadata").unwrap();
        let other = root.path().join("keep");
        std::fs::write(&other, "keep").unwrap();
        cleanup_at(root.path()).unwrap();
        assert!(!cache.exists());
        assert!(other.exists());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("keep"), "keep").unwrap();
            std::os::unix::fs::symlink(outside.path(), &cache).unwrap();
            cleanup_at(root.path()).unwrap();
            assert!(outside.path().join("keep").exists());
        }
    }
    #[test]
    fn caches_are_session_scoped_and_have_no_disk_destination() {
        let first = Cache::default();
        let second = Cache::default();
        let slot = Arc::new(Mutex::new(Some(CachedIcon {
            image: Some("fictional-image".into()),
            fetched_at: Instant::now(),
        })));
        first
            .0
            .lock()
            .unwrap()
            .insert("https://example.com/favicon.ico".into(), slot);
        assert_eq!(
            first.fetch("https://example.com/favicon.ico").as_deref(),
            Some("fictional-image")
        );
        assert!(second.0.lock().unwrap().is_empty());
    }
}
