use crate::{access::mnemonic::*, sync::identity::hex};
use secrecy::ExposeSecret;

const WORDS: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[test]
fn bip39_seed_and_slip10_root_match_reference_vectors() {
    let phrase = RecoveryPhrase::parse(WORDS).unwrap();
    // BIP39/Trezor English vector 1 (the standard test passphrase).
    assert_eq!(
        hex(&phrase.0.to_seed_normalized("TREZOR")),
        "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04"
    );
    // Empty passphrase is a fixed part of Lockbox's recovery contract.
    assert_eq!(
        hex(phrase.seed().as_ref()),
        "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4"
    );
    // SLIP-0010 Ed25519 vector 1, chain m.
    let seed: Vec<u8> = (0..16).collect();
    let root = RecoveryPhrase::root_from_seed(&seed);
    assert_eq!(
        hex(&root.to_bytes()),
        "2b4be7f19ee27bbf30c667b642d5f4aa69fd169872f8fc3059c08ebae2eb19e7"
    );
    assert_eq!(
        hex(&root.verifying_key().to_bytes()),
        "a4b2856bfec510abab89753fac1ac0e1112364e7d250545963f135f2a33188ed"
    );
    assert_eq!(
        hex(&phrase.signing_key().to_bytes()),
        "560f9f3c94558b6551928bb781cf6092c6b8800b4fc544af2c9444ed126d51aa"
    );
}

#[test]
fn phrases_require_twelve_known_words_and_valid_checksum() {
    assert!(RecoveryPhrase::parse("").is_err());
    assert!(RecoveryPhrase::parse(&"abandon ".repeat(12)).is_err());
    assert!(RecoveryPhrase::parse(&WORDS.replace("about", "nonsense")).is_err());
    assert!(RecoveryPhrase::parse(&Mnemonic::from_entropy(&[0; 32]).unwrap().to_string()).is_err());
    let spaced = WORDS.replace(' ', "\n  ");
    assert_eq!(
        RecoveryPhrase::parse(&spaced)
            .unwrap()
            .signing_key()
            .to_bytes(),
        RecoveryPhrase::parse(WORDS)
            .unwrap()
            .signing_key()
            .to_bytes()
    );
}

#[test]
fn generated_words_roundtrip_and_key_domains_are_independent() {
    fn wipes_on_drop<T: zeroize::ZeroizeOnDrop>() {}
    wipes_on_drop::<Mnemonic>();
    let first = RecoveryPhrase::generate().unwrap();
    let second = RecoveryPhrase::generate().unwrap();
    let words = first.words();
    assert_eq!(words.expose_secret().split_whitespace().count(), 12);
    assert_eq!(
        first.signing_key().to_bytes(),
        RecoveryPhrase::parse(words.expose_secret())
            .unwrap()
            .signing_key()
            .to_bytes()
    );
    assert_ne!(
        first.signing_key().to_bytes(),
        second.signing_key().to_bytes()
    );
    assert_ne!(*first.wrapping_key(b"one"), *first.wrapping_key(b"two"));
    assert_ne!(*first.wrapping_key(b"one"), first.signing_key().to_bytes());
}
