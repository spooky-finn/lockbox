use crate::access::cloud_setup::CloudSetup;

fn valid() -> CloudSetup {
    CloudSetup {
        username: "Алиса".into(),
        space_name: "Команда".into(),
        base_url: "https://gateway.storjshare.io".into(),
        bucket: "lockbox-team".into(),
        access_key: "example-access".to_owned().into(),
        secret_key: "example-secret".to_owned().into(),
    }
}

#[test]
fn accepts_gateway_and_separate_bucket() {
    assert!(valid().validate().is_ok());
}

#[test]
fn rejects_ambiguous_or_unsafe_endpoints_without_echoing_credentials() {
    for endpoint in [
        "http://gateway.storjshare.io",
        "https://user:private@gateway.storjshare.io",
        "https://gateway.storjshare.io/bucket",
        "https://gateway.storjshare.io?secret=private",
        "https://gateway.storjshare.io#fragment",
        "https://",
        " https://gateway.storjshare.io",
    ] {
        let mut setup = valid();
        setup.base_url = endpoint.into();
        let error = setup.validate().unwrap_err();
        assert!(!error.contains("private"));
    }
}

#[test]
fn rejects_missing_identity_bucket_and_credentials() {
    for field in 0..6 {
        let mut setup = valid();
        match field {
            0 => setup.username = "  ".into(),
            1 => setup.space_name = "name\nline".into(),
            2 => setup.bucket = "invalid/bucket".into(),
            3 => setup.bucket = "Bucket".into(),
            4 => setup.access_key.clear(),
            _ => setup.secret_key = "contains space".to_owned().into(),
        }
        assert!(setup.validate().is_err());
    }
}

#[test]
fn form_and_transport_accept_the_same_connection_syntax_without_echoing_secrets() {
    use crate::sync::transport::storj::Storj;

    let longest_bucket = "a".repeat(63);
    let excessive_bucket = "a".repeat(64);
    for (field, value, accepted) in [
        (0, "https://example.invalid", true),
        (0, "https://example.invalid:7777/", true),
        (0, "http://example.invalid", false),
        (0, "https://sentinel-private@example.invalid", false),
        (0, "https://user:sentinel-private@example.invalid", false),
        (0, "https://example.invalid/path", false),
        (0, "https://example.invalid?sentinel-private", false),
        (0, "https://example.invalid#sentinel-private", false),
        (0, " https://example.invalid", false),
        (0, "https://example.invalid\n", false),
        (0, "https://exam\tple.invalid", false),
        (0, "https://example.invalid/\0", false),
        (0, "https://", false),
        (1, "a-b", true),
        (1, longest_bucket.as_str(), true),
        (1, "", false),
        (1, "ab", false),
        (1, excessive_bucket.as_str(), false),
        (1, "Bucket", false),
        (1, "bucket/path", false),
        (1, "bucket.name", false),
        (1, "-bucket", false),
        (1, "bucket-", false),
        (2, "access-key", true),
        (2, "", false),
        (2, "sentinel-private key", false),
        (2, "sentinel-private\0", false),
        (3, "secret/key+value=", true),
        (3, "", false),
        (3, "sentinel-private key", false),
        (3, "sentinel-private\n", false),
    ] {
        let mut setup = valid();
        match field {
            0 => setup.base_url = value.into(),
            1 => setup.bucket = value.into(),
            2 => setup.access_key = value.to_owned().into(),
            _ => setup.secret_key = value.to_owned().into(),
        }
        let form = setup.validate();
        assert_eq!(form.is_ok(), accepted, "form field {field}");
        // Client construction validates syntax without any remote credential probe.
        let transport = Storj::new(
            &setup.base_url,
            &setup.bucket,
            &setup.access_key,
            &setup.secret_key,
        );
        assert_eq!(transport.is_ok(), accepted, "transport field {field}");
        if let Err(error) = form {
            assert!(!error.contains("sentinel-private"));
        }
        if let Err(error) = transport {
            assert!(!error.to_string().contains("sentinel-private"));
        }
    }
}

#[test]
fn password_step_requires_valid_cloud_setup_and_back_wipes_passwords() {
    use crate::access::auth::Locked;
    use crate::access::auth::Mode;
    use crate::access::auth::StorageKind;
    let mut form = Locked::new(None);
    form.mode = Mode::Create;
    form.storage_kind = Some(StorageKind::Cloud);
    form.continue_cloud();
    assert!(!form.cloud_password_step);
    assert!(form.error.is_some());
    form.cloud = valid();
    form.continue_cloud();
    assert!(form.cloud_password_step);
    assert!(form.error.is_none());
    form.password.push_str("master password");
    form.repeat.push_str("master password");
    form.edit_cloud_connection();
    assert!(!form.cloud_password_step);
    assert!(form.password.is_empty());
    assert!(form.repeat.is_empty());
    assert_eq!(form.cloud.secret_key.as_str(), "example-secret");
}
