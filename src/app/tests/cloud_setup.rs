use super::CloudSetup;

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
fn password_step_requires_valid_cloud_setup_and_back_wipes_passwords() {
    use crate::app::auth::{Locked, Mode, StorageKind};
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
