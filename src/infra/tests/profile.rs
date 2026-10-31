use super::*;

pub(crate) const FAST: KdfParams = KdfParams {
    memory_kib: 8192,
    passes: 1,
    lanes: 1,
};
pub(crate) fn profile() -> BootstrapProfile {
    BootstrapProfile::generate(
        "Alice".into(),
        "Team".into(),
        ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "team-vault".into(),
            access_key: "test-access".into(),
            secret_key: "test-secret".into(),
        },
        &SecretString::from("independent recovery password".to_owned()),
        FAST,
    )
    .unwrap()
}

#[test]
fn root_capsule_requires_recovery_password_and_authenticates_identity() {
    let mut profile = profile();
    assert!(profile.validate());
    let recovery = SecretString::from("independent recovery password".to_owned());
    let root = profile
        .root_capsule
        .open(&recovery, &profile.identity.root_key)
        .unwrap();
    assert_eq!(root.verifying_key().to_bytes(), profile.identity.root_key);
    assert!(
        profile
            .root_capsule
            .open(
                &SecretString::from("master password".to_owned()),
                &profile.identity.root_key
            )
            .is_err()
    );
    assert!(profile.root_capsule.open(&recovery, &[0; 32]).is_err());
    profile.root_capsule.ciphertext[0] ^= 1;
    assert!(
        profile
            .root_capsule
            .open(&recovery, &profile.identity.root_key)
            .is_err()
    );
}

#[test]
fn profile_roundtrip_preserves_keys_and_keeps_secrets_encrypted() {
    use crate::infra::vault::Vault;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.lbx");
    let profile = profile();
    let user_id = profile.identity.user_id;
    let device_id = profile.identity.device_id;
    let (changes, checkpoint) = profile.space.content_keys();
    let master = SecretString::from("local master password".to_owned());
    let vault = Vault::create_profile(&path, &master, FAST, profile).unwrap();
    let unlock_key = vault.export_key();
    assert!(matches!(
        Vault::open(&path, &master),
        Err(VaultError::ProfileInUse)
    ));
    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.lbx");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(matches!(
            Vault::open(alias, &master),
            Err(VaultError::ProfileInUse)
        ));
    }
    vault.save().unwrap(); // replacing the data file must keep the lock
    assert!(matches!(
        Vault::open(&path, &master),
        Err(VaultError::ProfileInUse)
    ));
    let data = std::fs::read(&path).unwrap();
    assert_eq!(&data[4..6], &3u16.to_le_bytes());
    for secret in [
        b"Alice".as_slice(),
        b"test-access",
        b"test-secret",
        changes.as_ref(),
        checkpoint.as_ref(),
    ] {
        assert!(!data.windows(secret.len()).any(|window| window == secret));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    drop(vault);
    assert!(matches!(
        Vault::open(&path, &SecretString::from("wrong".to_owned())),
        Err(VaultError::WrongPassword)
    ));
    let opened = Vault::open(&path, &master).unwrap();
    let profile = opened.profile.as_ref().unwrap();
    assert_eq!(profile.identity.user_id, user_id);
    assert_eq!(profile.identity.device_id, device_id);
    assert_eq!(profile.space.content_keys(), (changes, checkpoint));
    assert_eq!(profile.provider.secret_key, "test-secret");
    profile
        .root_capsule
        .open(
            &SecretString::from("independent recovery password".to_owned()),
            &profile.identity.root_key,
        )
        .unwrap();
    drop(opened);
    let opened = Vault::open_with_key(&path, &unlock_key).unwrap();
    assert_eq!(opened.profile.as_ref().unwrap().identity.user_id, user_id);
    drop(opened);
    let mut data = std::fs::read(&path).unwrap();
    *data.last_mut().unwrap() ^= 1;
    std::fs::write(&path, data).unwrap();
    assert!(matches!(
        Vault::open(&path, &master),
        Err(VaultError::Corrupted)
    ));
}

#[test]
fn creating_profile_never_replaces_existing_file() {
    use crate::infra::vault::Vault;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.lbx");
    std::fs::write(&path, b"original").unwrap();
    assert!(matches!(
        Vault::create_profile(
            &path,
            &SecretString::from("master".to_owned()),
            FAST,
            profile()
        ),
        Err(VaultError::AlreadyExists(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
}
