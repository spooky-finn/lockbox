use crate::{
    sharing::actions::*,
    sync::transport::Download,
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::MemoryWitness,
    },
};
use secrecy::SecretString;

fn create(dir: &tempfile::TempDir, name: &str) -> Unlocked {
    let store = ProfileStore::new(Arc::new(MemoryWitness::default()));
    Unlocked::new(
        store
            .create_shared(
                dir.path().join(format!("{name}.lbx")),
                &SecretString::from("master".to_owned()),
                &SecretString::from("recovery".to_owned()),
                KdfParams {
                    memory_kib: 8192,
                    passes: 1,
                    lanes: 1,
                },
                SharedProfile {
                    username: name.into(),
                    space_name: "Test team".into(),
                    provider: ProviderConfiguration {
                        base_url: "https://gateway.storjshare.io".into(),
                        bucket: "fixture".into(),
                        access_key: "fictional-access".into(),
                        secret_key: "fictional-secret".into(),
                    },
                },
            )
            .unwrap(),
    )
}

fn review(u: &mut Unlocked, kind: Kind, bytes: &[u8]) {
    let artifact = Artifact::decode(kind, bytes).unwrap();
    u.sharing.job = Some((
        u.sharing.ticket(&u.vault),
        Worker::ready(Ok(FileReply::Imported(Box::new(artifact)))),
    ));
    u.poll_sharing(100);
    assert!(u.error.is_none(), "{:?}", u.error);
}

fn action(u: &mut Unlocked, action: Action) {
    u.sharing_action(action, 100);
    assert!(u.error.is_none(), "{:?}", u.error);
}

fn deliver(from: &Unlocked, to: &mut Unlocked) {
    to.vault
        .receive_sync(Download {
            incomplete: false,
            objects: from
                .vault
                .sync_state()
                .unwrap()
                .objects
                .iter()
                .map(|(h, b)| (*h, b.0.clone()))
                .collect(),
        })
        .unwrap();
}

#[test]
fn reviewed_desktop_flow_requires_trusted_code_and_supports_roles_and_removal() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = create(&dir, "owner");
    let mut guest = create(&dir, "guest");
    let own_space = guest.vault.space_descriptor().unwrap();
    action(&mut guest, Action::Certificate);
    review(
        &mut owner,
        Kind::Certificate,
        &guest.sharing.output.as_ref().unwrap().bytes,
    );
    owner.sharing_action(Action::Issue(Role::Editor), 100);
    assert!(owner.error.is_some());
    assert!(owner.sharing.output.is_none());
    owner.sharing.recipient_verified = true;
    action(&mut owner, Action::Issue(Role::Editor));
    let offer = owner.sharing.output.clone().unwrap();
    review(&mut guest, Kind::Invitation, &offer.bytes);
    guest.sharing.code = "00".repeat(32);
    guest.sharing_action(Action::Accept, 100);
    assert!(guest.error.is_some());
    assert!(guest.vault.accepted_invitations().is_empty());
    guest.sharing.code = offer.code.unwrap().to_string();
    action(&mut guest, Action::Accept);
    assert!(guest.vault.space_descriptor().unwrap() == own_space);
    review(
        &mut owner,
        Kind::Response,
        &guest.sharing.output.as_ref().unwrap().bytes,
    );
    action(&mut owner, Action::Admit);
    review(
        &mut guest,
        Kind::Receipt,
        &owner.sharing.output.as_ref().unwrap().bytes,
    );
    action(&mut guest, Action::Join);
    assert!(guest.vault.space_descriptor().unwrap() == owner.vault.space_descriptor().unwrap());
    let certificate = guest.vault.device_certificate().unwrap();
    action(
        &mut owner,
        Action::Role(certificate.body.user, Role::Reader),
    );
    deliver(&owner, &mut guest);
    assert_eq!(
        guest.vault.sync_replay().unwrap().unwrap().role,
        Some(Role::Reader)
    );
    let before = canonical(owner.vault.sync_state().unwrap()).unwrap();
    owner.sharing_action(Action::Remove(certificate.body.device), 100);
    assert!(owner.error.is_some());
    assert_eq!(
        canonical(owner.vault.sync_state().unwrap()).unwrap(),
        before
    );
    owner.sharing.confirm_remove = Some(certificate.body.device);
    action(&mut owner, Action::Remove(certificate.body.device));
    deliver(&owner, &mut guest);
    assert!(guest.vault.sync_replay().unwrap().unwrap().role.is_none());
}

#[test]
fn active_draft_blocks_membership_actions_and_preserves_its_contents() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = create(&dir, "owner");
    let before = canonical(owner.vault.sync_state().unwrap()).unwrap();
    owner.start_new();
    owner.draft.as_mut().unwrap().entry.password = "draft private value".into();
    owner.open_sharing();
    owner.sharing_action(Action::Role(Hash([99; 32]), Role::Reader), 100);
    assert!(owner.error.is_some());
    assert_eq!(
        owner.draft.as_ref().unwrap().entry.password,
        "draft private value"
    );
    assert_eq!(
        canonical(owner.vault.sync_state().unwrap()).unwrap(),
        before
    );
}

