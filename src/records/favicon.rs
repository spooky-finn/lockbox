//! Opt-in website icons. Owns expiry, per-website request sharing and memory state.
//! Downloads and plaintext persistence enforce separate network/filesystem boundaries.

mod download;
mod store;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_URL_BYTES: usize = 4096;
const MAX_IMAGE_BYTES: usize = 1024 * 1024;
const IMAGE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(5 * 60);

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
    /// False once the owning session locked or turned website icons off.
    active: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(dirs::cache_dir().map(|root| root.join("lockbox/favicons/v1")))
    }
}

/// The session's cache. Dropping or replacing it (lock, opting out of website
/// icons) stops downloads still in flight from fetching or writing to disk.
#[derive(Default)]
pub(crate) struct SessionCache(Cache);

impl SessionCache {
    /// A clone for one background fetch; it stays bound to this session.
    pub(crate) fn handle(&self) -> Cache {
        self.0.clone()
    }
}

impl Drop for SessionCache {
    fn drop(&mut self) {
        self.0
            .active
            .store(false, std::sync::atomic::Ordering::SeqCst);
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
            active: Arc::new(true.into()),
        }
    }

    pub(crate) fn fetch(&self, website: &str) -> Option<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        self.cached_fetch(website, now, || download::download(website))
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
        let path = self
            .directory
            .as_deref()
            .and_then(|directory| store::prepare_path(directory, website));
        if let Some(entry) = path.as_deref().and_then(|path| store::load(path, now)) {
            let image = entry.image.clone();
            *cached = Some(entry);
            return image;
        }
        let active = || self.active.load(std::sync::atomic::Ordering::SeqCst);
        if !active() {
            return None;
        }
        let image = download();
        // Opted out or locked while downloading: keep nothing, write nothing.
        if !active() {
            return None;
        }
        let entry = CachedIcon {
            image: image.clone(),
            fetched_at: now,
        };
        if let Some(path) = path {
            let _ = store::store(&path, &entry);
        }
        *cached = Some(entry);
        image
    }
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

#[cfg(test)]
#[path = "tests/favicon.rs"]
mod tests;
