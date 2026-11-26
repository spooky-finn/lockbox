//! Builds `assets/icons.jsonl` from the simple-icons npm package tarball.
//!
//! ```sh
//! curl -fsSO https://registry.npmjs.org/simple-icons/-/simple-icons-<version>.tgz
//! cargo run --bin build-icons -- simple-icons-<version>.tgz
//! ```
//!
//! Only the pinned release is accepted: the tarball must match
//! `PINNED_INTEGRITY` before it is unpacked. To upgrade, set `PINNED_VERSION`
//! and `PINNED_INTEGRITY` to the new version and the output of
//! `npm view simple-icons@<version> dist.integrity`.
//!
//! Every icon becomes one JSON line: slug, title, brand colour, SVG path and
//! the domains it is shown for. Domains come from the icon's `source` URL when
//! that site is clearly the brand's own, and from `assets/icon_domains.txt`.
//! lockbox also matches a domain whose name equals the slug (gitlab.com →
//! gitlab) at runtime, so those need no entry there.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
};

use base64::{Engine, prelude::BASE64_STANDARD};
use lockbox::icons::{host, normalized, registrable};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

/// The npm `dist.integrity` of the only simple-icons release this tool accepts.
const PINNED_VERSION: &str = "16.36.0";
const PINNED_INTEGRITY: &str = "sha512-+VNPPkqSOLidFeSgBKUWX4n50WPpDC09hav+BgS26anIF1oQOlGBb8LvwGR+6fYRjo+B6CiKzNBceMPkSVXANQ==";

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

/// Accepts the tarball only if its SHA-512 is the `sha512-<base64>` pin.
fn verify_tarball(bytes: &[u8], integrity: &str) -> Result<(), String> {
    let expected = integrity
        .strip_prefix("sha512-")
        .and_then(|digest| BASE64_STANDARD.decode(digest).ok())
        .filter(|digest| digest.len() == 64)
        .ok_or_else(|| format!("not a sha512 integrity: {integrity:?}"))?;

    if Sha512::digest(bytes).as_slice() == expected {
        Ok(())
    } else {
        Err(format!("tarball does not match the pinned {integrity}"))
    }
}

/// Unpacks the verified bytes, not the file that could change after the check.
fn unpack(bytes: &[u8], into: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(into);
    fs::create_dir_all(into).unwrap_or_else(|error| fail(format!("{}: {error}", into.display())));
    let mut tar = Command::new("tar")
        .arg("-xzf")
        .arg("-")
        .arg("-C")
        .arg(into)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| fail(format!("tar: {error}")));
    let written = tar.stdin.take().expect("stdin is piped").write_all(bytes);
    let status = tar.wait();

    if written.is_err() || !status.is_ok_and(|status| status.success()) {
        fail("tar could not unpack the tarball");
    }
    into.join("package")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, tarball] = args.as_slice() else {
        fail(format!(
            "usage: cargo run --bin build-icons -- simple-icons-{PINNED_VERSION}.tgz"
        ));
    };

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let assets = root.join("assets");
    let bytes = fs::read(tarball).unwrap_or_else(|error| fail(format!("{tarball}: {error}")));
    verify_tarball(&bytes, PINNED_INTEGRITY).unwrap_or_else(|error| fail(error));
    let package = unpack(
        &bytes,
        &root
            .join("target")
            .join(format!("simple-icons-{PINNED_VERSION}")),
    );

    let version = serde_json::from_str::<serde_json::Value>(&read(&package.join("package.json")))
        .ok()
        .and_then(|meta| meta["version"].as_str().map(str::to_owned))
        .unwrap_or_else(|| fail("package.json has no version"));

    if version != PINNED_VERSION {
        fail(format!(
            "package.json is {version}, the pin is {PINNED_VERSION}"
        ));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const BYTES: &[u8] = b"fictional tarball bytes";
    const INTEGRITY: &str = "sha512-nNHcLWTwOXrFC18Ik91M0AFYoprr7i8FgUsd21Fx4FhVAzp7J9nQ3Hwa46HsfEwajlhRqYUVDK7wWE2VuBNnsg==";

    #[test]
    fn tarball_matching_the_pinned_integrity_is_accepted() {
        assert_eq!(verify_tarball(BYTES, INTEGRITY), Ok(()));
    }

    #[test]
    fn tampered_or_unpinned_tarballs_are_refused() {
        let mut tampered = BYTES.to_vec();
        tampered[0] ^= 1;
        assert!(verify_tarball(&tampered, INTEGRITY).is_err());
        assert!(verify_tarball(b"", INTEGRITY).is_err());

        for pin in [
            "",
            "sha512-",
            "sha512-not base64!",
            &INTEGRITY.replace("sha512-", "sha256-"),
            &INTEGRITY.replace("sha512-", "SHA512-"),
        ] {
            assert!(verify_tarball(BYTES, pin).is_err(), "{pin}");
        }
    }

    #[test]
    fn the_pin_names_the_recorded_simple_icons_version() {
        let recorded = include_str!("../../assets/SIMPLE_ICONS_VERSION");
        assert_eq!(recorded.trim(), PINNED_VERSION);
        assert!(PINNED_INTEGRITY.starts_with("sha512-"));
    }
}
