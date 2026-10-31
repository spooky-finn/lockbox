use super::*;

#[test]
fn sigv4_matches_independently_computed_aws_example_request() {
    // https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html
    // Published GET vector signs Range as well. The same AWS signing key is
    // checked independently, then our exact three-header request has a fixed
    // cross-checked HMAC result (computed with Python's hashlib/hmac).
    let initial = b"AWS4wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    let key = hmac(initial, b"20130524");
    let key = hmac(key.as_ref(), b"us-east-1");
    let key = hmac(key.as_ref(), b"s3");
    let key = hmac(key.as_ref(), b"aws4_request");
    assert_eq!(
        hex(key.as_ref()),
        "dbb893acc010964918f1fd433add87c70e8b0db6be30c1fbeafefa5ec6ba8378"
    );
    let value = authorization(
        "GET",
        "/test.txt",
        "",
        "examplebucket.s3.amazonaws.com",
        "20130524",
        "20130524T000000Z",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "us-east-1",
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    );
    assert!(value.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,"
    ));
    assert!(value.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"));
    assert!(
        value.ends_with(
            "Signature=df548e2ce037944d03f3e68682813b093763996d597cf890ca3d9037fd231eb4"
        )
    );
}

#[test]
fn s3_query_encoding_and_xml_keep_cursor_opaque() {
    assert_eq!(encode("a /+=&é", false), "a%20%2F%2B%3D%26%C3%A9");
    assert_eq!(encode("/spaces/abc.json", true), "/spaces/abc.json");
    let listing: Listing = quick_xml::de::from_str(r#"<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><IsTruncated>true</IsTruncated><NextContinuationToken>a+/=&amp;x</NextContinuationToken><Contents><Key>file.json</Key></Contents></ListBucketResult>"#).unwrap();
    assert!(listing.truncated);
    assert_eq!(listing.next.as_deref(), Some("a+/=&x"));
    assert_eq!(listing.contents[0].key, "file.json");
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
    let hash = Hash([3; 32]);
    let object_key = format!("/bucket/spaces/{namespace}/objects/{hash}.json");
    let expected_key = object_key.clone();
    let (sender, receiver) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for request_index in 0..2 {
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
                request.push_str(&line);
            }
            sender.send(request.clone()).unwrap();
            let body = if request_index == 0 {
                assert!(request.starts_with("GET /bucket?continuation-token=a%2B%2F%3D%26x&list-type=2&max-keys=1000&prefix=spaces%2F"));
                format!(
                    "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>spaces/{namespace}/objects/{hash}.json</Key></Contents></ListBucketResult>"
                )
            } else {
                assert!(request.starts_with(&format!("GET {expected_key} HTTP/1.1")));
                "ciphertext fixture".into()
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
    let mut storage = Storj {
        client: Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        endpoint: url::Url::parse(&format!("http://{address}")).unwrap(),
        bucket: "bucket".into(),
        access: Zeroizing::new("fixture-access".into()),
        secret: Zeroizing::new("fixture-secret".into()),
        region: "us-east-1".into(),
    };
    assert_eq!(
        storage.list(namespace, Some("a+/=&x")).unwrap().objects,
        vec![hash]
    );
    assert_eq!(storage.get(namespace, hash).unwrap(), b"ciphertext fixture");
    server.join().unwrap();
    for request in receiver.try_iter() {
        assert!(request.contains("authorization: AWS4-HMAC-SHA256 Credential=fixture-access/"));
        assert!(request.contains(&format!("host: {address}")));
        assert!(request.contains(
            "x-amz-content-sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(!request.contains("fixture-secret"));
    }
}
