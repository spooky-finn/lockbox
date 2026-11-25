use super::{Grant, Hash, ROUTES, Result, TransportError, guest_writes, route};
use reqwest::blocking::{Client, ClientBuilder};
use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    time::Duration,
};
use url::Url;

pub(super) fn builder() -> ClientBuilder {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
}

impl Grant {
    pub(crate) fn validate(
        &self,
        endpoint: &Url,
        bucket: &str,
        space: &str,
        session: Hash,
    ) -> Result<()> {
        let objects = super::super::object_prefix(space)?;
        let prefix = objects
            .strip_suffix("objects/")
            .ok_or(TransportError::InvalidResponse)?;
        for (slot, raw) in ROUTES.into_iter().zip(&self.urls) {
            let url = Url::parse(raw).map_err(|_| TransportError::InvalidResponse)?;
            let path = format!("/{bucket}/{prefix}invite-v1/{session}/{}", route(slot)?.1);
            let write = guest_writes(slot);
            if url.scheme() != "https"
                || url.origin() != endpoint.origin()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || percent_encoding::percent_decode_str(url.path())
                    .decode_utf8()
                    .as_deref()
                    != Ok(path.as_str())
                || !valid_query(&url, write)
            {
                return Err(TransportError::InvalidResponse);
            }
        }
        Ok(())
    }
}

// The issuer uses fixed SigV4 credentials and exposes exactly one S3 operation.
// Reject extra or repeated parameters, including S3 subresources such as ACLs.
fn valid_query(url: &Url, write: bool) -> bool {
    let mut keys = std::collections::BTreeSet::new();
    for (key, value) in url.query_pairs() {
        let valid = match key.as_ref() {
            "x-id" => value == if write { "PutObject" } else { "GetObject" },
            "X-Amz-Algorithm" => value == "AWS4-HMAC-SHA256",
            "X-Amz-Credential" => !value.is_empty(),
            "X-Amz-Date" => value.len() == 16,
            "X-Amz-Expires" => value
                .parse::<u64>()
                .is_ok_and(|n| (1..=604800).contains(&n)),
            "X-Amz-SignedHeaders" => value == if write { "content-length;host" } else { "host" },
            "X-Amz-Signature" => value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
            _ => false,
        };
        if !valid || !keys.insert(key) {
            return false;
        }
    }
    keys.len() == 7
}

// Resolve once, reject special-use addresses, and pin the complete result for
// the client's lifetime. The invitation author controls this endpoint.
pub(super) fn guest_client(url: &Url) -> Result<Client> {
    let host = url.host_str().ok_or(TransportError::InvalidResponse)?;
    let port = url
        .port_or_known_default()
        .ok_or(TransportError::InvalidResponse)?;
    let addresses: Vec<SocketAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(url::Host::Domain(name)) => (name, port)
            .to_socket_addrs()
            .map_err(|_| TransportError::Unavailable)?
            .collect(),
        None => return Err(TransportError::InvalidResponse),
    };
    if addresses.is_empty() || addresses.iter().any(|a| !public(a.ip())) {
        return Err(TransportError::AccessDenied);
    }
    builder()
        .no_proxy()
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| TransportError::Unavailable)
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !matches!(
            ip.octets(),
            [0 | 10 | 127, ..]
                | [100, 64..=127, ..]
                | [169, 254, ..]
                | [172, 16..=31, ..]
                | [192, 0, 0 | 2, ..]
                | [192, 88, 99, ..]
                | [192, 168, ..]
                | [198, 18 | 19, ..]
                | [198, 51, 100, ..]
                | [203, 0, 113, ..]
                | [224..=255, ..]
        ),
        IpAddr::V6(ip) => {
            matches!(ip.segments(), [0x2000..=0x3fff, ..])
                && !matches!(
                    ip.segments(),
                    [0x2001, 0..=0x1ff | 0xdb8, ..] | [0x2002, ..] | [0x3fff, ..]
                )
        }
    }
}
