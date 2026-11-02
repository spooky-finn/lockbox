use super::*;
use crate::sync::protocol::{canonical, decode};

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
fn counter_text_validation_matches_decimal_formatting_and_error_order() {
    use serde::{
        Deserialize,
        de::value::{Error as TextError, StrDeserializer},
    };
    fn check(text: &str) {
        let expected = text
            .parse::<u64>()
            .map_err(|error| error.to_string())
            .and_then(|value| {
                if value.to_string() == text {
                    Ok(Counter(value))
                } else {
                    Err("noncanonical counter".to_owned())
                }
            });
        let actual = Counter::deserialize(StrDeserializer::<TextError>::new(text))
            .map_err(|error| error.to_string());
        assert_eq!(actual, expected, "{text:?}");
    }
    fn short_inputs(text: &mut String, remaining: usize) {
        check(text);
        if remaining == 0 {
            return;
        }
        for character in ['0', '1', '9', '+', '-', '_', ' ', '\n'] {
            text.push(character);
            short_inputs(text, remaining - 1);
            text.pop();
        }
    }
    short_inputs(&mut String::new(), 5);
    for value in [
        0,
        1,
        9,
        10,
        99,
        100,
        (1u64 << 53) - 1,
        (1u64 << 53) + 1,
        u64::MAX - 1,
        u64::MAX,
    ] {
        for prefix in ["", "+", "-", "0", "00", "+0", "-0", " ", "\t"] {
            for suffix in ["", " ", "\n", "_", "x", "\0"] {
                check(&format!("{prefix}{value}{suffix}"));
            }
        }
    }
    for text in [
        "١",
        "１２",
        "＋1",
        "−1",
        "1\u{2003}",
        "\u{feff}1",
        "18446744073709551616",
        "184467440737095516150",
        "000018446744073709551615",
        "000018446744073709551616",
    ] {
        check(text);
    }
}

#[test]
fn blob_text_matches_strict_decoder_and_reencode_oracle() {
    use base64::{Engine as _, alphabet, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde::{
        Deserialize,
        de::value::{Error as TextError, StrDeserializer},
    };
    fn check(text: &str) {
        let decoded = URL_SAFE_NO_PAD.decode(text);
        if let Ok(bytes) = &decoded {
            assert_eq!(
                URL_SAFE_NO_PAD.encode(bytes),
                text,
                "strict decoder accepted alternate spelling"
            );
        }
        let expected = decoded
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                if URL_SAFE_NO_PAD.encode(&bytes) == text {
                    Ok(Blob(bytes))
                } else {
                    Err("noncanonical bytes".to_owned())
                }
            });
        let actual = Blob::deserialize(StrDeserializer::<TextError>::new(text))
            .map_err(|error| error.to_string());
        assert_eq!(actual, expected, "{text:?}");
    }
    check("");
    let symbols = alphabet::URL_SAFE.as_str().as_bytes();
    for &first in symbols {
        check(std::str::from_utf8(&[first]).unwrap());
        for &second in symbols {
            check(std::str::from_utf8(&[first, second]).unwrap());
            for &third in symbols {
                check(std::str::from_utf8(&[first, second, third]).unwrap());
                let mut prefixed = *b"AAECAwQFAAA";
                prefixed[8..].copy_from_slice(&[first, second, third]);
                check(std::str::from_utf8(&prefixed).unwrap());
            }
        }
    }
    for text in [
        "+", "/", "AA+A", "AA/A", " YQ", "YQ ", "YQ\n", "YQ\r\n", "YQ\0", "YQ==", "YQ=", "YQ===",
        "YR", "Yf", "YWJ", "雪", "é",
    ] {
        check(text);
    }
    for size in [
        0, 1, 2, 3, 4, 16, 24, 32, 64, 80, 95, 96, 97, 255, 256, 257, 1023, 1024, 1025, 8192,
    ] {
        let bytes: Vec<_> = (0..size).map(|index| index as u8).collect();
        let text = URL_SAFE_NO_PAD.encode(&bytes);
        check(&text);
        assert_eq!(
            Blob::deserialize(StrDeserializer::<TextError>::new(&text)).unwrap(),
            Blob(bytes)
        );
        for suffix in ["=", "==", "===", " ", "\n", "%", "+", "/"] {
            check(&format!("{text}{suffix}"));
        }
    }
}

#[test]
fn blob_string_encoding_preserves_empty_small_and_large_payloads() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    #[derive(serde::Serialize)]
    struct Envelope<'a> {
        value: &'a Blob,
    }
    for size in (0..=128).chain([255, 256, 257, 1023, 1024, 1025, 8192]) {
        let bytes: Vec<_> = (0..size).map(|index| (index * 73 + 41) as u8).collect();
        let text = URL_SAFE_NO_PAD.encode(&bytes);
        let blob = Blob(bytes);
        assert_eq!(
            serde_json::to_vec(&blob).unwrap(),
            serde_json::to_vec(&text).unwrap()
        );
        assert_eq!(canonical(&blob).unwrap(), canonical(&text).unwrap());
        let envelope = Envelope { value: &blob };
        let expected = serde_json::json!({"value": &text});
        assert_eq!(
            serde_json::to_vec(&envelope).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(canonical(&envelope).unwrap(), canonical(&expected).unwrap());
        assert_eq!(decode::<Blob>(&canonical(&blob).unwrap()).unwrap(), blob);
    }
}
