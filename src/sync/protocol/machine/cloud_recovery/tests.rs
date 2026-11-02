use super::*;

fn fixture() -> (SigningKey, DeviceSecret, SyncState, CloudBackup) {
    let root = SigningKey::from_bytes(&[7; 32]);
    let (device, certificate) = DeviceSecret::generate(&root).unwrap();
    let state = Machine::create(
        Hash([1; 32]),
        Hash([2; 32]),
        certificate,
        &device,
        &[3; 32],
        Hash([4; 32]),
    )
    .unwrap();
    let backup = CloudBackup {
        target: target(&root, state.descriptor()).unwrap(),
        archive: vec![],
        upload: None,
    };
    (root, device, state, backup)
}

#[test]
fn recovery_material_is_stable_and_separate_from_signing_keys() {
    let (root, _, _, backup) = fixture();
    assert_eq!(
        directory(&root),
        directory(&SigningKey::from_bytes(&[7; 32]))
    );
    assert_eq!(
        directory(&root).to_string(),
        "71a4160bae163e040e8d3eedf098c5a3a985d11cb95ceb723c8152ac03f31263"
    );
    assert_eq!(
        crate::sync::identity::hex(material(&root, b"recovery-encryption\0").as_ref()),
        "27e00ad5bb1ec67256823e30847f8376c5d8fbae55cccdb11862a53a27c658c0"
    );
    assert_ne!(directory(&root).0, root.to_bytes());
    assert_ne!(directory(&root).0, backup.target.body.recipient);
    assert_ne!(
        directory(&root),
        directory(&SigningKey::from_bytes(&[8; 32]))
    );
}

#[test]
fn recovered_read_keys_and_archives_never_grant_device_write_authority() {
    let (_, device, state, backup) = fixture();
    let keys = Machine::recovery_epoch_keys(&state, &device, &backup.archive).unwrap();
    let view = Machine::replay_recovery(&state, &keys).unwrap();
    assert!(view.ready);
    assert!(view.role.is_none());
    assert!(
        crate::sync::transport::publication(&state, &view)
            .unwrap()
            .is_empty()
    );
    let (stranger, _) = DeviceSecret::generate(&SigningKey::from_bytes(&[8; 32])).unwrap();
    let archived = Machine::replay_archived(&state, &stranger, &keys).unwrap();
    assert!(!archived.ready);
    assert!(archived.role.is_none());
}

#[test]
fn capsule_addressing_checks_budget_encoding_and_shape_before_hash_or_decryption() {
    let root = SigningKey::from_bytes(&[7; 32]);
    let wrong_hash = Hash([0; 32]);
    let invalid_encoding = |bytes: &[u8]| {
        assert!(matches!(object_hash(bytes), Err(Error::InvalidEncoding)));
        assert!(matches!(
            open(&root, wrong_hash, bytes),
            Err(Error::InvalidEncoding)
        ));
    };
    for (version, encapsulated_len, ciphertext_len) in [
        (0, 32, 16),
        (2, 32, 16),
        (1, 31, 16),
        (1, 33, 16),
        (1, 32, 0),
        (1, 32, 15),
    ] {
        invalid_encoding(
            &canonical(&Envelope {
                version,
                encapsulated: Blob(vec![0; encapsulated_len]),
                ciphertext: Blob(vec![0; ciphertext_len]),
            })
            .unwrap(),
        );
    }
    let bytes = canonical(&Envelope {
        version: 1,
        encapsulated: Blob(vec![0; 32]),
        ciphertext: Blob(vec![0; 16]),
    })
    .unwrap();
    assert_eq!(
        object_hash(&bytes).unwrap(),
        Hash::digest(OBJECT_DOMAIN, &bytes)
    );
    assert!(matches!(
        open(&root, wrong_hash, &bytes),
        Err(Error::HashMismatch)
    ));
    let mut trailing_space = bytes.clone();
    trailing_space.push(b' ');
    invalid_encoding(&trailing_space);
    let mut extra_field: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    extra_field["extra"] = serde_json::json!(0);
    invalid_encoding(&canonical(&extra_field).unwrap());
    invalid_encoding(b"{}");
    let oversized = vec![b' '; crate::sync::protocol::MAX_OBJECT_BYTES + 1];
    assert!(matches!(
        object_hash(&oversized),
        Err(Error::ResourceBudgetExceeded)
    ));
    assert!(matches!(
        open(&root, wrong_hash, &oversized),
        Err(Error::ResourceBudgetExceeded)
    ));
}

