//! Pure provider connection syntax shared by forms and transport construction.
//! Validation performs no I/O and errors never retain supplied credentials.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldError {
    EndpointSyntax,
    Endpoint,
    Bucket,
    AccessKey,
    SecretKey,
}

/// Validate connection fields without constructing a client or testing access.
pub(crate) fn validate(
    endpoint: &str,
    bucket: &str,
    access: &str,
    secret: &str,
) -> Result<url::Url, FieldError> {
    let parsed = url::Url::parse(endpoint).map_err(|_| FieldError::EndpointSyntax)?;

    if endpoint
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
        || parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err(FieldError::Endpoint);
    }

    let bucket = bucket.as_bytes();

    if !(3..=63).contains(&bucket.len())
        || !bucket.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bucket.last().is_some_and(u8::is_ascii_alphanumeric)
        || !bucket
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    {
        return Err(FieldError::Bucket);
    }

    for (key, error) in [
        (access, FieldError::AccessKey),
        (secret, FieldError::SecretKey),
    ] {
        if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(error);
        }
    }

    Ok(parsed)
}
