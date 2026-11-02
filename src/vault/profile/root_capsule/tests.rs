use crate::vault::profile::tests::profile;
use secrecy::SecretString;

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
