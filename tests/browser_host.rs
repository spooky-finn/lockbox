//! Real stdio protocol round trip using a temporary vault and fictional secrets.

use lockbox::{
    records::entry::Entry,
    vault::{KdfParams, Vault},
};
use secrecy::SecretString;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

#[test]
fn native_host_unlock_suggest_generate_save_fill_and_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("browser.lbx");
    let password = SecretString::from("fictional master");
    let mut vault = Vault::create(
        &path,
        &password,
        KdfParams {
            memory_kib: 8192,
            passes: 1,
            lanes: 1,
        },
    )
    .unwrap();
    let mut entry = Entry::new();
    entry.title = "Fictional account".into();
    entry.username = "fictional-user".into();
    entry.password = "fictional-password".into();
    entry.url = "https://accounts.example/login".into();
    let entry_id = entry.id.clone();
    vault.upsert_entry(entry).unwrap();
    drop(vault);

    let mut host = Command::new(env!("CARGO_BIN_EXE_lockbox-browser-host"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = host.stdin.take().unwrap();
    let mut output = host.stdout.take().unwrap();
    let mut id = 0u32;
    let mut request = |body: Value| -> Value {
        id += 1;
        let bytes = serde_json::to_vec(&json!({ "id": id, "request": body })).unwrap();
        input
            .write_all(&(bytes.len() as u32).to_ne_bytes())
            .unwrap();
        input.write_all(&bytes).unwrap();
        input.flush().unwrap();
        let mut length = [0u8; 4];
        output.read_exact(&mut length).unwrap();
        let length = u32::from_ne_bytes(length) as usize;
        assert!(length <= 64 * 1024);
        let mut reply = vec![0; length];
        output.read_exact(&mut reply).unwrap();
        let reply: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(reply["id"], id);
        reply
    };

    assert_eq!(
        request(json!({ "action": "status" }))["data"]["unlocked"],
        false
    );
    assert_eq!(
        request(json!({ "action": "unlock", "path": path, "password": "fictional master" }))["status"],
        "ok"
    );
    let suggestions =
        request(json!({ "action": "suggestions", "origin": "https://accounts.example" }));
    assert_eq!(suggestions["data"]["items"][0]["id"], entry_id);
    assert!(!suggestions.to_string().contains("fictional-password"));
    assert_eq!(
        request(
            json!({ "action": "credential", "origin": "https://evil.example", "entry_id": entry_id })
        )["status"],
        "error"
    );
    assert_eq!(
        request(
            json!({ "action": "generate", "origin": "https://accounts.example", "length": 32, "symbols": true })
        )["status"],
        "ok"
    );
    assert_eq!(
        request(
            json!({ "action": "save_generated", "origin": "https://accounts.example", "entry_id": entry_id, "title": "", "username": "" })
        )["status"],
        "ok"
    );
    let credential = request(
        json!({ "action": "credential", "origin": "https://accounts.example", "entry_id": entry_id }),
    );
    let generated = credential["data"]["password"].as_str().unwrap().to_owned();
    assert_eq!(generated.len(), 32);
    assert_eq!(
        request(json!({ "action": "lock" }))["data"]["unlocked"],
        false
    );
    assert_eq!(
        request(
            json!({ "action": "credential", "origin": "https://accounts.example", "entry_id": entry_id })
        )["status"],
        "error"
    );

    drop(input);
    assert!(host.wait().unwrap().success());
    let vault = Vault::open(&path, &password).unwrap();
    assert_eq!(vault.entries()[0].password, generated);
}

#[test]
fn malformed_native_frame_exits_without_emitting_secrets() {
    let mut host = Command::new(env!("CARGO_BIN_EXE_lockbox-browser-host"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = host.stdin.take().unwrap();
    input.write_all(&4u32.to_ne_bytes()).unwrap();
    input.write_all(b"nope").unwrap();
    drop(input);
    let result = host.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
}
