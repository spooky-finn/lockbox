//! Storj's path-style S3 API with AWS Signature V4. No remote credential probes
//! or uploads occur at construction, and response bodies never enter diagnostics.

use super::sync::{Page, Result, Storage, StorageFactory, TransportError};
use crate::core::sync::{Hash, MAX_OBJECT_BYTES};
use reqwest::{
    Method, StatusCode,
    blocking::{Client, Response},
};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde::Deserialize;
use std::{io::Read, time::Duration};
use zeroize::Zeroizing;

pub struct Storj {
    client: Client,
    endpoint: url::Url,
    bucket: String,
    access: Zeroizing<String>,
    secret: Zeroizing<String>,
    region: String,
}

/// Configuration-only factory; the HTTP client is created on a worker thread.
/// Credentials are erased when the last factory reference is dropped.
pub struct StorjFactory {
    endpoint: String,
    bucket: String,
    access: Zeroizing<String>,
    secret: Zeroizing<String>,
    region: String,
}

impl StorjFactory {
    /// Retain configuration without opening a connection or logging credentials.
    pub fn new(endpoint: &str, bucket: &str, access: &str, secret: &str) -> Self {
        Self::with_region(endpoint, bucket, access, secret, "us-east-1")
    }

    /// Use an explicit signing region; validation occurs in [`StorageFactory::connect`].
    pub fn with_region(
        endpoint: &str,
        bucket: &str,
        access: &str,
        secret: &str,
        region: &str,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            access: Zeroizing::new(access.into()),
            secret: Zeroizing::new(secret.into()),
            region: region.into(),
        }
    }
}

impl StorageFactory for StorjFactory {
    fn connect(&self) -> Result<Box<dyn Storage + Send>> {
        Ok(Box::new(Storj::with_region(
            &self.endpoint,
            &self.bucket,
            &self.access,
            &self.secret,
            &self.region,
        )?))
    }
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
        let bucket = Bucket::new(
            self.endpoint.clone(),
            UrlStyle::Path,
            self.bucket.clone(),
            self.region.clone(),
        )
        .map_err(|_| TransportError::InvalidResponse)?;
        let credentials = Credentials::new(self.access.as_str(), self.secret.as_str());
        let url = match (method.clone(), key) {
            (Method::GET, None) => {
                let mut action = bucket.list_objects_v2(Some(&credentials));
                for (name, value) in query {
                    action.query_mut().insert(name, value);
                }
                action.sign(Duration::from_secs(60))
            }
            (Method::GET, Some(key)) => bucket
                .get_object(Some(&credentials), key)
                .sign(Duration::from_secs(60)),
            (Method::PUT, Some(key)) => bucket
                .put_object(Some(&credentials), key)
                .sign(Duration::from_secs(60)),
            _ => return Err(TransportError::InvalidResponse),
        };
        self.client
            .request(method, url)
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

#[cfg(test)]
mod tests;
