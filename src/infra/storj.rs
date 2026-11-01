//! Storj's path-style S3 API with AWS Signature V4. No remote credential probes
//! or uploads occur at construction, and response bodies never enter diagnostics.

use super::sync::{Page, Result, Storage, StorageFactory, TransportError};
use crate::core::sync::{Hash, MAX_OBJECT_BYTES};
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, retry::RetryConfig},
    primitives::ByteStream,
};

use tokio::runtime::Runtime;
use zeroize::Zeroizing;

mod transport;

pub struct Storj {
    client: Client,
    runtime: Runtime,
    bucket: String,
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
        let endpoint = super::provider::validate(endpoint, bucket, access, secret)
            .map_err(|_| TransportError::InvalidResponse)?;

        if region.is_empty() || region.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(TransportError::InvalidResponse);
        }
        Self::connect(endpoint.as_str(), bucket, access, secret, region, true)
    }

    fn connect(
        endpoint: &str,
        bucket: &str,
        access: &str,
        secret: &str,
        region: &str,
        https_only: bool,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| TransportError::Unavailable)?;
        let transport = transport::Transport::new(https_only)?;
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region.to_owned()))
            .credentials_provider(Credentials::new(access, secret, None, None, "lockbox"))
            .endpoint_url(endpoint)
            .force_path_style(true)
            .retry_config(RetryConfig::disabled())
            .request_checksum_calculation(
                aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired,
            )
            .response_checksum_validation(
                aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired,
            )
            .http_client(transport)
            .build();
        let client = {
            let _guard = runtime.enter();
            Client::from_conf(config)
        };

        Ok(Self {
            client,
            runtime,
            bucket: bucket.into(),
        })
    }
}

impl Storage for Storj {
    fn list(&mut self, namespace: Hash, cursor: Option<&str>) -> Result<Page> {
        let prefix = format!("spaces/{namespace}/objects/");
        let listing = self
            .runtime
            .block_on(
                self.client
                    .list_objects_v2()
                    .bucket(&self.bucket)
                    .prefix(&prefix)
                    .max_keys(1000)
                    .set_continuation_token(cursor.map(str::to_owned))
                    .send(),
            )
            .map_err(sdk_error)?;
        let truncated = listing
            .is_truncated
            .ok_or(TransportError::InvalidResponse)?;
        let next = listing.next_continuation_token;
        let contents = listing.contents.unwrap_or_default();

        if contents.len() > 1000
            || (truncated && next.as_ref().is_none_or(|s| s.is_empty()))
            || (!truncated && next.is_some())
        {
            return Err(TransportError::InvalidResponse);
        }

        let objects = contents
            .iter()
            .filter_map(|object| {
                object
                    .key
                    .as_deref()?
                    .strip_prefix(&prefix)?
                    .strip_suffix(".json")?
                    .parse()
                    .ok()
            })
            .collect();
        Ok(Page { objects, next })
    }

    fn get(&mut self, namespace: Hash, hash: Hash) -> Result<Vec<u8>> {
        let key = format!("spaces/{namespace}/objects/{hash}.json");
        self.runtime.block_on(async {
            let response = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(key)
                .send()
                .await
                .map_err(sdk_error)?;
            // The transport has already enforced MAX_OBJECT_BYTES before the SDK sees the body.
            response
                .body
                .collect()
                .await
                .map(|bytes| bytes.into_bytes().to_vec())
                .map_err(|_| TransportError::Unavailable)
        })
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
        self.runtime
            .block_on(
                self.client
                    .put_object()
                    .bucket(&self.bucket)
                    .key(key)
                    .content_type("application/json")
                    .body(ByteStream::from(bytes.to_vec()))
                    .send(),
            )
            .map_err(sdk_error)?;
        Ok(())
    }
}

fn sdk_error<E: std::error::Error + 'static>(
    error: aws_sdk_s3::error::SdkError<E>,
) -> TransportError {
    let fallback = match &error {
        aws_sdk_s3::error::SdkError::ResponseError(_)
        | aws_sdk_s3::error::SdkError::ServiceError(_) => TransportError::InvalidResponse,
        _ => TransportError::Unavailable,
    };

    let mut source: &(dyn std::error::Error + 'static) = &error;
    loop {
        if let Some(error) = source.downcast_ref::<TransportError>() {
            return *error;
        }

        match source.source() {
            Some(next) => source = next,
            None => return fallback,
        }
    }
}

#[cfg(test)]
mod tests;
