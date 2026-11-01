//! Download website-declared icons without blocking the UI.
use base64::{Engine, engine::general_purpose::STANDARD};
use kuchikiki::traits::TendrilSink;
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn icon_links(html: &str, page: &url::Url) -> Vec<String> {
    let document = kuchikiki::parse_html().one(html).document_node;
    let base = document
        .select_first("base[href]")
        .ok()
        .and_then(|node| page.join(node.attributes.borrow().get("href")?).ok())
        .unwrap_or_else(|| page.clone());
    let mut icons = Vec::new();
    if let Ok(links) = document.select("link[href][rel]") {
        for link in links {
            let attrs = link.attributes.borrow();
            let rel = attrs.get("rel").unwrap_or_default();
            if !rel.split_ascii_whitespace().any(|value| {
                value.eq_ignore_ascii_case("icon") || value.eq_ignore_ascii_case("apple-touch-icon")
            }) {
                continue;
            }
            if let Ok(mut icon) = base.join(attrs.get("href").unwrap_or_default()) {
                if !matches!(icon.scheme(), "http" | "https") {
                    continue;
                }
                let _ = icon.set_username("");
                let _ = icon.set_password(None);
                icons.push(icon.to_string());
            }
        }
    }
    icons
}

const IMAGE_TTL: u64 = 30 * 24 * 60 * 60;
const FAILURE_TTL: u64 = 5 * 60;
const MAX_CACHE_BYTES: u64 = 1_500_000;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
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
            .is_some_and(|age| age < ttl)
    }
}

type CacheSlot = Arc<Mutex<Option<CachedIcon>>>;

pub(super) fn fetch(fallback: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheSlot>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let slot = {
        let mut cache = cache.lock().ok()?;
        if cache.len() >= 256 {
            // Keep slots with active requests so concurrent callers still share them.
            cache.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        cache.entry(fallback.to_owned()).or_default().clone()
    };
    let directory = dirs::cache_dir().map(|root| root.join("lockbox/favicons/v1"));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    cached_fetch(&slot, directory.as_deref(), fallback, now, || {
        download(fallback)
    })
}

fn cached_fetch(
    slot: &Mutex<Option<CachedIcon>>,
    directory: Option<&Path>,
    website: &str,
    now: u64,
    download: impl FnOnce() -> Option<String>,
) -> Option<String> {
    use sha2::{Digest, Sha256};
    // Hold only this website's lock during I/O, allowing other sites to load concurrently.
    let mut memory = slot.lock().ok()?;
    if let Some(entry) = memory.as_ref().filter(|entry| entry.fresh(now)) {
        return entry.image.clone();
    }
    let path = directory
        .map(|directory| directory.join(format!("{:x}.json", Sha256::digest(website.as_bytes()))));
    if let Some(entry) = path.as_ref().and_then(|path| {
        let file = std::fs::File::open(path).ok()?;
        if file.metadata().ok()?.len() > MAX_CACHE_BYTES {
            return None;
        }
        let entry: CachedIcon = serde_json::from_reader(file.take(MAX_CACHE_BYTES)).ok()?;
        (entry.fresh(now)
            && entry
                .image
                .as_ref()
                .is_none_or(|image| image.starts_with("data:image/")))
        .then_some(entry)
    }) {
        let image = entry.image.clone();
        *memory = Some(entry);
        return image;
    }
    let image = download();
    let entry = CachedIcon {
        image: image.clone(),
        fetched_at: now,
    };
    if let Some(path) = path {
        // A failed cache write must never prevent displaying a downloaded icon.
        let _ = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let mut random = [0u8; 8];
            getrandom::fill(&mut random).map_err(std::io::Error::other)?;
            let temporary = path.with_extension(format!("{:x}.tmp", u64::from_le_bytes(random)));
            let result = (|| -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(&serde_json::to_vec(&entry)?)?;
                std::fs::rename(&temporary, &path)
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(temporary);
            }
            result
        })();
    }
    *memory = Some(entry);
    image
}

