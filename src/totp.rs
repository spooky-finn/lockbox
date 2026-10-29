//! Time-based one-time passwords (RFC 6238) and `otpauth://totp/` links.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::{Zeroize, Zeroizing};

use crate::entry::{Totp, TotpAlgorithm};

pub const DIGITS: std::ops::RangeInclusive<u8> = 6..=8;
pub const PERIOD: std::ops::RangeInclusive<u32> = 1..=300;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TotpError {
    #[error("секрет TOTP пустой")]
    EmptySecret,
    #[error("секрет TOTP не похож на Base32: проверьте, что он скопирован полностью")]
    InvalidSecret,
    #[error("ссылка должна начинаться с otpauth://totp/")]
    NotTotpLink,
    #[error("коды по счётчику (otpauth://hotp) не поддерживаются")]
    HotpUnsupported,
    #[error("в ссылке нет секрета (параметр secret)")]
    MissingSecret,
    #[error("неизвестный алгоритм: {0}")]
    UnknownAlgorithm(String),
    #[error("число цифр должно быть от 6 до 8")]
    InvalidDigits,
    #[error("период должен быть от 1 до 300 секунд")]
    InvalidPeriod,
    #[error("ссылка otpauth повреждена")]
    MalformedLink,
}

/// Uppercase, without spaces or padding: the form services print most often.
pub fn normalize_secret(secret: &str) -> String {
    secret
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect::<String>()
        .trim_end_matches('=')
        .to_owned()
}

/// RFC 4648 Base32, ignoring case, spaces and trailing padding.
pub fn decode_base32(secret: &str) -> Result<Zeroizing<Vec<u8>>, TotpError> {
    let secret = Zeroizing::new(normalize_secret(secret));
    if secret.is_empty() {
        return Err(TotpError::EmptySecret);
    }
    // Lengths that no whole number of bytes encodes to: almost always a truncated copy.
    if matches!(secret.len() % 8, 1 | 3 | 6) {
        return Err(TotpError::InvalidSecret);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(secret.len() * 5 / 8));
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in secret.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return Err(TotpError::InvalidSecret),
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    buffer.zeroize();
    Ok(bytes)
}

pub fn validate(totp: &Totp) -> Result<(), TotpError> {
    decode_base32(&totp.secret)?;
    if !DIGITS.contains(&totp.digits) {
        return Err(TotpError::InvalidDigits);
    }
    if !PERIOD.contains(&totp.period) {
        return Err(TotpError::InvalidPeriod);
    }
    Ok(())
}

/// The code valid at `unix_time`, zero-padded to the configured length.
pub fn code(totp: &Totp, unix_time: u64) -> Result<String, TotpError> {
    validate(totp)?;
    let key = decode_base32(&totp.secret)?;
    let counter = unix_time / u64::from(totp.period);
    let digest = hmac(totp.algorithm, &key, counter);
    // RFC 4226 dynamic truncation.
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    let code = binary % 10u32.pow(u32::from(totp.digits));
    Ok(format!("{code:0width$}", width = usize::from(totp.digits)))
}

/// Seconds until the next code, from `period` down to 1.
pub fn seconds_left(totp: &Totp, unix_time: u64) -> u64 {
    let period = u64::from(totp.period.max(1));
    period - unix_time % period
}

/// `482913` → `482 913`, `12345678` → `1234 5678`, for easier reading aloud.
pub fn group(code: &str) -> String {
    let (left, right) = code.split_at(code.len() / 2);
    format!("{left} {right}")
}

fn hmac(algorithm: TotpAlgorithm, key: &[u8], counter: u64) -> Zeroizing<Vec<u8>> {
    macro_rules! digest {
        ($hash:ty) => {{
            let mut mac = Hmac::<$hash>::new_from_slice(key).expect("HMAC accepts any key length");
            mac.update(&counter.to_be_bytes());
            mac.finalize().into_bytes().to_vec()
        }};
    }
    Zeroizing::new(match algorithm {
        TotpAlgorithm::Sha1 => digest!(Sha1),
        TotpAlgorithm::Sha256 => digest!(Sha256),
        TotpAlgorithm::Sha512 => digest!(Sha512),
    })
}

