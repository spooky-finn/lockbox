//! Draft connection settings. Never serialize these into public configuration.

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
        let endpoint = url::Url::parse(&self.base_url)
            .map_err(|_| "Enter a Base URL in the form https://address")?;
        if self.base_url.trim() != self.base_url
            || self.base_url.chars().any(char::is_whitespace)
            || endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
        {
            return Err(
                "Base URL must be an HTTPS gateway address without credentials, path, query, or fragment",
            );
        }
        let bucket = self.bucket.as_bytes();
        if !(3..=63).contains(&bucket.len())
            || !bucket.first().is_some_and(u8::is_ascii_alphanumeric)
            || !bucket.last().is_some_and(u8::is_ascii_alphanumeric)
            || !bucket
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        {
            return Err(
                "Bucket: 3–63 characters using a–z, 0–9, and hyphens; start and end with a letter or digit",
            );
        }
        for (key, error) in [
            (
                &self.access_key,
                "Enter an Access Key without spaces or control characters",
            ),
            (
                &self.secret_key,
                "Enter a Secret Key without spaces or control characters",
            ),
        ] {
            if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(error);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/cloud_setup.rs"]
mod tests;