fn download(fallback: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;
    let home = url::Url::parse(fallback).ok()?.join("/").ok()?;
    let mut candidates = Vec::new();
    if let Ok(response) = client.get(home).send().and_then(|r| r.error_for_status()) {
        let page = response.url().clone();
        let mut body = String::new();
        if response.take(512 * 1024).read_to_string(&mut body).is_ok() {
            candidates = icon_links(&body, &page);
        }
    }
    candidates.push(fallback.to_owned());
    candidates.dedup();
    for candidate in candidates.into_iter().take(8) {
        let Ok(response) = client
            .get(candidate)
            .send()
            .and_then(|r| r.error_for_status())
        else {
            continue;
        };
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !mime.starts_with("image/") {
            continue;
        }
        let mut bytes = Vec::new();
        if response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.is_empty()
            || bytes.len() > 1024 * 1024
        {
            continue;
        }
        return Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Option<String> {
        Some("data:image/png;base64,aWNvbg==".into())
    }

    #[test]
    fn disk_cache_survives_new_memory_cache_and_expires() {
        let directory = tempfile::tempdir().unwrap();
        let website = "https://example.com/favicon.ico";
        let first = Mutex::new(None);
        assert_eq!(
            cached_fetch(&first, Some(directory.path()), website, 100, image),
            image()
        );
        let restarted = Mutex::new(None);
        assert_eq!(
            cached_fetch(&restarted, Some(directory.path()), website, 101, || panic!(
                "cached icon must not download"
            )),
            image()
        );
        assert_eq!(
            cached_fetch(
                &restarted,
                Some(directory.path()),
                website,
                100 + IMAGE_TTL,
                || Some("data:image/png;base64,bmV3".into())
            ),
            Some("data:image/png;base64,bmV3".into())
        );
        let files: Vec<_> = std::fs::read_dir(directory.path()).unwrap().collect();
        assert_eq!(files.len(), 1);
        assert!(
            !files[0]
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("example")
        );
    }

    #[test]
    fn failures_retry_after_short_expiry() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            cached_fetch(
                &Mutex::new(None),
                Some(directory.path()),
                "site",
                100,
                || None
            ),
            None
        );
        let restarted = Mutex::new(None);
        assert_eq!(
            cached_fetch(&restarted, Some(directory.path()), "site", 101, || panic!(
                "failure cached"
            )),
            None
        );
        assert_eq!(
            cached_fetch(
                &restarted,
                Some(directory.path()),
                "site",
                100 + FAILURE_TTL,
                image
            ),
            image()
        );
    }

    #[test]
    fn corrupt_cache_and_unwritable_directory_do_not_block_download() {
        let directory = tempfile::tempdir().unwrap();
        cached_fetch(
            &Mutex::new(None),
            Some(directory.path()),
            "site",
            100,
            image,
        );
        let path = std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(&path, "broken json").unwrap();
        assert_eq!(
            cached_fetch(
                &Mutex::new(None),
                Some(directory.path()),
                "site",
                101,
                image
            ),
            image()
        );
        assert_eq!(
            cached_fetch(&Mutex::new(None), Some(&path), "site", 102, image),
            image()
        );
    }

    #[test]
    fn concurrent_requests_share_download() {
        let slot = Arc::new(Mutex::new(None));
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    assert_eq!(
                        cached_fetch(&slot, None, "site", 100, || {
                            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            image()
                        }),
                        image()
                    );
                });
            }
        });
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn discovers_real_icons_and_resolves_base_and_entities() {
        let page = url::Url::parse("https://example.com/login/").unwrap();
        assert_eq!(
            icon_links(
                r#"<base href='/assets/'><link rel='stylesheet' href='a.css'><link REL='shortcut ICON' href='brand.png?v=1&amp;s=2'><link rel='apple-touch-icon' href='//cdn.example.com/apple.png'><link rel='icon' href='javascript:bad'>"#,
                &page
            ),
            vec![
                "https://example.com/assets/brand.png?v=1&s=2",
                "https://cdn.example.com/apple.png"
            ]
        );
    }
    #[test]
    fn downloads_declared_image_instead_of_guessed_favicon() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (path, content_type, body) in [
                ("/", "text/html", "<link rel=icon href=/brand.png>"),
                ("/brand.png", "image/png", "actual-image-bytes"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let length = stream.read(&mut request).unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..length])
                        .starts_with(&format!("GET {path} HTTP/1.1"))
                );
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        assert_eq!(
            download(&format!("http://{address}/favicon.ico")),
            Some(format!(
                "data:image/png;base64,{}",
                STANDARD.encode("actual-image-bytes")
            ))
        );
        server.join().unwrap();
    }

    #[test]
    fn resolves_relative_icons_after_redirects() {
        let page = url::Url::parse("https://www.example.com/site/").unwrap();
        assert_eq!(
            icon_links("<link rel=icon href=logo.ico>", &page),
            vec!["https://www.example.com/site/logo.ico"]
        );
        assert!(icon_links("<html>No icon</html>", &page).is_empty());
    }
}
