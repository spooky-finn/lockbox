use super::*;

#[test]
fn keys_are_independent_and_identifiers_use_protocol_domains() {
    let root = SigningKey::from_bytes(&random32().unwrap());
    let (device, public) = DeviceKeys::generate(&root).unwrap();
    assert!(device.matches(&public));
    assert_ne!(public.root_key, public.signing_key);
    assert_ne!(public.signing_key, public.hpke_key);
    assert_eq!(
        public.user_id,
        fingerprint(b"lockbox.user.v1\0", &public.root_key)
    );
    let (_, another) = DeviceKeys::generate(&root).unwrap();
    assert_eq!(public.user_id, another.user_id);
    assert_ne!(public.device_id, another.device_id);
    assert!(!device.matches(&another));
    let space = SpaceKeys::generate().unwrap();
    let (changes, checkpoints) = space.content_keys();
    assert_ne!(*changes, *checkpoints);
    let mut info = b"lockbox.changes.v1\0".to_vec();
    info.extend_from_slice(&space.epoch_context);
    let mut expected = [0; 32];
    Hkdf::<Sha256>::new(Some(&space.space_id), &space.epoch_key)
        .expand(&info, &mut expected)
        .unwrap();
    assert_eq!(*changes, expected);
    assert_ne!(space.space_id, space.namespace);
    assert_ne!(space.epoch_key, SpaceKeys::generate().unwrap().epoch_key);
}

#[test]
fn generated_keys_sign_with_strict_verification() {
    use ed25519_dalek::Signer;
    fn requires_secret_wiping<T: zeroize::ZeroizeOnDrop>() {}
    requires_secret_wiping::<ed25519_dalek::SigningKey>();
    fn requires_zeroize<T: zeroize::Zeroize>() {}
    requires_zeroize::<x25519_dalek::StaticSecret>();
    assert!(std::mem::needs_drop::<x25519_dalek::StaticSecret>());
    let root = SigningKey::from_bytes(&random32().unwrap());
    let (device, public) = DeviceKeys::generate(&root).unwrap();
    let key = SigningKey::from_bytes(&device.signing_secret);
    let signature = key.sign(b"test message");
    let verifier = ed25519_dalek::VerifyingKey::from_bytes(&public.signing_key).unwrap();
    verifier.verify_strict(b"test message", &signature).unwrap();
    assert!(verifier.verify_strict(b"changed", &signature).is_err());
}
