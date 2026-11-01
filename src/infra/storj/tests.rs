use super::transport::check;
use super::*;
use reqwest::StatusCode;
use std::{io::Read, time::Duration};

#[test]
fn rejects_invalid_signing_regions_before_client_construction() {
    for region in ["", "us east-1", "us-east-1\n", "us-east-1\0"] {
        assert!(matches!(
            Storj::with_region(
                "https://example.invalid",
                "bucket",
                "access",
                "secret",
                region
            ),
            Err(TransportError::InvalidResponse)
        ));
    }
}

#[test]
fn rejects_insecure_endpoints_and_maps_provider_denial_without_body() {
    for endpoint in [
        "http://example.com",
        "https://user:pass@example.com",
        "https://example.com/path",
        "https://example.com?token=secret",
    ] {
        assert!(Storj::new(endpoint, "bucket", "access", "secret").is_err());
    }
    assert_eq!(
        check(StatusCode::FORBIDDEN),
        Err(TransportError::AccessDenied)
    );
    assert_eq!(
        check(StatusCode::SERVICE_UNAVAILABLE),
        Err(TransportError::Unavailable)
    );
}

#[test]
fn signed_path_style_requests_round_trip_through_a_local_s3_server() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let namespace = Hash([2; 32]);
    let (device, certificate) =
        crate::core::sync::DeviceSecret::generate(&ed25519_dalek::SigningKey::from_bytes(&[1; 32]))
            .unwrap();
    let state = crate::core::sync::Machine::create(
        Hash([1; 32]),
        namespace,
        certificate,
        &device,
        &[3; 32],
        Hash([4; 32]),
    )
    .unwrap();
    let hash = state.genesis;
    let bytes = state.objects[&hash].0.clone();
    let expected_bytes = bytes.clone();
    let object_key = format!("/bucket/spaces/{namespace}/objects/{hash}.json");
    let expected_key = object_key.clone();
    let (sender, receiver) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut stored = Vec::new();

        for request_index in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();

                if line == "\r\n" {
                    break;
                }
                assert!(
                    !line.is_empty(),
                    "client closed the request before its headers"
                );
                request.push_str(&line);
            }
            sender.send(request.clone()).unwrap();
            let body = if request_index == 0 {
                let target = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
                let query: std::collections::HashMap<_, _> =
                    url.query_pairs().into_owned().collect();
                assert!(matches!(url.path(), "/bucket" | "/bucket/"));
                assert_eq!(query["continuation-token"], "a+/=&x");
                assert_eq!(query["list-type"], "2");
                assert_eq!(query["max-keys"], "1000");
                assert_eq!(query["prefix"], format!("spaces/{namespace}/objects/"));
                format!(
                    "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>spaces/{namespace}/objects/{hash}.json</Key></Contents></ListBucketResult>"
                )
            } else if request_index == 1 {
                assert!(
                    request.starts_with(&format!("PUT {expected_key}?x-id=PutObject HTTP/1.1"))
                );
                let length: usize = request
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                assert_eq!(length, expected_bytes.len());
                stored.resize(length, 0);
                reader.read_exact(&mut stored).unwrap();
                assert_eq!(stored, expected_bytes);
                String::new()
            } else {
                assert!(
                    request.starts_with(&format!("GET {expected_key}?x-id=GetObject HTTP/1.1"))
                );
                String::from_utf8(stored.clone()).unwrap()
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });
    // Only this private test fixture permits loopback HTTP. Production constructors
    // enforce HTTPS and never follow redirects carrying credentials.
    let mut storage = Storj::connect(
        &format!("http://{address}"),
        "bucket",
        "fixture-access",
        "fixture-secret",
        "us-east-1",
        false,
    )
    .unwrap();
    assert_eq!(
        storage.list(namespace, Some("a+/=&x")).unwrap().objects,
        vec![hash]
    );
    storage.put(namespace, hash, &bytes).unwrap();
    assert_eq!(storage.get(namespace, hash).unwrap(), bytes);
    server.join().unwrap();

    for request in receiver.try_iter() {
        assert!(request.contains("authorization: AWS4-HMAC-SHA256"));
        assert!(request.contains("Credential=fixture-access/"));
        assert!(request.contains("Signature="));
        assert!(request.contains(&format!("host: {address}")));
        assert!(!request.contains("fixture-secret"));
    }
}

#[test]
fn sdk_preserves_denial_redirect_and_response_budget_errors() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    for (status, length, expected) in [
        ("403 Forbidden", 0, TransportError::AccessDenied),
        ("302 Found", 0, TransportError::InvalidResponse),
        ("200 OK", MAX_OBJECT_BYTES + 1, TransportError::Budget),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(!line.is_empty());

                if line == "\r\n" {
                    break;
                }
            }
            write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nLocation: http://127.0.0.1:1/\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut storage = Storj::connect(
            &format!("http://{address}"),
            "bucket",
            "access",
            "secret",
            "us-east-1",
            false,
        )
        .unwrap();
        assert_eq!(storage.get(Hash([1; 32]), Hash([2; 32])), Err(expected));
        server.join().unwrap();
    }
}
