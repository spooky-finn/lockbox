use super::*;
use crate::infra::{
    sync::Download,
    vault::{KdfParams, ProfileStore, ProviderConfiguration, SharedProfile},
    witness::MemoryWitness,
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
    u.import_sharing(
        old.clone(),
        dir.path().join("missing.json"),
        Kind::Certificate,
    );
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
