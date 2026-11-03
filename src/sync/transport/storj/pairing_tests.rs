use super::*;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

#[test]
fn scoped_s3_mailbox_chunks_round_trip_and_rejects_corruption_and_oversized_manifests() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let objects = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
    let server_objects = objects.clone();
    let session = Hash([7; 32]);
    let request = Hash([8; 32]);
    let ticket = Hash([9; 32]);
    let server = std::thread::spawn(move || {
        for _ in 0..14 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut headers = BTreeMap::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.trim().split_once(':').unwrap();
                headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
            }
            assert!(headers["authorization"].starts_with("AWS4-HMAC-SHA256 "));
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap();
            let target = parts.next().unwrap();
            let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
            let path = url.path().to_owned();
            let (status, bytes) = if method == "PUT" {
                assert!(
                    path.contains(&format!("/pairing/{session}/"))
                        || path.contains(&format!("/pairing/{ticket}/"))
                );
                assert!(!path.contains("/objects/"));
                assert!(!path.contains("/recovery/"));
                let length = headers["content-length"].parse::<usize>().unwrap();
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                server_objects.lock().unwrap().insert(path, bytes);
                ("200 OK", Vec::new())
            } else if url.query_pairs().any(|(key, _)| key == "list-type") {
                let prefix = url
                    .query_pairs()
                    .find(|(key, _)| key == "prefix")
                    .unwrap()
                    .1
                    .into_owned();
                assert!(prefix.ends_with(&format!("pairing/{session}/requests/")));
                (
                    "200 OK",
                    b"<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"
                        .to_vec(),
                )
            } else {
                match server_objects.lock().unwrap().get(&path).cloned() {
                    Some(bytes) => ("200 OK", bytes),
                    None => ("404 Not Found", Vec::new()),
                }
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        }
    });
    let storj = Storj::connect(
        &format!("http://{address}"),
        "bucket",
        "Team / 100%",
        "fictional",
        "fictional-secret",
        "us-east-1",
        false,
    )
    .unwrap();
    let mut store = Store(storj);
    let cancel = Cancellation::default();
    let ticket_bytes = b"fictional signed ticket";
    store
        .put(ticket, Slot::Ticket(ticket), ticket_bytes, &cancel)
        .unwrap();
    assert_eq!(
        store
            .get(ticket, Slot::Ticket(ticket), &cancel)
            .unwrap()
            .unwrap(),
        ticket_bytes
    );
    let bytes = vec![42; CHUNK_BYTES + 23];
    store
        .put(session, Slot::Offer(request), &bytes, &cancel)
        .unwrap();
    assert_eq!(
        store
            .get(session, Slot::Offer(request), &cancel)
            .unwrap()
            .unwrap(),
        bytes
    );
    assert!(store.requests(session, None).unwrap().objects.is_empty());
    let chunk_key = objects
        .lock()
        .unwrap()
        .keys()
        .find(|key| key.contains("/chunks/"))
        .unwrap()
        .clone();
    objects.lock().unwrap().get_mut(&chunk_key).unwrap()[0] ^= 1;
    // Corrupt the first chunk deterministically, regardless of hash ordering.
    let first_hash = chunk_hash(&bytes[..CHUNK_BYTES]);
    let first_key = objects
        .lock()
        .unwrap()
        .keys()
        .find(|key| key.ends_with(&format!("{first_hash}.json")))
        .unwrap()
        .clone();
    objects.lock().unwrap().get_mut(&first_key).unwrap()[0] ^= 2;
    assert!(matches!(
        store.get(session, Slot::Offer(request), &cancel),
        Err(TransportError::InvalidResponse)
    ));
    let manifest_key = objects
        .lock()
        .unwrap()
        .keys()
        .find(|key| key.contains("/offers/"))
        .unwrap()
        .clone();
    let oversized = canonical(&Document {
        hash: Hash([0; 32]),
        length: (Slot::Offer(request).budget() + 1) as u64,
        chunks: vec![],
    })
    .unwrap();
    objects.lock().unwrap().insert(manifest_key, oversized);
    assert!(matches!(
        store.get(session, Slot::Offer(request), &cancel),
        Err(TransportError::Budget)
    ));
    server.join().unwrap();
}
