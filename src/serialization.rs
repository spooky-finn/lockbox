//! Measure encoded byte budgets without retaining the final serialized buffer.

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
