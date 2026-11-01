use crate::records::totp::*;

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