/// What an `otpauth://totp/` link describes.
#[derive(Debug, PartialEq, Eq)]
pub struct Link {
    pub totp: Totp,
    pub issuer: Option<String>,
    pub account: Option<String>,
}

/// Parses `otpauth://totp/Issuer:account?secret=…&issuer=…&algorithm=…&digits=…&period=…`.
pub fn parse_link(link: &str) -> Result<Link, TotpError> {
    let link = link.trim();
    let rest = strip_prefix_ignore_case(link, "otpauth://").ok_or(TotpError::NotTotpLink)?;
    let (kind, rest) = rest.split_once('/').ok_or(TotpError::NotTotpLink)?;
    match kind.to_ascii_lowercase().as_str() {
        "totp" => {}
        "hotp" => return Err(TotpError::HotpUnsupported),
        _ => return Err(TotpError::NotTotpLink),
    }
    let (label, query) = rest.split_once('?').unwrap_or((rest, ""));
    let label = percent_decode(label)?;
    let (mut issuer, account) = match label.split_once(':') {
        Some((issuer, account)) => (non_empty(issuer), non_empty(account)),
        None => (None, non_empty(&label)),
    };

    let mut secret = None;
    let (mut algorithm, mut digits, mut period) = (TotpAlgorithm::Sha1, 6u8, 30u32);
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(&value.replace('+', " "))?;
        match key.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(Zeroizing::new(value)),
            "issuer" => issuer = non_empty(&value).or(issuer),
            "algorithm" => {
                algorithm = match value.to_ascii_uppercase().as_str() {
                    "SHA1" => TotpAlgorithm::Sha1,
                    "SHA256" => TotpAlgorithm::Sha256,
                    "SHA512" => TotpAlgorithm::Sha512,
                    _ => return Err(TotpError::UnknownAlgorithm(value)),
                }
            }
            "digits" => digits = value.parse().map_err(|_| TotpError::InvalidDigits)?,
            "period" => period = value.parse().map_err(|_| TotpError::InvalidPeriod)?,
            _ => {}
        }
    }
    let secret = secret
        .filter(|secret| !secret.trim().is_empty())
        .ok_or(TotpError::MissingSecret)?;
    let totp = Totp {
        secret: normalize_secret(&secret),
        algorithm,
        digits,
        period,
    };
    validate(&totp)?;
    Ok(Link {
        totp,
        issuer,
        account,
    })
}

