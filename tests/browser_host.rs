//! Native host rejects legacy vault APIs and malformed input without emitting secrets.
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn match_reads_plaintext_index_with_no_desktop_or_unlocked_vault() {
    let home = tempfile::tempdir().unwrap();
    let directory = home.path().join(".lockbox/browser-sites");
    std::fs::create_dir_all(&directory).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(
        directory.join("fixture.json"),
        br#"["https://saved.example"]"#,
    )
    .unwrap();
    let mut host = Command::new(env!("CARGO_BIN_EXE_lockbox-browser-host"))
        .env("HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = host.stdin.take().unwrap();
    for (id, origin) in [
        (1, "https://saved.example/login"),
        (2, "https://unknown.example"),
        (3, "https://saved.example:444"),
    ] {
        let bytes = serde_json::to_vec(
            &serde_json::json!({"id":id,"request":{"action":"match","origin":origin}}),
        )
        .unwrap();
        input
            .write_all(&(bytes.len() as u32).to_ne_bytes())
            .unwrap();
        input.write_all(&bytes).unwrap();
    }
    drop(input);
    let output = host.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let mut frames = output.stdout.as_slice();
    for (id, matched) in [(1, true), (2, false), (3, false)] {
        let length = u32::from_ne_bytes(frames[..4].try_into().unwrap()) as usize;
        let reply: serde_json::Value = serde_json::from_slice(&frames[4..4 + length]).unwrap();
        assert_eq!(
            reply,
            serde_json::json!({"id":id,"status":"ok","data":{"matched":matched}})
        );
        frames = &frames[4 + length..];
    }
    assert!(frames.is_empty());
}
#[test]
fn legacy_unlock_is_rejected() {
    let mut host = Command::new(env!("CARGO_BIN_EXE_lockbox-browser-host"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let bytes = br#"{"id":1,"request":{"action":"unlock","path":"/tmp/vault","password":"fictional-secret"}}"#;
    let mut input = host.stdin.take().unwrap();
    input
        .write_all(&(bytes.len() as u32).to_ne_bytes())
        .unwrap();
    input.write_all(bytes).unwrap();
    drop(input);
    let result = host.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
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
