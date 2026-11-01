//! Storj's path-style S3 API with AWS Signature V4. No remote credential probes
//! or uploads occur at construction, and response bodies never enter diagnostics.

use super::sync::{Page, Result, Storage, TransportError};
use crate::core::sync::{Hash, MAX_OBJECT_BYTES};
use hmac::{Hmac, Mac};
use reqwest::{
    Method, StatusCode,
    blocking::{Client, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{io::Read, time::Duration};
use time::{OffsetDateTime, macros::format_description};
use zeroize::Zeroizing;

pub struct Storj {
    client: Client,
    endpoint: url::Url,
    bucket: String,
    access: Zeroizing<String>,
    secret: Zeroizing<String>,
    region: String,
}

impl Storj {
    pub fn new(endpoint: &str, bucket: &str, access: &str, secret: &str) -> Result<Self> {
        Self::with_region(endpoint, bucket, access, secret, "us-east-1")
    }

    pub fn with_region(
        endpoint: &str,
        bucket: &str,
        access: &str,
        secret: &str,
        region: &str,
    ) -> Result<Self> {
        let endpoint = url::Url::parse(endpoint).map_err(|_| TransportError::InvalidResponse)?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || endpoint.path() != "/"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !(3..=63).contains(&bucket.len())
            || !bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !bucket.as_bytes()[0].is_ascii_alphanumeric()
            || !bucket.as_bytes()[bucket.len() - 1].is_ascii_alphanumeric()
            || access.is_empty()
            || secret.is_empty()
            || region.is_empty()
            || [access, secret, region]
                .iter()
                .any(|s| s.chars().any(|c| c.is_whitespace() || c.is_control()))
        {
            return Err(TransportError::InvalidResponse);
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| TransportError::Unavailable)?;
        Ok(Self {
            client,
            endpoint,
            bucket: bucket.into(),
            access: Zeroizing::new(access.into()),
            secret: Zeroizing::new(secret.into()),
            region: region.into(),
        })
    }

    fn request(
        &self,
        method: Method,
        key: Option<&str>,
        query: Vec<(&str, &str)>,
        body: &[u8],
    ) -> Result<Response> {
        let path = match key {
            Some(key) => format!("/{}/{}", self.bucket, key),
            None => format!("/{}", self.bucket),
        };
        let mut query: Vec<_> = query
            .into_iter()
            .map(|(k, v)| (encode(k, false), encode(v, false)))
            .collect();
        query.sort();
        let query = query
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let path = encode(&path, true);
        let mut url = self.endpoint.clone();
        url.set_path(&path);
        if !query.is_empty() {
            url.set_query(Some(&query));
        }
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap()),
            None => url.host_str().unwrap().to_owned(),
        };
        let now = OffsetDateTime::now_utc();
        let date = now
            .format(format_description!("[year][month][day]"))
            .map_err(|_| TransportError::InvalidResponse)?;
        let timestamp = now
            .format(format_description!(
                "[year][month][day]T[hour][minute][second]Z"
            ))
            .map_err(|_| TransportError::InvalidResponse)?;
        let payload_hash = hex(&Sha256::digest(body));
        let authorization = authorization(
            method.as_str(),
            &path,
            &query,
            &host,
            &date,
            &timestamp,
            &payload_hash,
            &self.region,
            &self.access,
            &self.secret,
        );
        self.client
            .request(method, url)
            .header("host", host)
            .header("x-amz-date", timestamp)
            .header("x-amz-content-sha256", payload_hash)
            .header("authorization", authorization)
            .header("content-type", "application/json")
            .body(body.to_vec())
            .send()
            .map_err(|_| TransportError::Unavailable)
    }
}

