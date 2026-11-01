use super::*;
use crate::vault::KdfParams;
use std::io::Cursor;

const FAST: KdfParams = KdfParams {
    memory_kib: 8 * 1024,
    passes: 1,
    lanes: 1,
};

fn fixture() -> (tempfile::TempDir, Session, String) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("browser.lbx");
    let mut vault = Vault::create(&path, &SecretString::from("fictional master"), FAST).unwrap();
    let mut entry = Entry::new();
    entry.title = "Fictional login".into();
    entry.url = "https://accounts.example/login".into();
    entry.username = "fictional-user".into();
    entry.password = "fictional-password".into();
    entry.notes = "retain these notes".into();
    entry
        .add_attachment("example.txt".into(), b"fictional attachment".to_vec())
        .unwrap();
    let id = entry.id.clone();
    vault.upsert_entry(entry).unwrap();
    drop(vault);
    let mut session = Session::default();
    session
        .handle(Request::Unlock {
            path: path.to_string_lossy().into_owned(),
            password: Zeroizing::new("fictional master".into()),
        })
        .unwrap();
    (directory, session, id)
}

#[test]
fn origins_require_https_and_match_effective_port_exactly() {
    assert_eq!(
        origin("https://ACCOUNTS.example:443/path?q=1#fragment").unwrap(),
        "https://accounts.example"
    );
    for invalid in [
        "http://accounts.example",
        "file:///secret",
        "https://user:pass@accounts.example",
        "javascript:alert(1)",
        "accounts.example",
    ] {
        assert!(origin(invalid).is_err(), "{invalid}");
    }
    let mut entry = Entry::default();
    entry.url = "https://accounts.example/login".into();
    for other in [
        "https://evilaccounts.example",
        "https://accounts.example.attacker.test",
        "https://sub.accounts.example",
        "https://accounts.example:444",
        "https://аccounts.example",
    ] {
        assert!(!matches(&entry, other), "{other}");
    }
    assert!(matches(&entry, "https://accounts.example"));
}

#[test]
fn suggestions_exclude_secrets_and_credential_requests_recheck_origin() {
    let (_directory, mut session, id) = fixture();
    let data = session
        .handle(Request::Suggestions {
            origin: "https://accounts.example".into(),
        })
        .unwrap();
    let encoded = serde_json::to_string(&data).unwrap();
    assert!(encoded.contains("fictional-user"));
    for secret in [
        "fictional-password",
        "retain these notes",
        "fictional attachment",
        "fictional master",
    ] {
        assert!(!encoded.contains(secret));
    }
    assert!(
        session
            .handle(Request::Credential {
                origin: "https://accounts.example.attacker.test".into(),
                entry_id: id.clone()
            })
            .is_err()
    );
    assert!(matches!(
        session
            .handle(Request::Credential {
                origin: "https://accounts.example".into(),
                entry_id: id
            })
            .unwrap(),
        Data::Credential { .. }
    ));
}

#[test]
fn generated_password_save_preserves_metadata_and_history() {
    let (directory, mut session, id) = fixture();
    session
        .handle(Request::Generate {
            origin: "https://accounts.example".into(),
            length: 32,
            symbols: true,
        })
        .unwrap();
    assert!(
        session
            .handle(Request::SaveGenerated {
                origin: "https://other.example".into(),
                title: "".into(),
                username: Zeroizing::new("".into()),
                entry_id: Some(id.clone())
            })
            .is_err()
    );
    session
        .handle(Request::SaveGenerated {
            origin: "https://accounts.example".into(),
            title: "ignored".into(),
            username: Zeroizing::new("ignored".into()),
            entry_id: Some(id.clone()),
        })
        .unwrap();
    assert!(session.generated.is_none());
    session.lock();
    let vault = Vault::open(
        directory.path().join("browser.lbx"),
        &SecretString::from("fictional master"),
    )
    .unwrap();
    let entry = &vault.entries()[0];
    assert_eq!(entry.password.len(), 32);
    assert_ne!(entry.password, "fictional-password");
    assert_eq!(entry.username, "fictional-user");
    assert_eq!(entry.notes, "retain these notes");
    assert_eq!(entry.attachments[0].data, b"fictional attachment");
    assert_eq!(entry.id, id);
    assert_eq!(vault.record_history().unwrap().len(), 2);
}

