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
    let (hash, encoded) = object.encode().unwrap();
    assert_eq!(encoded, expected);
    assert_eq!(object.bytes().unwrap(), expected);
    assert_eq!(
        hash.to_string(),
        "88a4ea5764e7a65286f301e2abfb66cb3c3eca91c753b3618ad7916736c5face"
    );
    assert_eq!(object.hash().unwrap(), hash);
    let parsed = Object::decode(expected, hash).unwrap();
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
fn canonical_decoder_rejects_alternate_json_spellings() {
    for text in [
        r#"{"z":1,"a":2}"#,
        r#"{"a":1,"a":2}"#,
        r#""\u0061""#,
        r#""\u96ea""#,
        "1e-6",
        "1.0",
        "-0",
        "true ",
        " true",
    ] {
        assert!(serde_json::from_str::<serde_json::Value>(text).is_ok());
        assert_eq!(
            decode::<serde_json::Value>(text.as_bytes()),
            Err(Error::InvalidEncoding),
            "{text}"
        );
    }
    let value = serde_json::json!({"z": [true, null, "雪\n\"\\"], "a": 0.000001});
    assert_eq!(
        decode::<serde_json::Value>(&canonical(&value).unwrap()).unwrap(),
        value
    );

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Defaulted {
        #[serde(default)]
        flag: bool,
    }
    assert!(matches!(
        decode::<Defaulted>(b"{}"),
        Err(Error::InvalidEncoding)
    ));
}

#[test]
fn canonical_equality_preserves_values_and_serialization_error_order() {
    use crate::sync::protocol::canonical_eq;
    use std::cell::RefCell;
    for (left, right) in [
        (serde_json::json!(null), serde_json::json!(null)),
        (serde_json::json!(1.0), serde_json::json!(1)),
        (
            serde_json::json!(["雪", 0.000001]),
            serde_json::json!(["雪", 0.000001]),
        ),
        (serde_json::json!({"a": "x"}), serde_json::json!({"a": "y"})),
    ] {
        assert_eq!(
            canonical_eq(&left, &right).unwrap(),
            canonical(&left).unwrap() == canonical(&right).unwrap()
        );
    }
    struct Logged<'a> {
        id: u8,
        fail: bool,
        visits: &'a RefCell<Vec<u8>>,
    }
    impl serde::Serialize for Logged<'_> {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            use serde::ser::{Error as _, SerializeSeq};
            self.visits.borrow_mut().push(self.id);
            let mut sequence = serializer.serialize_seq(Some(1))?;
            sequence.serialize_element(&self.id)?;
            if self.fail {
                Err(S::Error::custom("late serializer error"))
            } else {
                sequence.end()
            }
        }
    }
    for (left_fail, right_fail, expected) in [
        (false, false, Ok(false)),
        (true, false, Err(Error::InvalidEncoding)),
        (false, true, Err(Error::InvalidEncoding)),
        (true, true, Err(Error::InvalidEncoding)),
    ] {
        let visits = RefCell::new(Vec::new());
        let left = Logged {
            id: 1,
            fail: left_fail,
            visits: &visits,
        };
        let right = Logged {
            id: 2,
            fail: right_fail,
            visits: &visits,
        };
        assert_eq!(canonical_eq(&left, &right), expected);
        assert_eq!(
            visits.into_inner(),
            if left_fail { vec![1] } else { vec![1, 2] }
        );
    }
}

