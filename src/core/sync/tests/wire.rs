use super::*;

#[test]
fn change_wire_contract_survives_internal_refactors() {
    let signing = SigningKey::from_bytes(&[11; 32]);
    let body = Change {
        header: ChangeHeader {
            profile: PROFILE.into(),
            space: Hash([0x11; 32]),
            membership: Hash([0x22; 32]),
            epoch: Counter(17),
            author: Hash::digest(b"lockbox.device.v1\0", &signing.verifying_key().to_bytes()),
            sequence: Counter(42),
            previous: Some(Hash([0x33; 32])),
            parents: vec![Hash([0x33; 32]), Hash([0x44; 32])],
            nonce: Blob(vec![0x55; 24]),
        },
        ciphertext: Blob(vec![1, 2, 3, 4, 5]),
    };
    let signed = crypto::sign(body, &signing, CHANGE_DOMAIN).unwrap();
    crypto::verify(&signed, &signing.verifying_key().to_bytes(), CHANGE_DOMAIN).unwrap();
    let object = Object::Change(signed);
    let expected = include_bytes!("../fixtures/change-v0.json");
    assert_eq!(object.bytes().unwrap(), expected);
    assert_eq!(
        object.hash().unwrap().to_string(),
        "f2029d61aef0fdfb6ad9b5f008d4aa26bda81b08433ccd5c1409fa120c0673c0"
    );
    let parsed: Object = decode(expected).unwrap();
    assert_eq!(parsed.hash().unwrap(), object.hash().unwrap());
}

#[test]
fn wire_rejects_duplicates_unknown_fields_and_noncanonical_primitives() {
    assert!(decode::<Counter>(b"\"01\"").is_err());
    assert!(decode::<Counter>(b"\"18446744073709551616\"").is_err());
    assert!(decode::<Blob>(b"\"YQ==\"").is_err());
    assert!(decode::<Hash>(b"\"ABC\"").is_err());
    let (state, _) = owner();
    let bytes = &state.objects[&state.genesis].0;
    let text = String::from_utf8(bytes.clone()).unwrap();
    let duplicate = text.replacen(
        "\"kind\":\"control\"",
        "\"kind\":\"control\",\"kind\":\"control\"",
        1,
    );
    assert!(decode::<Object>(duplicate.as_bytes()).is_err());
    let unknown = text.replacen('{', "{\"unknown\":0,", 1);
    assert!(decode::<Object>(unknown.as_bytes()).is_err());
    assert!(decode::<Object>(&vec![b' '; crate::core::sync::MAX_OBJECT_BYTES + 1]).is_err());
}
