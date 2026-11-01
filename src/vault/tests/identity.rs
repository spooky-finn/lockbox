use crate::{
    access::mnemonic::RecoveryPhrase,
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, VaultError,
        identity::Identity, witness::MemoryWitness,
    },
};
use secrecy::SecretString;
use std::sync::Arc;

fn kdf() -> KdfParams {
    KdfParams {
        memory_kib: 8192,
        passes: 1,
        lanes: 1,
    }
}

fn configuration(name: &str) -> SharedProfile {
    SharedProfile {
        username: "Alice".into(),
        space_name: name.into(),
        provider: ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "fictional-bucket".into(),
            access_key: "fictional-access".into(),
            secret_key: "fictional-secret".into(),
        },
    }
}

#[test]
fn saved_identity_survives_restart_and_cannot_be_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings/identity.key");
    let password = SecretString::from("identity master password".to_owned());
    let phrase = RecoveryPhrase::generate().unwrap();
    let identity = Identity::from_phrase(&phrase);
    let root = identity.signing_key().to_bytes();
    identity.save_new(&path, &password, kdf()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.windows(32).any(|window| window == root));
    assert!(
        !bytes
            .windows(identity.words().unwrap().len())
            .any(|window| window == identity.words().unwrap().as_bytes())
    );
    assert!(Identity::open(&path, &SecretString::from("wrong".to_owned())).is_err());
    drop(identity);
    let reopened = Identity::open(&path, &password).unwrap();
    assert_eq!(reopened.signing_key().to_bytes(), root);
    assert!(reopened.phrase().unwrap().is_some());
    reopened.save_new(&path, &password, kdf()).unwrap();
    let other = Identity::from_phrase(&RecoveryPhrase::generate().unwrap());
    assert!(matches!(
        other.save_new(&path, &password, kdf()),
        Err(VaultError::IdentityMismatch)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn spaces_share_user_root_and_keep_independent_device_and_space_keys() {
    let directory = tempfile::tempdir().unwrap();
    let password = SecretString::from("master password".to_owned());
    let identity = Identity::from_phrase(&RecoveryPhrase::generate().unwrap());
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let first = store
        .create_shared_with_identity(
            &directory.path().join("first.lbx"),
            &password,
            kdf(),
            configuration("First"),
            &identity,
        )
        .unwrap();
    let second_path = directory.path().join("second.lbx");
    let second = store
        .create_shared_with_identity(
            &second_path,
            &password,
            kdf(),
            configuration("Second"),
            &identity,
        )
        .unwrap();
    let a = first.test_profile().unwrap();
    let b = second.test_profile().unwrap();
    assert_eq!(a.identity.root_key, b.identity.root_key);
    assert_eq!(a.identity.user_id, b.identity.user_id);
    assert_ne!(a.identity.device_id, b.identity.device_id);
    assert_ne!(a.space.space_id, b.space.space_id);
    assert_ne!(a.space.epoch_key(), b.space.epoch_key());
    drop(second);
    assert!(
        store
            .open_with_recovery_phrase(&second_path, &identity.phrase().unwrap().unwrap())
            .is_ok()
    );
}

#[test]
fn existing_profile_root_is_adopted_without_rewriting_it() {
    let directory = tempfile::tempdir().unwrap();
    let password = SecretString::from("master password".to_owned());
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    let path = directory.path().join("existing.lbx");
    let original = store
        .create_shared(
            &path,
            &password,
            &password,
            kdf(),
            configuration("Existing"),
        )
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let identity = Identity::from_vault(&original, &password).unwrap();
    assert!(identity.phrase().unwrap().is_none());
    let identity_path = directory.path().join("identity.key");
    identity.save_new(&identity_path, &password, kdf()).unwrap();
    let reopened = Identity::open(&identity_path, &password).unwrap();
    let next = store
        .create_shared_with_identity(
            &directory.path().join("next.lbx"),
            &password,
            kdf(),
            configuration("Next"),
            &reopened,
        )
        .unwrap();
    assert_eq!(
        original.test_profile().unwrap().identity.root_key,
        next.test_profile().unwrap().identity.root_key
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn damaged_identity_is_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.key");
    let bytes = b"damaged saved identity";
    std::fs::write(&path, bytes).unwrap();
    let identity = Identity::from_phrase(&RecoveryPhrase::generate().unwrap());
    let password = SecretString::from("master password".to_owned());
    assert!(identity.save_new(&path, &password, kdf()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn recovering_identity_changes_password_and_keeps_the_private_key() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.key");
    let password = SecretString::from("old identity password".to_owned());
    let replacement = SecretString::from("replacement identity password".to_owned());
    let phrase = RecoveryPhrase::generate().unwrap();
    let identity = Identity::from_phrase(&phrase);
    identity.save_new(&path, &password, kdf()).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(Identity::recover(&path, &RecoveryPhrase::generate().unwrap(), &replacement).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let recovered = Identity::recover(&path, &phrase, &replacement).unwrap();
    assert_eq!(
        recovered.signing_key().to_bytes(),
        identity.signing_key().to_bytes()
    );
    assert!(Identity::open(&path, &password).is_err());
    assert_eq!(
        Identity::open(&path, &replacement)
            .unwrap()
            .signing_key()
            .to_bytes(),
        identity.signing_key().to_bytes()
    );
}
