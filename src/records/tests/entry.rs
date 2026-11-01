use crate::records::entry::*;

#[test]
fn old_entries_default_to_no_files_and_binary_round_trips() {
    let old = r#"{"id":"old","title":"Legacy","username":"","password":"","url":"","notes":"","totp":null}"#;
    let mut entry: Entry = serde_json::from_str(old).unwrap();
    assert!(entry.attachments.is_empty());
    assert!(
        !serde_json::to_string(&entry)
            .unwrap()
            .contains("attachments")
    );
    entry
        .add_attachment("秘密.bin".into(), vec![0, 255, 128, 42])
        .unwrap();
    let encoded = serde_json::to_vec(&entry).unwrap();
    assert_eq!(entry, serde_json::from_slice(&encoded).unwrap());
    assert!(!format!("{entry:?}").contains("秘密"));
}

#[test]
fn invalid_names_counts_and_total_size_leave_entry_unchanged() {
    let mut entry = Entry::new();
    for name in ["", " ", "..", "../escape", "C:\\escape", "line\nbreak"] {
        assert!(entry.add_attachment(name.into(), vec![1]).is_err());
        assert!(entry.attachments.is_empty());
    }
    entry
        .add_attachment("limit".into(), vec![42; MAX_ATTACHMENT_BYTES])
        .unwrap();
    assert!(entry.add_attachment("extra".into(), vec![1]).is_err());
    assert_eq!(entry.attachments.len(), 1);
    entry.attachments.clear();
    for _ in 0..MAX_ATTACHMENTS {
        entry.add_attachment("empty".into(), vec![]).unwrap();
    }
    assert!(entry.add_attachment("overflow".into(), vec![]).is_err());
    let duplicate = entry.attachments[0].clone();
    entry.attachments[1] = duplicate;
    assert!(entry.validate_attachments().is_err());
}

#[test]
fn attachment_ids_require_canonical_hex_without_restricting_legacy_entry_ids() {
    let mut entry = Entry::default();
    entry.id = "legacy-local-id".into();
    entry
        .add_attachment("key.bin".into(), vec![0, 255])
        .unwrap();
    for invalid in [
        String::new(),
        "a".repeat(31),
        "a".repeat(33),
        "A".repeat(32),
        "g".repeat(32),
        "é".repeat(16),
    ] {
        entry.attachments[0].id = invalid;
        assert_eq!(
            entry.validate_attachments(),
            Err("Invalid attachment name or identifier")
        );
    }
    entry.attachments[0].id = "0123456789abcdef0123456789abcdef".into();
    entry.validate_attachments().unwrap();
    assert_eq!(entry.id, "legacy-local-id");
}