#[test]
fn stale_dialogs_and_worker_results_cannot_cross_unlocked_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mut u = create(&dir, "owner");
    let old = u.sharing.ticket(&u.vault);
    u.sharing = Sharing::default();
    u.import_sharing(old.clone(), dir.path().join("missing.json"));
    assert!(!u.sharing.busy());
    let certificate = u.vault.device_certificate().unwrap();
    u.sharing.job = Some((
        old,
        Worker::ready(Ok(FileReply::Imported(Box::new(Artifact::Certificate(
            certificate,
        ))))),
    ));
    u.poll_sharing(100);
    assert!(u.sharing.review.is_none());
    action(&mut u, Action::Certificate);
    let old_output = u.sharing.output.clone().unwrap();
    action(&mut u, Action::Certificate);
    let path = dir.path().join("stale.json");
    u.export_sharing(u.sharing.ticket(&u.vault), path.clone(), old_output);
    assert!(!u.sharing.busy());
    assert!(!path.exists());
}

#[test]
fn cancellation_invalidates_the_previous_output_and_an_open_export_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = create(&dir, "owner");
    let guest = create(&dir, "guest");
    review(
        &mut owner,
        Kind::Certificate,
        &canonical(&guest.vault.device_certificate().unwrap()).unwrap(),
    );
    owner.sharing.recipient_verified = true;
    action(&mut owner, Action::Issue(Role::Reader));
    let output = owner.sharing.output.clone().unwrap();
    let ticket = owner.sharing.ticket(&owner.vault);
    owner.export_sharing(
        ticket.clone(),
        dir.path().join("offer.json"),
        output.clone(),
    );
    finish_file_job(&mut owner);
    assert!(owner.sharing.saved_output.is_some());
    let id = owner.vault.issued_invitations(100)[0].id;
    action(&mut owner, Action::Cancel(id));
    assert!(owner.sharing.output.is_none());
    assert!(owner.sharing.saved_output.is_none());
    let path = dir.path().join("cancelled-invitation.json");
    owner.export_sharing(ticket, path.clone(), output);
    assert!(!owner.sharing.busy());
    assert!(!path.exists());
}

#[test]
fn automatic_detection_keeps_artifact_verification_and_rejects_mixed_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = create(&dir, "owner");
    let mut guest = create(&dir, "guest");
    let certificate = guest.vault.device_certificate().unwrap();
    let certificate_bytes = canonical(&certificate).unwrap();
    assert!(matches!(
        Artifact::detect(&certificate_bytes).unwrap(),
        Artifact::Certificate(_)
    ));
    let invitation = owner
        .vault
        .issue_invitation(certificate, Role::Reader, None)
        .unwrap();
    assert!(matches!(
        Artifact::detect(&invitation.bytes().unwrap()).unwrap(),
        Artifact::Invitation(_)
    ));
    let response = guest
        .vault
        .accept_invitation(invitation.clone(), invitation.descriptor())
        .unwrap();
    assert!(matches!(
        Artifact::detect(&response.bytes().unwrap()).unwrap(),
        Artifact::Response(_)
    ));
    let receipt = owner.vault.admit_invitation(response, 0).unwrap();
    assert!(matches!(
        Artifact::detect(&receipt.bytes().unwrap()).unwrap(),
        Artifact::Receipt(_)
    ));
    let mut mixed: serde_json::Value =
        serde_json::from_slice(&invitation.bytes().unwrap()).unwrap();
    mixed["request"] = serde_json::json!({});
    assert!(Artifact::detect(&serde_json::to_vec(&mixed).unwrap()).is_err());
    let mut tampered: serde_json::Value = serde_json::from_slice(&certificate_bytes).unwrap();
    tampered["body"]["device"] = serde_json::json!(Hash([99; 32]));
    assert!(Artifact::detect(&serde_json::to_vec(&tampered).unwrap()).is_err());
    for bytes in [b"{}".as_slice(), b"[]", b"not JSON"] {
        assert!(Artifact::detect(bytes).is_err());
    }
}

fn finish_file_job(u: &mut Unlocked) {
    for _ in 0..1000 {
        u.poll_sharing(100);
        if !u.sharing.busy() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("sharing worker timed out");
}

#[test]
fn generic_import_uses_contents_and_failed_save_retains_retry_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut guest = create(&dir, "guest");
    let mut owner = create(&dir, "owner");
    action(&mut guest, Action::Certificate);
    let output = guest.sharing.output.clone().unwrap();
    let existing = dir.path().join("already-exists.json");
    std::fs::write(&existing, b"keep me").unwrap();
    guest.export_sharing(
        guest.sharing.ticket(&guest.vault),
        existing.clone(),
        output.clone(),
    );
    finish_file_job(&mut guest);
    assert!(guest.error.is_some());
    assert!(guest.sharing.output.is_some());
    assert!(guest.sharing.saved_output.is_none());
    assert_eq!(std::fs::read(existing).unwrap(), b"keep me");
    let path = dir.path().join("arbitrary-name.json");
    guest.export_sharing(guest.sharing.ticket(&guest.vault), path.clone(), output);
    finish_file_job(&mut guest);
    assert!(guest.error.is_none());
    assert!(guest.sharing.output.is_none());
    assert!(guest.sharing.saved_output.is_some());
    owner.import_sharing(owner.sharing.ticket(&owner.vault), path);
    finish_file_job(&mut owner);
    assert!(owner.error.is_none());
    assert!(matches!(
        owner.sharing.review,
        Some(Artifact::Certificate(_))
    ));
}