pub fn is_link(text: &str) -> bool {
    strip_prefix_ignore_case(text.trim(), "otpauth://").is_some()
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn percent_decode(text: &str) -> Result<String, TotpError> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3).ok_or(TotpError::MalformedLink)?;
            decoded.push(u8::from_str_radix(hex, 16).map_err(|_| TotpError::MalformedLink)?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| TotpError::MalformedLink)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_base32(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        let (mut out, mut buffer, mut bits) = (String::new(), 0u32, 0u32);
        for &byte in bytes {
            buffer = (buffer << 8) | u32::from(byte);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
            }
        }
        if bits > 0 {
            out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
        }
        out
    }

    fn totp(seed: &[u8], algorithm: TotpAlgorithm) -> Totp {
        Totp {
            secret: encode_base32(seed),
            algorithm,
            digits: 8,
            period: 30,
        }
    }

    /// Test vectors from RFC 6238, appendix B.
    #[test]
    fn codes_match_rfc_6238_vectors() {
        let sha1 = totp(b"12345678901234567890", TotpAlgorithm::Sha1);
        let sha256 = totp(b"12345678901234567890123456789012", TotpAlgorithm::Sha256);
        let sha512 = totp(
            b"1234567890123456789012345678901234567890123456789012345678901234",
            TotpAlgorithm::Sha512,
        );
        for (time, expected) in [
            (59, ["94287082", "46119246", "90693936"]),
            (1111111109, ["07081804", "68084774", "25091201"]),
            (1111111111, ["14050471", "67062674", "99943326"]),
            (1234567890, ["89005924", "91819424", "93441116"]),
            (2000000000, ["69279037", "90698825", "38618901"]),
            (20000000000, ["65353130", "77737706", "47863826"]),
        ] {
            assert_eq!(code(&sha1, time).unwrap(), expected[0], "SHA1 at {time}");
            assert_eq!(
                code(&sha256, time).unwrap(),
                expected[1],
                "SHA256 at {time}"
            );
            assert_eq!(
                code(&sha512, time).unwrap(),
                expected[2],
                "SHA512 at {time}"
            );
        }
    }

    #[test]
    fn six_digit_codes_keep_leading_zeros() {
        let mut sha1 = totp(b"12345678901234567890", TotpAlgorithm::Sha1);
        sha1.digits = 6;
        assert_eq!(code(&sha1, 1111111109).unwrap(), "081804");
    }

    #[test]
    fn base32_ignores_case_spaces_and_padding() {
        let expected = b"Hello!\xde\xad\xbe\xef".to_vec();
        for input in [
            "JBSWY3DPEHPK3PXP",
            "jbswy3dpehpk3pxp",
            "JBSW Y3DP EHPK 3PXP",
            "JBSWY3DPEHPK3PXP====",
        ] {
            assert_eq!(*decode_base32(input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn broken_secrets_are_rejected() {
        assert_eq!(decode_base32("  ").unwrap_err(), TotpError::EmptySecret);
        assert_eq!(
            decode_base32("JBSWY3D0").unwrap_err(),
            TotpError::InvalidSecret
        );
        assert_eq!(
            decode_base32("JBSWY3DPE").unwrap_err(),
            TotpError::InvalidSecret
        );
    }

    #[test]
    fn full_link_is_parsed() {
        let link = parse_link(
            "otpauth://totp/GitLab%20Inc:a8ee%40example.com?secret=jbswy3dpehpk3pxp\
             &issuer=GitLab&algorithm=SHA256&digits=8&period=60",
        )
        .unwrap();
        assert_eq!(link.issuer.as_deref(), Some("GitLab"));
        assert_eq!(link.account.as_deref(), Some("a8ee@example.com"));
        assert_eq!(link.totp.secret, "JBSWY3DPEHPK3PXP");
        assert_eq!(link.totp.algorithm, TotpAlgorithm::Sha256);
        assert_eq!((link.totp.digits, link.totp.period), (8, 60));
    }

    #[test]
    fn minimal_link_uses_defaults() {
        let link = parse_link("OTPAUTH://TOTP/alice?secret=JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(link.issuer, None);
        assert_eq!(link.account.as_deref(), Some("alice"));
        assert_eq!(link.totp.algorithm, TotpAlgorithm::Sha1);
        assert_eq!((link.totp.digits, link.totp.period), (6, 30));
    }

    #[test]
    fn label_issuer_is_used_without_issuer_parameter() {
        let link = parse_link("otpauth://totp/Почта:me?secret=JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(link.issuer.as_deref(), Some("Почта"));
        assert_eq!(link.account.as_deref(), Some("me"));
    }

    #[test]
    fn unsupported_or_broken_links_are_rejected() {
        for (link, error) in [
            ("https://example.com", TotpError::NotTotpLink),
            (
                "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP&counter=1",
                TotpError::HotpUnsupported,
            ),
            ("otpauth://totp/x?issuer=X", TotpError::MissingSecret),
            ("otpauth://totp/x?secret=JBSWY3D0", TotpError::InvalidSecret),
            (
                "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=10",
                TotpError::InvalidDigits,
            ),
            (
                "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&period=0",
                TotpError::InvalidPeriod,
            ),
            (
                "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&algorithm=MD5",
                TotpError::UnknownAlgorithm("MD5".into()),
            ),
            (
                "otpauth://totp/x%zz?secret=JBSWY3DPEHPK3PXP",
                TotpError::MalformedLink,
            ),
        ] {
            assert_eq!(parse_link(link).unwrap_err(), error, "{link}");
        }
    }

    #[test]
    fn seconds_left_counts_down_to_the_next_code() {
        let sha1 = totp(b"12345678901234567890", TotpAlgorithm::Sha1);
        assert_eq!(seconds_left(&sha1, 60), 30);
        assert_eq!(seconds_left(&sha1, 89), 1);
    }

    #[test]
    fn codes_are_grouped_in_halves() {
        assert_eq!(group("482913"), "482 913");
        assert_eq!(group("12345678"), "1234 5678");
    }
}
