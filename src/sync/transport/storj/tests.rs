use crate::sync::transport::storj::{transport::check, *};
use reqwest::StatusCode;
use std::{io::Read, time::Duration};

#[test]
fn rejects_invalid_signing_regions_before_client_construction() {
    for region in ["", "us east-1", "us-east-1\n", "us-east-1\0"] {
        assert!(matches!(
            Storj::with_region(
                "https://example.invalid",
                "bucket",
                "Team",
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
        assert!(Storj::new(endpoint, "bucket", "Team", "access", "secret").is_err());
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
    for (name, segment, encoded) in [
        (
            "Команда & Team",
            "Команда & Team",
            "%D0%9A%D0%BE%D0%BC%D0%B0%D0%BD%D0%B4%D0%B0%20%26%20Team",
        ),
        ("Team / 100%", "Team %2F 100%25", "Team%20%252F%20100%2525"),
        ("..", "%2E%2E", "%252E%252E"),
        ("%2E%2E", "%252E%252E", "%25252E%25252E"),
    ] {
        named_space_round_trip(name, segment, encoded, false);
    }
}

#[test]
fn encrypted_recovery_objects_round_trip_in_a_separate_root_directory() {
    named_space_round_trip("Recovery", "Recovery", "Recovery", true);
}

fn named_space_round_trip(space_name: &str, segment: &'static str, encoded: &str, recovery: bool) {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut namespace = Hash([2; 32]);
    let (device, certificate) = crate::sync::protocol::DeviceSecret::generate(
        &ed25519_dalek::SigningKey::from_bytes(&[1; 32]),
    )
    .unwrap();
    let state = crate::sync::protocol::Machine::create(
        Hash([1; 32]),
        namespace,
        certificate,
        &device,
        &[3; 32],
        Hash([4; 32]),
    )
    .unwrap();
    let (hash, bytes) = if recovery {
        use crate::sync::protocol::cloud_recovery::{self, CloudBackup};
        let root = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
        let backup = CloudBackup {
            target: cloud_recovery::target(&root, state.descriptor()).unwrap(),
            archive: vec![],
            upload: None,
        };
        let upload = cloud_recovery::prepare(&state, &device, &backup, "Owner", space_name)
            .unwrap()
            .unwrap();
        namespace = cloud_recovery::directory(&root);
        (upload.hash, upload.bytes.0)
    } else {
        (state.genesis, state.objects[&state.genesis].0.clone())
    };
    let expected_bytes = bytes.clone();
    let prefix = if recovery {
        format!("recovery/v1/{namespace}/")
    } else {
        format!("spaces/{segment}/objects/")
    };
    let object_key = if recovery {
        format!("/bucket/{prefix}{hash}.json")
    } else {
        format!("/bucket/spaces/{encoded}/objects/{hash}.json")
    };
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
                assert_eq!(query["prefix"], prefix);
                let xml_prefix = prefix.replace('&', "&amp;");
                format!(
                    "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>{xml_prefix}{hash}.json</Key></Contents></ListBucketResult>"
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
        space_name,
        "fixture-access",
        "fixture-secret",
        "us-east-1",
        false,
    )
    .unwrap();
    let noncanonical = [b" ".as_slice(), bytes.as_slice()].concat();
    let oversized = vec![b' '; MAX_OBJECT_BYTES + 1];
    for (hash, bytes, expected) in [
        (
            Hash::default(),
            bytes.as_slice(),
            TransportError::Verification(crate::sync::protocol::Error::HashMismatch),
        ),
        (
            Hash::digest(b"lockbox.sync.object\0", &noncanonical),
            noncanonical.as_slice(),
            TransportError::Verification(crate::sync::protocol::Error::InvalidEncoding),
        ),
        (
            Hash::default(),
            oversized.as_slice(),
            TransportError::Budget,
        ),
    ] {
        let result = if recovery {
            storage.put_recovery(namespace, hash, bytes)
        } else {
            storage.put(namespace, hash, bytes)
        };
        assert_eq!(result, Err(expected));
    }
    if recovery {
        assert_eq!(
            storage
                .list_recovery(namespace, Some("a+/=&x"))
                .unwrap()
                .objects,
            vec![hash]
        );
        storage.put_recovery(namespace, hash, &bytes).unwrap();
        assert_eq!(storage.get_recovery(namespace, hash).unwrap(), bytes);
    } else {
        assert_eq!(
            storage.list(namespace, Some("a+/=&x")).unwrap().objects,
            vec![hash]
        );
        storage.put(namespace, hash, &bytes).unwrap();
        assert_eq!(storage.get(namespace, hash).unwrap(), bytes);
    }
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
            "Team",
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
