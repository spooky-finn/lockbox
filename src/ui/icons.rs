//! Brand icons from Simple Icons, built into the binary: lockbox never
//! downloads them. Matching is by the domain of an entry's URL.

use std::{collections::HashMap, sync::OnceLock};

use resvg::{tiny_skia, usvg};
use serde::Deserialize;

use crate::core::entry::Entry;

static DATA: &str = include_str!("../../assets/icons.jsonl");

/// Suffixes under which names are registered one level deeper (example.co.uk).
const TWO_LEVEL: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "com.au", "net.au", "co.jp", "ne.jp", "com.br", "com.cn", "com.tr",
    "co.kr", "co.in", "com.mx", "co.za", "com.ru", "net.ru", "org.ru", "msk.ru", "spb.ru",
    "com.ua", "co.nz",
];

#[derive(Debug, Deserialize)]
pub struct Icon {
    pub slug: String,
    pub title: String,
    /// Brand colour, `RRGGBB`.
    pub hex: String,
    /// SVG path in a 24×24 box.
    pub path: String,
    #[serde(default)]
    pub domains: Vec<String>,
}

pub struct Catalog {
    icons: Vec<Icon>,
    by_slug: HashMap<String, usize>,
    by_domain: HashMap<String, usize>,
}

/// Parsed once, on first use.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| Catalog::parse(DATA))
}

impl Catalog {
    fn parse(data: &str) -> Self {
        let icons: Vec<Icon> = data
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).expect("assets/icons.jsonl is valid"))
            .collect();
        let mut by_slug = HashMap::new();
        let mut by_domain = HashMap::new();

        for (index, icon) in icons.iter().enumerate() {
            by_slug.insert(icon.slug.clone(), index);

            for domain in &icon.domains {
                by_domain.insert(domain.clone(), index);
            }
        }
        Self {
            icons,
            by_slug,
            by_domain,
        }
    }

    pub fn len(&self) -> usize {
        self.icons.len()
    }

    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }

    pub fn get(&self, slug: &str) -> Option<&Icon> {
        self.by_slug.get(slug).map(|&index| &self.icons[index])
    }

    /// The most specific listed domain wins (mail.google.com before google.com);
    /// otherwise a site named like an icon gets it (gitlab.com → gitlab).
    pub fn for_url(&self, url: &str) -> Option<&Icon> {
        let host = host(url)?;
        let site = registrable(&host);
        let mut candidate = host.as_str();
        loop {
            if let Some(&index) = self.by_domain.get(candidate) {
                return Some(&self.icons[index]);
            }

            if candidate == site {
                break;
            }
            candidate = candidate.split_once('.')?.1;
        }

        self.get(site.split('.').next()?)
    }

    /// The icon chosen for the entry, else one for its URL, else one named like its title.
    pub fn for_entry(&self, entry: &Entry) -> Option<&Icon> {
        if let Some(slug) = &entry.icon {
            return self.get(slug);
        }

        self.for_url(&entry.url)
            .or_else(|| self.get(&normalized(&entry.title)))
    }

    /// Icons whose title or slug contains the query, best matches first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Icon> {
        let query = normalized(query);

        if query.is_empty() {
            return Vec::new();
        }

        let mut found: Vec<&Icon> = self
            .icons
            .iter()
            .filter(|icon| icon.slug.contains(&query) || normalized(&icon.title).contains(&query))
            .collect();
        found.sort_by_key(|icon| (!icon.slug.starts_with(&query), icon.slug.len()));
        found.truncate(limit);
        found
    }
}

/// Lower-case host of a URL typed by hand: scheme, user, port and path are optional.
/// Shared with `build-icons`, which reads the icons' source URLs the same way.
pub fn host(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?.trim_end_matches('.').to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    (host.contains('.') && !host.starts_with('.')).then_some(host)
}

/// The registered site of a host: `mail.google.com` → `google.com`.
pub fn registrable(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let last_two = labels[labels.len().saturating_sub(2)..].join(".");
    let keep = if TWO_LEVEL.contains(&last_two.as_str()) {
        3
    } else {
        2
    };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

/// Lower-case Latin letters and digits only: `Mail.Ru` → `mailru`.
pub fn normalized(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

pub fn rgb(hex: &str) -> [u8; 3] {
    let channel = |range| u8::from_str_radix(&hex[range], 16).unwrap_or(0);
    [channel(0..2), channel(2..4), channel(4..6)]
}

/// White glyphs on dark brand colours, black on light ones.
pub fn glyph_is_dark(background: [u8; 3]) -> bool {
    let [r, g, b] = background.map(|c| f32::from(c) / 255.0);
    0.2126 * r + 0.7152 * g + 0.0722 * b > 0.6
}

/// The glyph as premultiplied RGBA pixels, `size`×`size`.
pub fn render(icon: &Icon, size: u32, dark: bool) -> Option<Vec<u8>> {
    let colour = if dark { "000000" } else { "FFFFFF" };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#{colour}" d="{}"/></svg>"##,
        icon.path
    );
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / 24.0;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(pixmap.take())
}

/// Background for entries without a brand icon; stable for the same title.
pub fn letter_colour(title: &str) -> [u8; 3] {
    const PALETTE: [[u8; 3]; 10] = [
        [0x3B, 0x82, 0xF6],
        [0x10, 0xB9, 0x81],
        [0xF5, 0x9E, 0x0B],
        [0xEF, 0x44, 0x44],
        [0x8B, 0x5C, 0xF6],
        [0xEC, 0x48, 0x99],
        [0x14, 0xB8, 0xA6],
        [0xF9, 0x73, 0x16],
        [0x63, 0x66, 0xF1],
        [0x84, 0xCC, 0x16],
    ];
    // FNV-1a: tiny, deterministic across runs and platforms.
    let hash = title
        .to_lowercase()
        .bytes()
        .fold(0x811c_9dc5u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
    PALETTE[hash as usize % PALETTE.len()]
}

pub fn letter(title: &str) -> String {
    title
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
#[path = "tests/icons.rs"]
mod tests;
