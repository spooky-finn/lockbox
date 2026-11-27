use crate::vault::profile::*;

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
fn profile_roundtrip_preserves_keys_and_keeps_secrets_encrypted() {
    use crate::vault::Vault;
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
    assert_eq!(&data[4..6], &7u16.to_le_bytes());
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
    use crate::vault::Vault;
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

/// The format peeked before the lock may be swapped before the locked read.
/// A shared profile must still end up at its canonical path, never at the
/// symlink a local vault was reached through.
#[cfg(unix)]
#[test]
fn profile_swapped_in_before_the_lock_still_opens_canonically() {
    use crate::vault::Vault;
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.lbx");
    let shared = dir.path().join("shared.lbx");
    let alias = dir.path().join("alias.lbx");
    let master = SecretString::from("master".to_owned());
    drop(Vault::create(&local, &master, FAST).unwrap());
    drop(Vault::create_profile(&shared, &master, FAST, profile()).unwrap());
    std::os::unix::fs::symlink(&local, &alias).unwrap();

    let (link, target) = (alias.clone(), shared.clone());
    crate::vault::set_before_lock(move || {
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
    });
    let vault = Vault::open(&alias, &master).unwrap();
    assert!(vault.profile.as_ref().is_some());
    assert_eq!(vault.path, shared.canonicalize().unwrap());
}
