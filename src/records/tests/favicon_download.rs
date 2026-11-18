use super::*;

#[test]
fn rejects_local_special_and_translated_addresses() {
    for value in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.168.1.1",
        "192.0.2.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "64:ff9b::a00:1",
        "2001:db8::1",
        "2002:7f00:1::",
        "3fff::1",
    ] {
        assert!(!public_ip(value.parse().unwrap()), "{value}");
    }
    for value in [
        "1.1.1.1",
        "8.8.8.8",
        "93.184.216.34",
        "2606:4700:4700::1111",
        "2001:4860:4860::8888",
    ] {
        assert!(public_ip(value.parse().unwrap()), "{value}");
    }
    assert!(
        public_addresses(vec![
            "1.1.1.1:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap()
        ])
        .is_none()
    );
}

#[test]
fn website_html_cannot_select_another_origin_http_or_credentials() {
    let page = url::Url::parse("https://example.com/").unwrap();
    let links = icon_links(
        r#"<base href='https://evil.test/'><link rel=icon href='/safe.png'><link rel=icon href='http://example.com/no.png'><link rel=icon href='https://other.test/no.png'><link rel=icon href='https://user:pass@example.com/no.png'><link rel=icon href='https://example.com:444/no.png'>"#,
        &page,
    );
    assert_eq!(links, vec![page.join("safe.png").unwrap()]);
}

#[test]
fn private_destinations_and_http_are_rejected_before_connecting() {
    for url in [
        "http://127.0.0.1:1/favicon.ico",
        "https://127.0.0.1:1/favicon.ico",
        "https://[::1]:1/favicon.ico",
        "https://192.168.1.1:1/favicon.ico",
    ] {
        assert!(download(url).is_none());
    }
}