#[test]
fn recovery_proof_preserves_control_addresses_and_rejects_duplicate_or_noncontrol_bytes() {
    let (_, device, mut state, backup) = fixture();
    let mut snapshot = snapshot(&state, &device, &backup, "Owner", "Space").unwrap();
    let proof = proof_state(&snapshot).unwrap();
    assert!(proof.descriptor() == state.descriptor());
    assert_eq!(proof.objects.len(), 1);
    assert_eq!(
        proof.objects[&state.genesis].0,
        state.objects[&state.genesis].0
    );

    let mut noncanonical = snapshot.controls[0].clone();
    noncanonical.0.push(b' ');
    snapshot.controls.push(snapshot.controls[0].clone());
    assert!(matches!(
        proof_state(&snapshot),
        Err(Error::InvalidEncoding)
    ));
    snapshot.controls = vec![noncanonical];
    assert!(matches!(
        proof_state(&snapshot),
        Err(Error::InvalidEncoding)
    ));

    state.intent = Some(
        Machine::prepare(
            &state,
            &device,
            crate::sync::protocol::Transaction {
                mutations: vec![],
                rebased_from: None,
            },
        )
        .unwrap(),
    );
    Machine::complete_intent(&mut state, &device).unwrap();
    snapshot.controls = vec![
        state
            .objects
            .values()
            .find(|bytes| matches!(decode::<Object>(&bytes.0), Ok(Object::Change(_))))
            .unwrap()
            .clone(),
    ];
    assert!(matches!(
        proof_state(&snapshot),
        Err(Error::InvalidEncoding)
    ));
}

#[test]
fn encryption_to_public_recipient_without_controller_signature_is_rejected() {
    let (root, device, state, backup) = fixture();
    let snapshot = snapshot(&state, &device, &backup, "Owner", "Space").unwrap();
    let plaintext = Zeroizing::new(
        canonical(
            &crypto::sign(snapshot, &SigningKey::from_bytes(&[8; 32]), SNAPSHOT_DOMAIN).unwrap(),
        )
        .unwrap(),
    );
    let (_, public) = keypair(&root);
    let (enc, ciphertext) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
            &OpModeS::Base,
            &public,
            INFO,
            &plaintext,
            &directory(&root).0,
            &mut OsRng.unwrap_err(),
        )
        .unwrap();
    let bytes = canonical(&Envelope {
        version: 1,
        encapsulated: Blob(enc.to_bytes().to_vec()),
        ciphertext: Blob(ciphertext),
    })
    .unwrap();
    assert!(matches!(
        open(&root, object_hash(&bytes).unwrap(), &bytes),
        Err(Error::InvalidSignature)
    ));
}

#[test]
fn capsule_encrypts_metadata_and_authenticates_root_publisher_and_bytes() {
    let (root, device, state, backup) = fixture();
    let upload = prepare(
        &state,
        &device,
        &backup,
        "Private owner marker",
        "Private space marker",
    )
    .unwrap()
    .unwrap();
    let text = std::str::from_utf8(&upload.bytes.0).unwrap();
    assert!(!text.contains("Private owner marker"));
    assert!(!text.contains("Private space marker"));
    let space = open(&root, upload.hash, &upload.bytes.0).unwrap();
    assert_eq!(space.name(), "Private space marker");
    assert_eq!(space.keys().len(), 1);
    assert!(
        open(
            &SigningKey::from_bytes(&[8; 32]),
            upload.hash,
            &upload.bytes.0
        )
        .is_err()
    );
    let mut changed = upload.bytes.0.clone();
    changed[20] ^= 1;
    assert!(open(&root, upload.hash, &changed).is_err());
    let mut envelope: Envelope = decode(&upload.bytes.0).unwrap();
    envelope.ciphertext.0[0] ^= 1;
    let forged = canonical(&envelope).unwrap();
    assert!(open(&root, object_hash(&forged).unwrap(), &forged).is_err());
}

#[test]
fn unacknowledged_capsule_retries_exact_bytes_and_acknowledged_snapshot_is_deduplicated() {
    let (_, device, mut state, mut backup) = fixture();
    let first = prepare(&state, &device, &backup, "Owner", "Space")
        .unwrap()
        .unwrap();
    backup.upload = Some(first.clone());
    let retry = prepare(&state, &device, &backup, "Owner", "Space")
        .unwrap()
        .unwrap();
    assert_eq!(retry.bytes.0, first.bytes.0);
    backup.upload.as_mut().unwrap().acknowledged = true;
    assert!(
        prepare(&state, &device, &backup, "Owner", "Space")
            .unwrap()
            .is_none()
    );
    state.intent = Some(
        Machine::prepare(
            &state,
            &device,
            crate::sync::protocol::Transaction {
                mutations: vec![],
                rebased_from: None,
            },
        )
        .unwrap(),
    );
    Machine::complete_intent(&mut state, &device).unwrap();
    let next = prepare(&state, &device, &backup, "Owner", "Space")
        .unwrap()
        .unwrap();
    let root = SigningKey::from_bytes(&[7; 32]);
    assert_eq!(
        open(&root, next.hash, &next.bytes.0).unwrap().previous(),
        Some(first.hash)
    );
}