#[test]
fn idle_and_absolute_deadlines_drop_the_vault_and_generated_password() {
    let (_directory, mut session, _id) = fixture();
    session
        .handle(Request::Generate {
            origin: "https://accounts.example".into(),
            length: 24,
            symbols: false,
        })
        .unwrap();
    let (opened, wall) = session.opened.unwrap();
    session.expire(opened, wall + Duration::from_secs(301));
    assert!(session.vault.is_none());
    assert!(session.generated.is_none());

    let (_directory, mut session, _id) = fixture();
    let (opened, wall) = session.opened.unwrap();
    session.clock.as_mut().unwrap().activity(
        opened + Duration::from_secs(899),
        wall + Duration::from_secs(899),
    );
    session.expire(opened + MAX_SESSION, wall + MAX_SESSION);
    assert!(session.vault.is_none());
}

#[test]
fn status_does_not_extend_the_deadline_and_lock_releases_file_lease() {
    let (directory, mut session, _id) = fixture();
    let now = Instant::now();
    let wall = SystemTime::now();
    session.clock.as_mut().unwrap().activity(
        now - Duration::from_secs(299),
        wall - Duration::from_secs(299),
    );
    session.handle(Request::Status {}).unwrap();
    session.expire(now + Duration::from_secs(2), wall + Duration::from_secs(2));
    assert!(session.vault.is_none());
    assert!(
        session
            .handle(Request::Credential {
                origin: "https://accounts.example".into(),
                entry_id: "any".into()
            })
            .is_err()
    );
    assert!(
        Vault::open(
            directory.path().join("browser.lbx"),
            &SecretString::from("fictional master")
        )
        .is_ok()
    );
}

#[test]
fn failed_save_retains_suggestion_and_original_password() {
    let (directory, mut session, id) = fixture();
    session
        .handle(Request::Generate {
            origin: "https://accounts.example".into(),
            length: 24,
            symbols: true,
        })
        .unwrap();
    // Rename the directory so the leased file's original parent no longer exists.
    let moved = directory.path().with_extension("moved");
    std::fs::rename(directory.path(), &moved).unwrap();
    let result = session.handle(Request::SaveGenerated {
        origin: "https://accounts.example".into(),
        title: "".into(),
        username: Zeroizing::new("".into()),
        entry_id: Some(id),
    });
    std::fs::rename(&moved, directory.path()).unwrap();
    assert!(result.is_err());
    assert!(session.generated.is_some());
    assert_eq!(
        session.vault.as_ref().unwrap().entries()[0].password,
        "fictional-password"
    );
}

#[test]
fn framing_rejects_truncated_oversized_and_empty_frames() {
    assert!(
        read_message(&mut Cursor::new(Vec::<u8>::new()))
            .unwrap()
            .is_none()
    );
    for frame in [
        vec![1],
        0u32.to_ne_bytes().to_vec(),
        (MAX_MESSAGE as u32 + 1).to_ne_bytes().to_vec(),
        [5u32.to_ne_bytes().to_vec(), b"x".to_vec()].concat(),
    ] {
        assert!(read_message(&mut Cursor::new(frame)).is_err());
    }
    let reply = Reply {
        id: 7,
        result: Outcome::Ok {
            data: Data::State { unlocked: false },
        },
    };
    let mut bytes = Vec::new();
    write_reply(&mut bytes, &reply).unwrap();
    let decoded = read_message(&mut Cursor::new(bytes)).unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&decoded).unwrap()["id"],
        7
    );
}

#[test]
fn unknown_fields_and_actions_are_rejected() {
    for text in [
        r#"{"id":1,"request":{"action":"status","password":"unexpected"}}"#,
        r#"{"id":1,"request":{"action":"export_all"}}"#,
        r#"{"id":1,"request":{"action":"status"},"other":true}"#,
    ] {
        assert!(serde_json::from_str::<Message>(text).is_err());
    }
}
