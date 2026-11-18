//! Same-origin icon discovery over HTTPS with vetted, pinned public addresses.

use super::{MAX_IMAGE_BYTES, MAX_URL_BYTES, raster_mime, same_origin_https};
use base64::{Engine, engine::general_purpose::STANDARD};
use kuchikiki::traits::TendrilSink;
use std::{
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    time::Duration,
};

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

pub(super) fn download(fallback: &str) -> Option<String> {
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
            .take(MAX_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .is_ok()
            && !bytes.is_empty()
            && bytes.len() <= MAX_IMAGE_BYTES
        {
            return Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
        }
    }
    None
}

#[cfg(test)]
#[path = "../tests/favicon_download.rs"]
mod tests;
