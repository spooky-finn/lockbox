//! Builds `assets/icons.jsonl` from an unpacked simple-icons npm package.
//!
//! ```sh
//! curl -fsSO https://registry.npmjs.org/simple-icons/-/simple-icons-<version>.tgz
//! tar -xzf simple-icons-<version>.tgz          # unpacks into ./package
//! cargo run --bin build-icons -- package
//! ```
//!
//! Every icon becomes one JSON line: slug, title, brand colour, SVG path and
//! the domains it is shown for. Domains come from the icon's `source` URL when
//! that site is clearly the brand's own, and from `assets/icon_domains.txt`.
//! lockbox also matches a domain whose name equals the slug (gitlab.com →
//! gitlab) at runtime, so those need no entry there.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process,
};

use lockbox::icons::{host, normalized, registrable};
use serde::{Deserialize, Serialize};

const SVG_START: &str =
    r#"<svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>"#;
const PATH_START: &str = r#"</title><path d=""#;
const SVG_END: &str = r#""/></svg>"#;

#[derive(Deserialize)]
struct Source {
    slug: String,
    title: String,
    hex: String,
    source: String,
}

/// Field order is the line format of `icons.jsonl`.
#[derive(Serialize)]
struct Line<'a> {
    slug: &'a str,
    title: &'a str,
    hex: String,
    path: &'a str,
    domains: Vec<String>,
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("build-icons: {message}");
    process::exit(1);
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| fail(format!("{}: {error}", path.display())))
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap_or_else(|error| fail(format!("{}: {error}", path.display())));
}

/// The SVG path, if the file is the single-path 24×24 icon simple-icons ships.
fn svg_path(svg: &str) -> Option<&str> {
    let rest = svg.strip_prefix(SVG_START)?;
    let (_, rest) = rest.split_once(PATH_START)?;
    let (path, tail) = rest.split_once('"')?;
    let tail = tail.strip_prefix(&SVG_END[1..])?;
    (!path.is_empty() && tail.trim().is_empty()).then_some(path)
}

/// The source site's domain, if its name is the brand's name.
fn source_domain(icon: &Source) -> Option<String> {
    let domain = registrable(&host(&icon.source)?);
    let name = domain.split('.').next()?;
    (name == icon.slug || name == normalized(&icon.title)).then_some(domain)
}

/// `slug domain…` lines in file order; `#` starts a comment.
fn manual_domains(assets: &Path) -> Vec<(String, Vec<String>)> {
    let mut table: Vec<(String, Vec<String>)> = Vec::new();

    for (number, line) in read(&assets.join("icon_domains.txt")).lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default();
        let mut words = line.split_whitespace();
        let Some(slug) = words.next() else {
            continue;
        };

        let domains: Vec<String> = words.map(str::to_lowercase).collect();

        if domains.is_empty() {
            fail(format!(
                "icon_domains.txt:{}: no domains for {slug}",
                number + 1
            ));
        }

        match table.iter_mut().find(|(known, _)| known == slug) {
            Some((_, known)) => known.extend(domains),
            None => table.push((slug.to_owned(), domains)),
        }
    }
    table
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, package] = args.as_slice() else {
        fail("usage: cargo run --bin build-icons -- <unpacked simple-icons package>");
    };

    let package = PathBuf::from(package);
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");

    let version = serde_json::from_str::<serde_json::Value>(&read(&package.join("package.json")))
        .ok()
        .and_then(|meta| meta["version"].as_str().map(str::to_owned))
        .unwrap_or_else(|| fail("package.json has no version"));
    let mut icons: Vec<Source> =
        serde_json::from_str(&read(&package.join("data").join("simple-icons.json")))
            .unwrap_or_else(|error| fail(format!("simple-icons.json: {error}")));
    icons.sort_by(|a, b| a.slug.cmp(&b.slug));
    let manual = manual_domains(&assets);

    let mut unknown: Vec<&str> = manual
        .iter()
        .map(|(slug, _)| slug.as_str())
        .filter(|slug| !icons.iter().any(|icon| icon.slug == *slug))
        .collect();

    if !unknown.is_empty() {
        unknown.sort_unstable();
        fail(format!(
            "icon_domains.txt names unknown icons: {}",
            unknown.join(", ")
        ));
    }

    // The manual table always wins; a source domain goes to the first icon
    // (in slug order) that claims it, so brand variants (uniqlo, uniqlo_ja) share nothing.
    let mut owner: HashMap<String, String> = HashMap::new();

    for (slug, domains) in &manual {
        for domain in domains {
            let first = owner.entry(domain.clone()).or_insert_with(|| slug.clone());

            if first != slug {
                fail(format!(
                    "icon_domains.txt gives {domain} to both {first} and {slug}"
                ));
            }
        }
    }

    let mut lines = Vec::with_capacity(icons.len());

    for icon in &icons {
        let svg = read(&package.join("icons").join(format!("{}.svg", icon.slug)));
        let path = svg_path(&svg)
            .unwrap_or_else(|| fail(format!("{}.svg is not a single-path 24×24 icon", icon.slug)));

        if icon.hex.len() != 6 || !icon.hex.chars().all(|c| c.is_ascii_hexdigit()) {
            fail(format!("{}: bad colour {}", icon.slug, icon.hex));
        }

        let mut domains = manual
            .iter()
            .find(|(slug, _)| *slug == icon.slug)
            .map(|(_, domains)| domains.clone())
            .unwrap_or_default();

        if let Some(domain) = source_domain(icon) {
            let first = owner
                .entry(domain.clone())
                .or_insert_with(|| icon.slug.clone());

            if *first == icon.slug && !domains.contains(&domain) {
                domains.push(domain);
            }
        }

        let line = Line {
            slug: &icon.slug,
            title: &icon.title,
            hex: icon.hex.to_uppercase(),
            path,
            domains,
        };
        lines.push(serde_json::to_string(&line).expect("icon lines always serialize"));
    }

    write(&assets.join("icons.jsonl"), &(lines.join("\n") + "\n"));
    write(
        &assets.join("SIMPLE_ICONS_VERSION"),
        &format!("{version}\n"),
    );

    for (from, to) in [
        ("LICENSE.md", "LICENSE-simple-icons.md"),
        ("DISCLAIMER.md", "DISCLAIMER-simple-icons.md"),
    ] {
        fs::copy(package.join(from), assets.join(to))
            .unwrap_or_else(|error| fail(format!("{from}: {error}")));
    }
    println!(
        "simple-icons {version}: {} icons, {} domains",
        lines.len(),
        owner.len()
    );
}
