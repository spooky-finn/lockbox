//! Inspect serialized bytes without retaining the final output buffer.

use serde::Serialize;
use std::io::{self, Write};

pub(crate) fn json_size(value: &impl Serialize) -> serde_json::Result<usize> {
    let mut counter = ByteCount::default();
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.bytes)
}

pub(crate) fn canonical_size(value: &impl Serialize) -> serde_json::Result<usize> {
    let mut counter = ByteCount::default();
    serde_jcs::to_writer(&mut counter, value)?;
    Ok(counter.bytes)
}

pub(crate) fn canonical_matches(value: &impl Serialize, bytes: &[u8]) -> serde_json::Result<bool> {
    let mut comparison = ByteMatch {
        remaining: Some(bytes),
    };
    serde_jcs::to_writer(&mut comparison, value)?;
    Ok(comparison.remaining.is_some_and(|rest| rest.is_empty()))
}

struct ByteMatch<'a> {
    remaining: Option<&'a [u8]>,
}

impl Write for ByteMatch<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.remaining = self.remaining.and_then(|rest| rest.strip_prefix(bytes));
        // Keep serializing after a mismatch so later serialization errors still surface.
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct ByteCount {
    bytes: usize,
}

impl Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("serialized size overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_comparison_matches_buffered_encoder_exactly() {
        for value in [
            serde_json::Value::Null,
            serde_json::json!(true),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({"z": ["雪🗝\n\"\\", null], "a": 0.000001}),
        ] {
            let bytes = serde_jcs::to_vec(&value).unwrap();
            assert!(canonical_matches(&value, &bytes).unwrap());
            for index in 0..bytes.len() {
                assert!(!canonical_matches(&value, &bytes[..index]).unwrap());
                let mut changed = bytes.clone();
                changed[index] ^= 1;
                assert!(!canonical_matches(&value, &changed).unwrap());
            }
            let mut trailing = bytes.clone();
            trailing.push(b' ');
            assert!(!canonical_matches(&value, &trailing).unwrap());
            let leading = [b" ".as_slice(), bytes.as_slice()].concat();
            assert!(!canonical_matches(&value, &leading).unwrap());
        }
    }

    #[test]
    fn byte_comparison_accepts_arbitrary_chunks_and_keeps_consuming_mismatches() {
        let bytes = "雪🗝".as_bytes();
        let mut comparison = ByteMatch {
            remaining: Some(bytes),
        };
        comparison.write_all(b"").unwrap();
        for chunk in bytes.chunks(1) {
            comparison.write_all(chunk).unwrap();
        }
        comparison.flush().unwrap();
        assert_eq!(comparison.remaining, Some(b"".as_slice()));
        comparison.write_all(b"extra").unwrap();
        assert!(comparison.remaining.is_none());
        comparison.write_all(b"still consumed").unwrap();
        assert!(comparison.remaining.is_none());
    }

    #[test]
    fn canonical_comparison_preserves_late_serialization_errors() {
        struct FailsAfterElement;
        impl Serialize for FailsAfterElement {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::{Error as _, SerializeSeq};
                let mut sequence = serializer.serialize_seq(Some(2))?;
                sequence.serialize_element(&0)?;
                Err(S::Error::custom("late serializer error"))
            }
        }
        let error = canonical_matches(&FailsAfterElement, b"wrong prefix").unwrap_err();
        assert!(error.to_string().contains("late serializer error"));
        assert!(canonical_matches(&f64::NAN, b"null").is_err());
    }

    #[test]
    fn sizes_follow_each_encoder_for_unicode_escapes_and_numbers() {
        let value = serde_json::json!({"z": "雪\n\"\\", "a": 0.000001});
        let json = serde_json::to_vec(&value).unwrap();
        let canonical = serde_jcs::to_vec(&value).unwrap();
        assert_ne!(json.len(), canonical.len());
        assert_eq!(json_size(&value).unwrap(), json.len());
        assert_eq!(canonical_size(&value).unwrap(), canonical.len());
        assert_eq!(json_size(&f64::NAN).unwrap(), 4);
        assert!(canonical_size(&f64::NAN).is_err());
    }
}
