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
fn suggestion_previews_preserve_the_first_64_unicode_scalars() {
    for value in [
        String::new(),
        "a".repeat(63),
        "a".repeat(64),
        "a".repeat(65),
        "雪🗝e\u{301}\n\"\\".repeat(40),
        "🗝".repeat(100),
    ] {
        assert_eq!(preview(&value), value.chars().take(64).collect::<String>());
    }
}

#[test]
fn suggestion_and_credential_replies_preserve_json_and_preview_boundaries() {
    let (_directory, mut session, id) = fixture();
    let title = "雪🗝e\u{301}\n".repeat(20);
    let username = "u\"\\雪".repeat(24);
    let mut value = session.vault.as_ref().unwrap().entries()[0].clone();
    value.title = title.clone();
    value.username = username.clone();
    session.vault.as_mut().unwrap().upsert_entry(value).unwrap();
    let cases = [
        (
            Request::Suggestions {
                origin: "https://accounts.example".into(),
            },
            serde_json::json!({"kind": "suggestions", "items": [{
                "id": id,
                "title": title.chars().take(64).collect::<String>(),
                "username": username.chars().take(64).collect::<String>(),
            }]}),
        ),
        (
            Request::Credential {
                origin: "https://accounts.example".into(),
                entry_id: id,
            },
            serde_json::json!({"kind": "credential", "username": username, "password": "fictional-password"}),
        ),
    ];
    for (request, expected) in cases {
        let data = session.handle(request).unwrap();
        let actual = serde_json::to_value(Reply {
            id: 73,
            result: Outcome::Ok { data },
        })
        .unwrap();
        assert_eq!(
            actual,
            serde_json::json!({"id": 73, "status": "ok", "data": expected})
        );
    }
}

#[test]
fn origins_require_https_and_match_effective_port_exactly() {
    assert_eq!(
        origin("https://ACCOUNTS.example:443/path?q=1#fragment")
            .unwrap()
            .ascii_serialization(),
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
        assert!(!matches(&entry, &origin(other).unwrap()), "{other}");
    }
    assert!(matches(
        &entry,
        &origin("https://accounts.example").unwrap()
    ));
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
fn credential_reads_and_generated_updates_enforce_the_same_account_origin() {
    let (_directory, mut session, id) = fixture();
    let original = session.vault.as_ref().unwrap().entries()[0].clone();

    for website in [
        "https://accounts.example.attacker.test",
        "https://sub.accounts.example",
        "https://accounts.example:444",
    ] {
        session
            .handle(Request::Generate {
                origin: website.into(),
                length: 24,
                symbols: false,
            })
            .unwrap();
        assert_eq!(
            session
                .handle(Request::Credential {
                    origin: website.into(),
                    entry_id: id.clone(),
                })
                .err(),
            Some("No matching account for this exact origin.")
        );
        assert_eq!(
            session
                .handle(Request::SaveGenerated {
                    origin: website.into(),
                    title: "".into(),
                    username: Zeroizing::new("".into()),
                    entry_id: Some(id.clone()),
                })
                .err(),
            Some("No matching account for this exact origin.")
        );
        assert!(session.generated.is_some());
        assert_eq!(
            session.vault.as_ref().unwrap().entries(),
            std::slice::from_ref(&original)
        );
    }
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

#[test]
fn origin_equality_and_account_matching_preserve_ascii_origin_semantics() {
    let mut inputs: Vec<String> = [
        "https://accounts.example",
        "https://ACCOUNTS.example:443/path?q=1#fragment",
        "https://accounts.example:444",
        "https://accounts.example:0",
        "https://accounts.example.",
        "https://sub.accounts.example",
        "https://evilaccounts.example",
        "https://bücher.example",
        "https://xn--bcher-kva.example:443/other",
        "https://[::1]",
        "https://[0:0:0:0:0:0:0:1]:443/path",
        "https://[::1]:444",
        "https://127.0.0.1",
        "https://127.000.000.001",
        "http://accounts.example",
        "https://user:pass@accounts.example",
        "https://user@accounts.example",
        "https://:@accounts.example",
        "file:///secret",
        "javascript:alert(1)",
        "",
        "bad",
        "https://",
        "https://accounts.example:65536",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    inputs.push("x".repeat(4097));
    for left in &inputs {
        for right in &inputs {
            let expected = legacy_origin(left)
                .and_then(|left| legacy_origin(right).map(|right| left == right));
            let actual = origin(left).and_then(|left| origin(right).map(|right| left == right));
            assert_eq!(actual, expected, "{left} / {right}");
            if let Ok(website) = origin(right) {
                let mut entry = Entry::default();
                entry.url = left.clone();
                let expected =
                    legacy_origin(left).is_ok_and(|left| left == legacy_origin(right).unwrap());
                assert_eq!(matches(&entry, &website), expected, "{left} / {right}");
            }
        }
    }
}

#[test]
fn new_generated_record_keeps_ascii_origin_as_url_and_default_title() {
    let (_directory, mut session, _) = fixture();
    let website = "https://BÜCHER.example:443/path?x=1#fragment";
    let expected = legacy_origin(website).unwrap();
    session
        .handle(Request::Generate {
            origin: website.into(),
            length: 24,
            symbols: false,
        })
        .unwrap();
    let Data::Saved { entry_id } = session
        .handle(Request::SaveGenerated {
            origin: format!("{expected}/other"),
            title: " ".into(),
            username: Zeroizing::new("user".into()),
            entry_id: None,
        })
        .unwrap()
    else {
        panic!()
    };
    let saved = session
        .vault
        .as_ref()
        .unwrap()
        .entries()
        .iter()
        .find(|entry| entry.id == entry_id)
        .unwrap();
    assert_eq!(saved.url, expected);
    assert_eq!(saved.title, expected);
    assert_eq!(saved.username, "user");
}

fn legacy_origin(value: &str) -> Result<String, &'static str> {
    if value.len() > 4096 {
        return Err("Use an HTTPS website.");
    }
    let url = Url::parse(value).map_err(|_| "Use an HTTPS website.")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Use an HTTPS website without embedded credentials.");
    }
    Ok(url.origin().ascii_serialization())
}
