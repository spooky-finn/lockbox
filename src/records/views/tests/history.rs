use super::*;
use crate::{
    records::entry::{Attachment, Totp, TotpAlgorithm},
    settings::language::Language,
};

fn render(before: Option<&Entry>, after: Option<&Entry>, reveal: bool) -> String {
    dioxus_ssr::render_element(record_diff(Language::English, before, after, reveal))
}

#[test]
fn changed_notes_have_context_and_escape_markup() {
    let mut old = Entry::default();
    old.title = "unchanged-title".into();
    old.notes = "context\nold line\nend".into();
    let mut new = old.clone();
    new.notes = "context\n<script>new</script>\nend".into();
    let html = render(Some(&old), Some(&new), false);
    assert!(html.contains("diff-removed"));
    assert!(html.contains("diff-added"));
    assert!(html.contains("diff-context"));
    assert!(!html.contains("unchanged-title"));
    assert!(!html.contains("<script>"));
    use kuchikiki::traits::TendrilSink;
    let document = kuchikiki::parse_html().one(html).document_node;
    assert!(document.select_first("script").is_err());
    assert!(document.text_contents().contains("<script>new</script>"));
}

#[test]
fn secret_changes_stay_masked_until_revealed() {
    let mut old = Entry::default();
    old.password = "old-private-password".into();
    old.totp = Some(Totp {
        secret: "OLDTOTPSECRET".into(),
        algorithm: TotpAlgorithm::Sha1,
        digits: 6,
        period: 30,
    });
    let mut new = old.clone();
    new.password = "new-private-password".into();
    new.totp.as_mut().unwrap().secret = "NEWTOTPSECRET".into();
    let hidden = render(Some(&old), Some(&new), false);
    for value in [
        "old-private-password",
        "new-private-password",
        "OLDTOTPSECRET",
        "NEWTOTPSECRET",
    ] {
        assert!(!hidden.contains(value));
        assert!(render(Some(&old), Some(&new), true).contains(value));
    }
    assert_eq!(hidden.matches("diff-removed").count(), 2);
    assert_eq!(hidden.matches("diff-added").count(), 2);
    assert!(!hidden.contains("diff-context"));
}

#[test]
fn creations_deletions_and_cleared_fields_have_correct_signs() {
    let mut entry = Entry::default();
    entry.title = "Created".into();
    entry.notes = "a\nb".into();
    let created = render(None, Some(&entry), false);
    assert_eq!(created.matches("diff-added").count(), 3);
    assert!(!created.contains("diff-removed"));
    let deleted = render(Some(&entry), None, false);
    assert_eq!(deleted.matches("diff-removed").count(), 3);
    assert!(!deleted.contains("diff-added"));
    let mut cleared = entry.clone();
    cleared.notes.clear();
    let html = render(Some(&entry), Some(&cleared), false);
    assert_eq!(html.matches("diff-removed").count(), 2);
    assert!(!html.contains("diff-added"));
}

#[test]
fn replacing_attachment_bytes_with_same_size_is_visible() {
    let mut old = Entry::default();
    old.attachments = vec![Attachment {
        id: "file".into(),
        name: "backup.bin".into(),
        data: vec![1, 2],
    }];
    let mut new = old.clone();
    new.attachments[0].data = vec![3, 4];
    let html = render(Some(&old), Some(&new), false);
    assert!(html.contains("diff-removed"));
    assert!(html.contains("diff-added"));
    assert_eq!(html.matches("backup.bin").count(), 2);
}