#[test]
fn scalar_string_encodings_preserve_all_hex_bytes_precision_and_map_keys() {
    use std::collections::BTreeMap;
    for start in [0u8, 32, 64, 96, 128, 160, 192, 224] {
        let hash = Hash(std::array::from_fn(|index| start + index as u8));
        let expected: String = hash.0.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hash.to_string(), expected);
        assert_eq!(
            serde_json::to_vec(&hash).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(canonical(&hash).unwrap(), canonical(&expected).unwrap());
        assert_eq!(decode::<Hash>(&canonical(&hash).unwrap()).unwrap(), hash);
        let map = BTreeMap::from([(hash, Counter(u64::MAX))]);
        let expected = BTreeMap::from([(expected, u64::MAX.to_string())]);
        assert_eq!(
            serde_json::to_vec(&map).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(canonical(&map).unwrap(), canonical(&expected).unwrap());
    }
    for value in [0, 9, 10, 99, 100, (1u64 << 53) + 1, u64::MAX] {
        let counter = Counter(value);
        let expected = value.to_string();
        assert_eq!(
            serde_json::to_vec(&counter).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(canonical(&counter).unwrap(), canonical(&expected).unwrap());
        assert_eq!(
            decode::<Counter>(&canonical(&counter).unwrap()).unwrap(),
            counter
        );
    }
}

#[test]
fn wire_text_deserialization_preserves_input_modes_and_diagnostics() {
    use serde::{
        Deserialize, Deserializer,
        de::{Error as _, Visitor},
    };
    type TextError = serde::de::value::Error;
    #[derive(Clone)]
    enum Input<'a> {
        Str(&'a str),
        BorrowedStr(&'a str),
        String(String),
        Bytes(&'a [u8]),
        BorrowedBytes(&'a [u8]),
        ByteBuf(Vec<u8>),
    }
    impl<'de> Deserializer<'de> for Input<'de> {
        type Error = TextError;
        fn deserialize_any<V: Visitor<'de>>(
            self,
            _: V,
        ) -> std::result::Result<V::Value, TextError> {
            Err(TextError::custom("must request the String hint"))
        }
        fn deserialize_string<V: Visitor<'de>>(
            self,
            visitor: V,
        ) -> std::result::Result<V::Value, TextError> {
            let result = match self {
                Self::Str(value) => visitor.visit_str(value),
                Self::BorrowedStr(value) => visitor.visit_borrowed_str(value),
                Self::String(value) => visitor.visit_string(value),
                Self::Bytes(value) => visitor.visit_bytes(value),
                Self::BorrowedBytes(value) => visitor.visit_borrowed_bytes(value),
                Self::ByteBuf(value) => visitor.visit_byte_buf(value),
            };
            result.map_err(|error: TextError| TextError::custom(format!("string context: {error}")))
        }
        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str bytes byte_buf
            option unit unit_struct newtype_struct seq tuple tuple_struct map
            struct enum identifier ignored_any
        }
    }
    fn check<T: serde::de::DeserializeOwned + Eq + std::fmt::Debug>(text: &str, expected: T) {
        for input in [
            Input::Str(text),
            Input::BorrowedStr(text),
            Input::String(text.to_owned()),
            Input::Bytes(text.as_bytes()),
            Input::BorrowedBytes(text.as_bytes()),
            Input::ByteBuf(text.as_bytes().to_vec()),
        ] {
            assert_eq!(T::deserialize(input).unwrap(), expected);
        }
        if let Some(first) = text.chars().next() {
            let escaped = format!(
                "\"\\u{:04x}{}\"",
                u32::from(first),
                &text[first.len_utf8()..]
            );
            assert_eq!(serde_json::from_str::<T>(&escaped).unwrap(), expected);
        }
        for value in [
            serde_json::Value::Null,
            serde_json::json!(true),
            serde_json::json!(7),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            let expected = String::deserialize(value.clone()).unwrap_err().to_string();
            assert_eq!(
                T::deserialize(value.clone()).unwrap_err().to_string(),
                expected
            );
        }
        for input in [
            Input::Bytes(&[255, 254]),
            Input::BorrowedBytes(&[255, 254]),
            Input::ByteBuf(vec![255, 254]),
        ] {
            let expected = String::deserialize(input.clone()).unwrap_err().to_string();
            assert_eq!(T::deserialize(input).unwrap_err().to_string(), expected);
        }
    }
    check(&Hash([0xab; 32]).to_string(), Hash([0xab; 32]));
    check("18446744073709551615", Counter(u64::MAX));
    check("AAH_Kg", Blob(vec![0, 1, 255, 42]));
    check("", Blob::default());
    for text in ["01", "+1"] {
        assert_eq!(
            Counter::deserialize(Input::Str(text))
                .unwrap_err()
                .to_string(),
            "noncanonical counter"
        );
    }
    assert_eq!(
        Hash::deserialize(Input::Str("bad"))
            .unwrap_err()
            .to_string(),
        Error::InvalidEncoding.to_string()
    );
    assert_eq!(
        Counter::deserialize(Input::Str("bad"))
            .unwrap_err()
            .to_string(),
        "bad".parse::<u64>().unwrap_err().to_string()
    );
}

#[test]
fn wire_text_parse_errors_keep_the_original_json_context_boundary() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde::{Deserialize, de::DeserializeOwned, de::Error as _};
    fn details(error: serde_json::Error) -> (String, usize, usize, String) {
        (
            error.to_string(),
            error.line(),
            error.column(),
            format!("{:?}", error.classify()),
        )
    }
    fn compare<T: DeserializeOwned>(
        source: &str,
        parse: impl FnOnce(&str) -> serde_json::Result<T>,
    ) {
        let mut legacy = serde_json::Deserializer::from_str(source);
        let expected = String::deserialize(&mut legacy)
            .and_then(|text| parse(&text))
            .err()
            .map(details);
        let mut current = serde_json::Deserializer::from_str(source);
        let direct = T::deserialize(&mut current).err().map(details);
        assert_eq!(direct, expected, "direct: {source}");
        let convenience = serde_json::from_str::<T>(source).err().map(details);
        assert_eq!(convenience, expected, "from_str: {source}");
    }
    for source in [
        "\"bad\"",
        "\"\"",
        "\"\\u0041\"",
        "true",
        "null",
        "\"unterminated",
    ] {
        compare::<Hash>(source, |text| {
            text.parse().map_err(serde_json::Error::custom)
        });
    }
    for source in [
        "\"0\"",
        "\"01\"",
        "\"+1\"",
        "\"bad\"",
        "\"18446744073709551616\"",
        "7",
        "null",
        "\"unterminated",
    ] {
        compare::<Counter>(source, |text| {
            let value = text.parse::<u64>().map_err(serde_json::Error::custom)?;
            if value.to_string() != text {
                return Err(serde_json::Error::custom("noncanonical counter"));
            }
            Ok(Counter(value))
        });
    }
    for source in [
        "\"\"",
        "\"AAH_Kg\"",
        "\"YQ==\"",
        "\"%\"",
        "true",
        "null",
        "\"unterminated",
    ] {
        compare::<Blob>(source, |text| {
            let bytes = URL_SAFE_NO_PAD
                .decode(text)
                .map_err(serde_json::Error::custom)?;
            if URL_SAFE_NO_PAD.encode(&bytes) != text {
                return Err(serde_json::Error::custom("noncanonical bytes"));
            }
            Ok(Blob(bytes))
        });
    }
}

#[test]
fn receive_checks_canonical_encoding_before_object_hash() {
    let bytes = include_bytes!("../fixtures/change.json");
    let noncanonical = [b" ".as_slice(), bytes.as_slice()].concat();
    let oversized = vec![b' '; crate::sync::protocol::MAX_OBJECT_BYTES + 1];
    let (mut state, _) = owner();
    let before = canonical(&state).unwrap();
    for (bytes, expected, error) in [
        (bytes.as_slice(), Hash::default(), Error::HashMismatch),
        (
            noncanonical.as_slice(),
            Hash::digest(b"lockbox.sync.object\0", &noncanonical),
            Error::InvalidEncoding,
        ),
        (b"{".as_slice(), Hash::default(), Error::InvalidEncoding),
        (
            oversized.as_slice(),
            Hash::default(),
            Error::ResourceBudgetExceeded,
        ),
    ] {
        assert_eq!(
            Machine::receive(&mut state, expected, bytes.to_vec()),
            Err(error)
        );
        assert_eq!(canonical(&state).unwrap(), before);
    }
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
