use super::*;

#[test]
fn signed_grant_binds_all_methods_paths_and_write_lengths() {
    let endpoint = "https://gateway.storjshare.io";
    let space = "invite / test %";
    let owner = Owner::new(
        endpoint,
        "testbucket",
        space,
        "ACCESS",
        "LOCAL-SECRET",
        ZERO,
    )
    .unwrap();
    let grant = owner.grant(Duration::from_secs(600)).unwrap();
    let endpoint = url::Url::parse(endpoint).unwrap();
    grant
        .validate(&endpoint, "testbucket", space, ZERO)
        .unwrap();
    let serialized = serde_json::to_string(&grant).unwrap();
    assert!(!serialized.contains("LOCAL-SECRET"));
    assert!(!serialized.contains("/objects/"));
    assert!(
        grant
            .validate(&endpoint, "testbucket", space, Hash([1; 32]))
            .is_err()
    );
    let mut changed = grant.clone();
    changed.urls.swap(0, 5);
    assert!(
        changed
            .validate(&endpoint, "testbucket", space, ZERO)
            .is_err()
    );
    let mut expanded = grant.clone();
    expanded.urls[5].push_str("&acl=");
    assert!(
        expanded
            .validate(&endpoint, "testbucket", space, ZERO)
            .is_err()
    );
    let foreign = url::Url::parse("https://evil.example").unwrap();
    assert!(grant.validate(&foreign, "testbucket", space, ZERO).is_err());
}

#[test]
fn guest_connection_rejects_private_network_targets() {
    for target in [
        "https://127.0.0.1",
        "https://169.254.169.254",
        "https://192.168.1.1",
        "https://[::1]",
        "https://[::ffff:127.0.0.1]",
        "https://[fc00::1]",
    ] {
        assert!(matches!(
            network::guest_client(&url::Url::parse(target).unwrap()),
            Err(TransportError::AccessDenied)
        ));
    }
}

#[test]
fn guest_posts_a_fixed_frame_and_owner_reads_the_original_message() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut stored = vec![0; FRAME_BYTES];
        for method in ["PUT", "GET"] {
            let (mut connection, _) = listener.accept().unwrap();
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(connection.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            assert!(first.starts_with(method));
            let mut length = None;
            for line in reader.by_ref().lines() {
                let line = line.unwrap().to_lowercase();
                if line.is_empty() {
                    break;
                }
                assert!(!line.starts_with("authorization:"));
                if let Some(value) = line.strip_prefix("content-length: ") {
                    length = Some(value.parse::<usize>().unwrap());
                }
            }
            let response: &[u8] = if method == "PUT" {
                assert_eq!(length, Some(FRAME_BYTES));
                reader.read_exact(&mut stored).unwrap();
                &[]
            } else {
                &stored
            };
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .unwrap();
            connection.write_all(response).unwrap();
        }
    });
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("http://{address}/request");
    let cancel = Cancellation::default();
    let mut guest = Guest {
        urls: Grant {
            urls: std::array::from_fn(|_| url.clone()),
        },
        session: ZERO,
        http: client.clone(),
    };
    assert!(matches!(
        guest.put(ZERO, Slot::Bootstrap, b"x", &cancel),
        Err(TransportError::AccessDenied)
    ));
    assert!(matches!(
        guest.get(ZERO, Slot::Request(ZERO), &cancel),
        Err(TransportError::AccessDenied)
    ));
    assert!(matches!(
        guest.get(Hash([42; 32]), Slot::Bootstrap, &cancel),
        Err(TransportError::AccessDenied)
    ));
    assert!(matches!(
        guest.requests(ZERO, None),
        Err(TransportError::AccessDenied)
    ));
    guest
        .put(ZERO, Slot::Request(ZERO), b"sealed message", &cancel)
        .unwrap();
    assert_eq!(
        read(&client, &url, Slot::Request(ZERO), &cancel)
            .unwrap()
            .as_deref(),
        Some(&b"sealed message"[..])
    );
    server.join().unwrap();
}

#[test]
fn overwritten_mail_is_delivered_only_to_its_addressed_request() {
    let first = Hash([1; 32]);
    let second = Hash([2; 32]);
    let message =
        format!(r#"{{"request":"{second}","kind":"rejected","ciphertext":"sealed"}}"#).into_bytes();
    assert!(
        addressed(Slot::Rejected(first), Some(message.clone()))
            .unwrap()
            .is_none()
    );
    assert!(
        addressed(Slot::Offer(second), Some(message.clone()))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        addressed(Slot::Rejected(second), Some(message.clone())).unwrap(),
        Some(message)
    );
}
