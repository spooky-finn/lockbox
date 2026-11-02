use super::*;
use std::io::Cursor;
#[test]
fn master_password_and_legacy_vault_requests_are_rejected() {
    for request in [
        serde_json::json!({"action":"unlock","path":"/tmp/vault.lbx","password":"secret"}),
        serde_json::json!({"action":"credential","origin":"https://example.com","entry_id":"id"}),
        serde_json::json!({"action":"fill","origin":"https://example.com","password":"secret"}),
    ] {
        assert!(serde_json::from_value::<Request>(request).is_err());
    }
}
#[test]
fn origin_requires_https_and_exact_port() {
    assert_eq!(
        origin("https://example.com/a").unwrap(),
        origin("https://example.com:443/b").unwrap()
    );
    assert_ne!(
        origin("https://example.com").unwrap(),
        origin("https://example.com:444").unwrap()
    );
    for website in [
        "http://example.com",
        "https://user:password@example.com",
        "file:///etc/passwd",
        "javascript:alert(1)",
    ] {
        assert!(origin(website).is_err());
    }
}
#[test]
fn frames_are_bounded_and_truncation_is_rejected() {
    assert!(read_message(&mut Cursor::new(vec![])).unwrap().is_none());
    for bytes in [
        vec![1],
        0u32.to_ne_bytes().to_vec(),
        (MAX_MESSAGE as u32 + 1).to_ne_bytes().to_vec(),
        vec![5, 0, 0, 0, 1],
    ] {
        assert!(read_message(&mut Cursor::new(bytes)).is_err());
    }
}