impl Storage for Storj {
    fn list(&mut self, namespace: Hash, cursor: Option<&str>) -> Result<Page> {
        let prefix = format!("spaces/{namespace}/objects/");
        let mut query = vec![
            ("list-type", "2"),
            ("prefix", &prefix),
            ("max-keys", "1000"),
        ];
        if let Some(cursor) = cursor {
            query.push(("continuation-token", cursor));
        }
        let response = self.request(Method::GET, None, query, &[])?;
        check(response.status())?;
        let bytes = bounded(response, 1024 * 1024)?;
        let listing: Listing = quick_xml::de::from_reader(bytes.as_slice())
            .map_err(|_| TransportError::InvalidResponse)?;
        if listing.contents.len() > 1000
            || (listing.truncated && listing.next.as_ref().is_none_or(|s| s.is_empty()))
            || (!listing.truncated && listing.next.is_some())
        {
            return Err(TransportError::InvalidResponse);
        }
        let objects = listing
            .contents
            .iter()
            .filter_map(|object| {
                object
                    .key
                    .strip_prefix(&prefix)?
                    .strip_suffix(".json")?
                    .parse()
                    .ok()
            })
            .collect();
        Ok(Page {
            objects,
            next: listing.next,
        })
    }

    fn get(&mut self, namespace: Hash, hash: Hash) -> Result<Vec<u8>> {
        let key = format!("spaces/{namespace}/objects/{hash}.json");
        let response = self.request(Method::GET, Some(&key), vec![], &[])?;
        check(response.status())?;
        bounded(response, MAX_OBJECT_BYTES)
    }

    fn put(&mut self, namespace: Hash, hash: Hash, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(TransportError::Budget);
        }
        let object: crate::core::sync::Object =
            crate::core::sync::decode(bytes).map_err(TransportError::Verification)?;
        if object.hash().map_err(TransportError::Verification)? != hash {
            return Err(TransportError::Verification(
                crate::core::sync::Error::HashMismatch,
            ));
        }
        let key = format!("spaces/{namespace}/objects/{hash}.json");
        // Equal content always has equal key. Correctness does not depend on
        // gateway conditional-write support; hostile replacement is hash-checked.
        check(
            self.request(Method::PUT, Some(&key), vec![], bytes)?
                .status(),
        )
    }
}

#[derive(Deserialize)]
struct Listing {
    #[serde(rename = "IsTruncated")]
    truncated: bool,
    #[serde(rename = "NextContinuationToken")]
    next: Option<String>,
    #[serde(rename = "Contents", default)]
    contents: Vec<Listed>,
}

#[derive(Deserialize)]
struct Listed {
    #[serde(rename = "Key")]
    key: String,
}

fn bounded(response: Response, max: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(TransportError::Budget);
    }
    let mut bytes = Vec::new();
    response
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| TransportError::Unavailable)?;
    if bytes.len() > max {
        return Err(TransportError::Budget);
    }
    Ok(bytes)
}

fn check(status: StatusCode) -> Result<()> {
    match status {
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => Err(TransportError::AccessDenied),
        StatusCode::NOT_FOUND => Err(TransportError::NotFound),
        status if status.is_success() => Ok(()),
        status
            if status.is_server_error()
                || status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::REQUEST_TIMEOUT =>
        {
            Err(TransportError::Unavailable)
        }
        _ => Err(TransportError::InvalidResponse),
    }
}

fn encode(value: &str, slash: bool) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) || (slash && byte == b'/') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    Zeroizing::new(mac.finalize().into_bytes().into())
}

#[allow(clippy::too_many_arguments)]
fn authorization(
    method: &str,
    path: &str,
    query: &str,
    host: &str,
    date: &str,
    timestamp: &str,
    payload_hash: &str,
    region: &str,
    access: &str,
    secret: &str,
) -> String {
    let headers =
        format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{timestamp}\n");
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical =
        format!("{method}\n{path}\n{query}\n{headers}\n{signed_headers}\n{payload_hash}");
    let scope = format!("{date}/{region}/s3/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let initial = Zeroizing::new(format!("AWS4{secret}"));
    let key = hmac(initial.as_bytes(), date.as_bytes());
    let key = hmac(key.as_ref(), region.as_bytes());
    let key = hmac(key.as_ref(), b"s3");
    let key = hmac(key.as_ref(), b"aws4_request");
    format!(
        "AWS4-HMAC-SHA256 Credential={access}/{scope},SignedHeaders={signed_headers},Signature={}",
        hex(hmac(key.as_ref(), to_sign.as_bytes()).as_ref())
    )
}

#[cfg(test)]
mod tests;
