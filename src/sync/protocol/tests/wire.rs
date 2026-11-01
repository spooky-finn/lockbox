use crate::sync::protocol::machine::tests::*;

#[test]
fn change_wire_contract_survives_internal_refactors() {
    let signing = SigningKey::from_bytes(&[11; 32]);
    let body = Change {
        header: ChangeHeader {
            space: Hash([0x11; 32]),
            membership: Hash([0x22; 32]),
            epoch: Counter(17),
            author: Hash::digest(b"lockbox.device\0", &signing.verifying_key().to_bytes()),
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
    let expected = include_bytes!("../fixtures/change.json");
    assert_eq!(object.bytes().unwrap(), expected);
    assert_eq!(
        object.hash().unwrap().to_string(),
        "88a4ea5764e7a65286f301e2abfb66cb3c3eca91c753b3618ad7916736c5face"
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
    assert!(decode::<Object>(&vec![b' '; crate::sync::protocol::MAX_OBJECT_BYTES + 1]).is_err());
}

#[test]
fn versioned_objects_and_signature_domains_are_rejected() {
    let bytes = include_bytes!("../fixtures/change.json");
    let object: Object = decode(bytes).unwrap();
    let Object::Change(signed) = object else {
        panic!()
    };
    let key = SigningKey::from_bytes(&[11; 32]).verifying_key().to_bytes();
    for version in 0..=2 {
        let domain = format!("lockbox.sync.change.v{version}\0");
        assert!(crypto::verify(&signed, &key, domain.as_bytes()).is_err());
        let mut change: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        change["object"]["body"]["header"]["profile"] = format!("LBX-SYNC-{version}").into();
        assert!(decode::<Object>(&canonical(&change).unwrap()).is_err());
        let (state, _) = owner();
        let mut control: serde_json::Value =
            serde_json::from_slice(&state.objects[&state.genesis].0).unwrap();
        control["object"]["body"]["epoch"]["profile"] = format!("LBX-SYNC-{version}").into();
        assert!(decode::<Object>(&canonical(&control).unwrap()).is_err());
    }
}

#[test]
fn dependency_hints_keep_order_duplicates_and_variant_limits() {
    use crate::sync::protocol::{ControlTransition, MAX_PARENTS};
    let references: Vec<_> = (0u64..514)
        .map(|index| Hash::digest(b"dependency-hint-test", &index.to_be_bytes()))
        .collect();
    let hints = |object: &Object| object.dependencies().collect::<Vec<_>>();
    let membership = Hash([90; 32]);
    let mut change: Object = decode(include_bytes!("../fixtures/change.json")).unwrap();
    let Object::Change(signed) = &mut change else {
        panic!()
    };
    signed.body.header.membership = membership;
    signed.body.header.parents = references.clone();
    signed.body.header.previous = Some(references[0]);
    let dependencies = hints(&change);
    assert_eq!(dependencies.len(), MAX_PARENTS + 2);
    assert_eq!(dependencies[0], membership);
    assert_eq!(&dependencies[1..=MAX_PARENTS], &references[..MAX_PARENTS]);
    assert_eq!(dependencies[MAX_PARENTS + 1], references[0]);
    let Object::Change(signed) = &mut change else {
        panic!()
    };
    signed.body.header.parents.clear();
    signed.body.header.previous = None;
    assert_eq!(hints(&change), [membership]);

    let (state, _) = owner();
    let mut control: Object = decode(&state.objects[&state.genesis].0).unwrap();
    for (transition, limit) in [
        (None, MAX_PARENTS),
        (Some(ControlTransition::Transfer), MAX_PARENTS),
        (Some(ControlTransition::Recovery), 512),
    ] {
        let Object::Control(signed) = &mut control else {
            panic!()
        };
        signed.body.epoch.previous = Some(membership);
        signed.body.transition = transition;
        signed.body.closed_frontier = references.clone();
        let dependencies = hints(&control);
        assert_eq!(dependencies.len(), limit + 1);
        assert_eq!(dependencies[0], membership);
        assert_eq!(&dependencies[1..], &references[..limit]);
    }
    let Object::Control(signed) = &mut control else {
        panic!()
    };
    signed.body.epoch.previous = None;
    signed.body.closed_frontier.clear();
    assert!(hints(&control).is_empty());
}
