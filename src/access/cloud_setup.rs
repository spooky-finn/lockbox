//! Draft connection settings. Never serialize these into public configuration.

use crate::sync::transport::provider::{self, FieldError};
use zeroize::Zeroizing;

#[derive(Default)]
pub(crate) struct CloudSetup {
    pub(crate) username: String,
    pub(crate) space_name: String,
    pub(crate) base_url: String,
    pub(crate) bucket: String,
    pub(crate) access_key: Zeroizing<String>,
    pub(crate) secret_key: Zeroizing<String>,
}

impl CloudSetup {
    /// Syntax validation only: no network requests or credential verification.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        for (name, error) in [
            (
                &self.username,
                "Enter a username (up to 256 bytes, without control characters)",
            ),
            (
                &self.space_name,
                "Enter a vault name (up to 256 bytes, without control characters)",
            ),
        ] {
            if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(error);
            }
        }
        provider::validate(
            &self.base_url,
            &self.bucket,
            &self.access_key,
            &self.secret_key,
        )
        .map(|_| ())
        .map_err(|error| match error {
            FieldError::EndpointSyntax => "Enter a Base URL in the form https://address",
            FieldError::Endpoint => "Base URL must be an HTTPS gateway address without credentials, path, query, or fragment",
            FieldError::Bucket => "Bucket: 3–63 characters using a–z, 0–9, and hyphens; start and end with a letter or digit",
            FieldError::AccessKey => "Enter an Access Key without spaces or control characters",
            FieldError::SecretKey => "Enter a Secret Key without spaces or control characters",
        })
    }
}

#[cfg(test)]
#[path = "tests/cloud_setup.rs"]
mod tests;
