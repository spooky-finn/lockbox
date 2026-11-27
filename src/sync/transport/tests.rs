use crate::sync::{
    protocol::{DeviceSecret, Machine},
    transport::*,
};
use ed25519_dalek::SigningKey;
use std::collections::BTreeMap;

#[derive(Default)]
struct Memory {
    objects: BTreeMap<Hash, Vec<u8>>,
    lost_response: bool,
    repeated_cursor: bool,
    missing: bool,
    hidden: BTreeSet<Hash>,
    /// Per-key GET failures, such as a header the transport cannot read.
    errors: BTreeMap<Hash, TransportError>,
    gets: usize,
}

impl Storage for Memory {
    fn list(&mut self, _: Hash, _: Option<&str>) -> Result<Page> {
        Ok(Page {
            objects: self
                .objects
                .keys()
                .rev()
                .filter(|hash| !self.hidden.contains(hash))
                .copied()
                .collect(),
            next: self.repeated_cursor.then(|| "same".into()),
        })
    }
    fn get(&mut self, _: Hash, hash: Hash) -> Result<Vec<u8>> {
        self.gets += 1;

        if self.missing {
            return Err(TransportError::NotFound);
        }

        if let Some(error) = self.errors.get(&hash) {
            return Err(*error);
        }

        self.objects
            .get(&hash)
            .cloned()
            .ok_or(TransportError::NotFound)
    }
    fn put(&mut self, _: Hash, hash: Hash, bytes: &[u8]) -> Result<()> {
        self.objects.insert(hash, bytes.to_vec());

        if std::mem::take(&mut self.lost_response) {
            Err(TransportError::Unavailable)
        } else {
            Ok(())
        }
    }
}

fn history() -> (SyncState, SyncState, DeviceSecret, Hash) {
    use crate::records::entry::Entry;
    use crate::sync::protocol::Transaction;
    let (mut state, device) = fixture();
    let base = state.clone();
    let mut entry = Entry::new();
    let mut last = state.genesis;

    for title in ["First", "Second", "Third"] {
        entry.title = title.into();
        let before = Machine::replay(&state, &device).unwrap().projection;
        let transaction = Transaction::diff(&before, &[entry.clone()], &[]).unwrap();
        state.intent = Some(Machine::prepare(&state, &device, transaction).unwrap());
        last = Machine::complete_intent(&mut state, &device).unwrap();
    }
    (base, state, device, last)
}

#[test]
fn pinned_genesis_is_fetched_even_when_list_omits_it() {
    let (state, device) = fixture();
    let mut join = Machine::join(
        state.space,
        state.namespace,
        state.genesis,
        state.owner_root,
    );
    let mut provider = Memory {
        objects: state
            .objects
            .iter()
            .map(|(h, b)| (*h, b.0.clone()))
            .collect(),
        hidden: BTreeSet::from([state.genesis]),
        ..Memory::default()
    };

    let batch = Discovery::for_state(&join)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert!(!batch.incomplete);
    assert_eq!(provider.gets, 1);

    for (hash, bytes) in batch.objects {
        Machine::receive(&mut join, hash, bytes).unwrap();
    }
    assert!(Machine::replay(&join, &device).unwrap().ready);
}

#[test]
fn a_required_control_anchor_is_fetched_without_provider_listing() {
    let (mut owner, device) = fixture();
    let members = Machine::members(&owner, &device).unwrap();
    owner.intent = Some(Machine::prepare_membership(&owner, &device, members).unwrap());
    let anchor = Machine::complete_intent(&mut owner, &device).unwrap();
    let mut joined = Machine::join_at(owner.descriptor(), anchor);
    let mut provider = Memory {
        objects: owner
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
        hidden: owner.objects.keys().copied().collect(),
        ..Memory::default()
    };

    let batch = Discovery::for_state(&joined)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert!(!batch.incomplete);
    assert_eq!(provider.gets, owner.objects.len());

    for (hash, bytes) in batch.objects {
        Machine::receive(&mut joined, hash, bytes).unwrap();
    }
    assert!(Machine::replay(&joined, &device).unwrap().ready);
}

#[test]
fn known_orphans_resolve_omitted_dependencies_recursively() {
    let (mut receiver, state, device, last) = history();
    Machine::receive(&mut receiver, last, state.objects[&last].0.clone()).unwrap();
    assert!(!Machine::replay(&receiver, &device).unwrap().ready);
    let mut provider = Memory {
        objects: state
            .objects
            .iter()
            .map(|(h, b)| (*h, b.0.clone()))
            .collect(),
        hidden: state.objects.keys().copied().collect(),
        ..Memory::default()
    };

    let batch = Discovery::for_state(&receiver)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert!(!batch.incomplete);
    assert_eq!(provider.gets, 2);

    for (hash, bytes) in batch.objects {
        Machine::receive(&mut receiver, hash, bytes).unwrap();
    }

    let replay = Machine::replay(&receiver, &device).unwrap();
    assert!(replay.ready);
    assert_eq!(replay.projection.entries()[0].title, "Third");
}

#[test]
fn discovering_a_head_fetches_its_unlisted_ancestry_in_the_same_pass() {
    let (_, state, device, last) = history();
    let mut receiver = Machine::join(
        state.space,
        state.namespace,
        state.genesis,
        state.owner_root,
    );
    let mut provider = Memory {
        objects: state
            .objects
            .iter()
            .map(|(h, b)| (*h, b.0.clone()))
            .collect(),
        hidden: state
            .objects
            .keys()
            .filter(|h| **h != last)
            .copied()
            .collect(),
        ..Memory::default()
    };

    let batch = download(
        &mut provider,
        state.namespace,
        &BTreeSet::new(),
        &Cancellation::default(),
    )
    .unwrap();
    assert!(!batch.incomplete);
    assert_eq!(provider.gets, state.objects.len());

    for (hash, bytes) in batch.objects {
        Machine::receive(&mut receiver, hash, bytes).unwrap();
    }
    assert!(Machine::replay(&receiver, &device).unwrap().ready);
}

#[test]
fn listed_objects_that_cannot_verify_are_skipped_or_rejected_without_failing_the_pass() {
    use crate::sync::protocol::{Error, MAX_OBJECT_BYTES};
    let (state, _) = fixture();
    let bytes = &state.objects[&state.genesis].0;
    let noncanonical = [b" ".as_slice(), bytes.as_slice()].concat();
    let oversized = vec![b' '; MAX_OBJECT_BYTES + 1];
    // Only bytes that hash to their key can condemn it; other junk is a bad copy.
    for (hash, bytes, expected) in [
        (Hash::default(), bytes.as_slice(), None),
        (Hash::default(), b"junk".as_slice(), None),
        (
            Hash::digest(b"lockbox.sync.object\0", &noncanonical),
            noncanonical.as_slice(),
            Some(Error::InvalidEncoding),
        ),
        (Hash::default(), oversized.as_slice(), None),
    ] {
        let mut provider = Memory {
            objects: BTreeMap::from([(hash, bytes.to_vec())]),
            ..Memory::default()
        };
        let batch = download(
            &mut provider,
            state.namespace,
            &BTreeSet::new(),
            &Cancellation::default(),
        )
        .unwrap();
        assert!(batch.objects.is_empty());
        assert!(!batch.incomplete);
        assert_eq!(
            batch.rejected,
            Vec::from_iter(expected.map(|error| (hash, error)))
        );
        assert_eq!(provider.gets, 1);
    }
}

#[test]
fn a_required_object_with_bad_bytes_stays_pending_instead_of_rejected() {
    let (state, _) = fixture();
    let join = Machine::join(
        state.space,
        state.namespace,
        state.genesis,
        state.owner_root,
    );
    let mut provider = Memory {
        objects: BTreeMap::from([(state.genesis, b"corrupted".to_vec())]),
        ..Memory::default()
    };
    let batch = Discovery::for_state(&join)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert!(batch.incomplete);
    assert!(batch.rejected.is_empty());
}

#[test]
fn omitted_unavailable_anchor_is_incomplete_and_missing_gets_are_bounded() {
    let (state, _) = fixture();
    let join = Machine::join(
        state.space,
        state.namespace,
        state.genesis,
        state.owner_root,
    );
    let mut provider = Memory::default();
    assert!(
        Discovery::for_state(&join)
            .unwrap()
            .download(&mut provider, &Cancellation::default())
            .unwrap()
            .incomplete
    );
    assert_eq!(provider.gets, 1);
    provider.gets = 0;
    provider.missing = true;

    for index in 0u64..1000 {
        let hash = Hash::digest(b"missing-test", &index.to_be_bytes());
        provider.objects.insert(hash, vec![]);
    }

    let batch = download(
        &mut provider,
        state.namespace,
        &BTreeSet::new(),
        &Cancellation::default(),
    )
    .unwrap();
    assert!(batch.incomplete);
    assert_eq!(provider.gets, 256);
    assert!(batch.objects.is_empty());
}

fn fixture() -> (SyncState, DeviceSecret) {
    let (device, certificate) = DeviceSecret::generate(&SigningKey::from_bytes(&[1; 32])).unwrap();
    (
        Machine::create(
            Hash([1; 32]),
            Hash([2; 32]),
            certificate,
            &device,
            &[3; 32],
            Hash([4; 32]),
        )
        .unwrap(),
        device,
    )
}

#[test]
fn lost_upload_response_retries_exact_same_object_and_deduplicates() {
    let (state, device) = fixture();
    let replay = Machine::replay(&state, &device).unwrap();
    let plan = publication(&state, &replay).unwrap();
    let mut provider = Memory {
        lost_response: true,
        ..Memory::default()
    };

    let cancel = Cancellation::default();
    assert_eq!(
        upload(&mut provider, state.namespace, &plan, &cancel),
        Err(TransportError::Unavailable)
    );
    assert_eq!(provider.objects.len(), 1);
    assert_eq!(
        upload(&mut provider, state.namespace, &plan, &cancel).unwrap(),
        vec![state.genesis]
    );
    assert_eq!(
        provider.objects[&state.genesis],
        state.objects[&state.genesis].0
    );
    assert_eq!(
        download(
            &mut provider,
            state.namespace,
            &state.objects.keys().copied().collect(),
            &cancel
        )
        .unwrap()
        .objects
        .len(),
        0
    );
}

#[test]
fn listing_omission_keeps_known_history_and_partial_visibility_is_pending() {
    let (state, _) = fixture();
    let cancel = Cancellation::default();
    let mut provider = Memory::default();
    let result = download(
        &mut provider,
        state.namespace,
        &state.objects.keys().copied().collect(),
        &cancel,
    )
    .unwrap();
    assert!(result.objects.is_empty());
    assert_eq!(state.objects.len(), 1);
    provider
        .objects
        .insert(state.genesis, state.objects[&state.genesis].0.clone());
    provider.missing = true;
    assert!(
        download(&mut provider, state.namespace, &BTreeSet::new(), &cancel)
            .unwrap()
            .incomplete
    );
}

#[test]
fn repeated_cursor_and_cancelled_session_stop_exchange() {
    let (state, _) = fixture();
    let cancel = Cancellation::default();
    let mut provider = Memory {
        repeated_cursor: true,
        ..Memory::default()
    };
    assert!(matches!(
        download(&mut provider, state.namespace, &BTreeSet::new(), &cancel),
        Err(TransportError::InvalidResponse)
    ));
    cancel.cancel();
    assert!(matches!(
        download(&mut provider, state.namespace, &BTreeSet::new(), &cancel),
        Err(TransportError::Cancelled)
    ));
}

#[test]
fn publication_orders_epoch_and_causal_dependencies_before_references() {
    use crate::records::entry::Entry;
    use crate::sync::protocol::Role;
    use crate::sync::protocol::Transaction;

    let (mut state, device) = fixture();
    let genesis = state.genesis;
    let mut entry = Entry::new();
    entry.title = "Before rotation".into();
    state.intent = Some(
        Machine::prepare(
            &state,
            &device,
            Transaction::diff(
                &Machine::replay(&state, &device).unwrap().projection,
                &[entry.clone()],
                &[],
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let first = Machine::complete_intent(&mut state, &device).unwrap();
    let members = Machine::members(&state, &device).unwrap();
    assert_eq!(members[0].1, Role::Owner);
    state.intent = Some(Machine::prepare_membership(&state, &device, members).unwrap());
    let control = Machine::complete_intent(&mut state, &device).unwrap();
    entry.title = "After rotation".into();
    state.intent = Some(
        Machine::prepare(
            &state,
            &device,
            Transaction::diff(
                &Machine::replay(&state, &device).unwrap().projection,
                &[entry],
                &[],
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let last = Machine::complete_intent(&mut state, &device).unwrap();
    let mut replay = Machine::replay(&state, &device).unwrap();
    let plan = publication(&state, &replay).unwrap();
    let hashes: Vec<_> = plan.iter().map(|(hash, _)| *hash).collect();
    assert_eq!(hashes, vec![genesis, first, control, last]);
    for (hash, bytes) in &plan {
        assert_eq!(bytes, &state.objects[hash]);
    }

    // Acknowledged dependencies stay in the journal but leave this upload plan.
    state.outbox.remove(&genesis);
    state.outbox.remove(&first);
    assert_eq!(publication(&state, &replay).unwrap(), plan[2..]);

    replay.excluded.insert(last);
    assert_eq!(publication(&state, &replay).unwrap(), plan[2..3]);
}

#[test]
fn publication_counts_repeated_causal_dependencies_without_stalling() {
    let (_, state, device, last) = history();
    let replay = Machine::replay(&state, &device).unwrap();
    let plan = publication(&state, &replay).unwrap();
    assert_eq!(plan.len(), 4);
    assert_eq!(plan[0].0, state.genesis);
    assert_eq!(plan[3].0, last);
    for index in 2..4 {
        let object: crate::sync::protocol::Object =
            crate::sync::protocol::decode(&plan[index].1.0).unwrap();
        assert_eq!(
            object.dependencies().collect::<Vec<_>>(),
            vec![state.genesis, plan[index - 1].0, plan[index - 1].0]
        );
    }
}

#[test]
fn publication_checks_unselected_outbox_items_after_readiness_guards() {
    let (mut state, device) = fixture();
    let mut replay = Machine::replay(&state, &device).unwrap();
    let missing = Hash([255; 32]);
    assert!(!replay.accepted.contains(&missing));
    state.outbox.insert(missing);
    assert_eq!(
        publication(&state, &replay),
        Err(TransportError::InvalidResponse)
    );

    state.fault = Some(crate::sync::protocol::Error::InvalidTransaction);
    assert!(publication(&state, &replay).unwrap().is_empty());
    state.fault = None;
    replay.ready = false;
    assert!(publication(&state, &replay).unwrap().is_empty());
    replay.ready = true;
    replay.role = None;
    assert!(publication(&state, &replay).unwrap().is_empty());
}

/// Junk under a content address says nothing about the address itself. Once
/// the honest bytes are back under that key, the replica must accept them.
#[test]
fn junk_under_a_listed_key_does_not_poison_its_hash() {
    let (base, state, _, head) = history();
    let honest = state.objects[&head].0.clone();
    let mut provider = Memory {
        objects: state
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
        ..Memory::default()
    };
    let mut receiver = base;
    let pass = |provider: &mut Memory, receiver: &mut SyncState| {
        let batch = Discovery::for_state(receiver)
            .unwrap()
            .download(provider, &Cancellation::default())
            .unwrap();
        for (hash, error) in batch.rejected {
            Machine::reject_download(receiver, hash, error);
        }
        for (hash, bytes) in batch.objects {
            Machine::receive(receiver, hash, bytes).unwrap();
        }
    };

    for junk in [b"junk".to_vec(), state.objects[&state.genesis].0.clone()] {
        provider.objects.insert(head, junk);
        pass(&mut provider, &mut receiver);
        assert!(!receiver.rejected_candidates.contains_key(&head));
    }

    provider.objects.insert(head, honest);
    pass(&mut provider, &mut receiver);
    assert!(
        receiver.objects.contains_key(&head) || receiver.candidates.contains_key(&head),
        "restored honest bytes were refused"
    );
}

fn pass_result(objects: &[Hash], bad_copies: &[Hash]) -> Download {
    Download {
        objects: objects.iter().map(|hash| (*hash, Vec::new())).collect(),
        incomplete: false,
        rejected: Vec::new(),
        bad_copies: bad_copies.to_vec(),
    }
}

/// Junk under a key waits 2, 4, 8 … passes, is retried in between and forgotten
/// as soon as honest bytes arrive. The map stays bounded under a flood.
#[test]
fn bad_copies_back_off_exponentially_and_clear_on_honest_bytes() {
    let junk = Hash([9; 32]);
    let mut backoff = BadCopies::default();
    let mut fetched_at = Vec::new();
    for pass in 0..40u64 {
        let skipped = backoff.skipped().any(|hash| hash == junk);
        let bad: &[Hash] = if skipped { &[] } else { &[junk] };
        if !skipped {
            fetched_at.push(pass);
        }
        backoff.record(&pass_result(&[], bad));
    }
    assert_eq!(fetched_at, vec![0, 3, 8, 17, 34]);

    backoff.record(&pass_result(&[junk], &[]));
    assert_eq!(backoff.skipped().count(), 0, "honest bytes must clear it");

    for _ in 0..2000u32 {
        backoff.record(&pass_result(&[], &[junk]));
    }
    assert!(backoff.skipped().any(|hash| hash == junk));
    let mut flood = BadCopies::default();
    let keys: Vec<_> = (0..3000u32)
        .map(|index| {
            let mut key = [0xB0; 32];
            key[..4].copy_from_slice(&index.to_be_bytes());
            Hash(key)
        })
        .collect();
    flood.record(&pass_result(&[], &keys));
    assert!(flood.skipped().count() <= 1024);
}

/// A skipped key costs no GET, but a required object is fetched regardless.
#[test]
fn skipped_keys_are_not_fetched_unless_required() {
    let (state, _) = fixture();
    let join = Machine::join(
        state.space,
        state.namespace,
        state.genesis,
        state.owner_root,
    );
    let junk = Hash([9; 32]);
    let mut provider = Memory {
        objects: BTreeMap::from([
            (junk, b"junk".to_vec()),
            (state.genesis, state.objects[&state.genesis].0.clone()),
        ]),
        ..Memory::default()
    };
    let batch = Discovery::for_state(&join)
        .unwrap()
        .skipping([junk, state.genesis])
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert_eq!(provider.gets, 1);
    assert_eq!(batch.objects.len(), 1);
    assert!(batch.bad_copies.is_empty());
}

/// A profile poisoned by an older version heals on load: rejections that only a
/// bad download could cause are dropped, authority rejections are kept, and the
/// honest object is received on the next pass.
#[test]
fn loading_a_poisoned_profile_forgets_download_rejections_only() {
    use crate::sync::protocol::Error;
    let (base, state, device, head) = history();
    let mut poisoned = base;
    let unauthorized = Hash([7; 32]);
    Machine::reject_download(&mut poisoned, head, Error::InvalidEncoding);
    Machine::reject_download(&mut poisoned, Hash([5; 32]), Error::HashMismatch);
    Machine::reject_download(&mut poisoned, Hash([6; 32]), Error::ResourceBudgetExceeded);
    Machine::reject_download(&mut poisoned, unauthorized, Error::UnauthorizedDevice);

    let saved = serde_json::to_vec(&poisoned).unwrap();
    let mut loaded: SyncState = serde_json::from_slice(&saved).unwrap();
    assert_eq!(
        loaded.rejected_candidates,
        BTreeMap::from([(unauthorized, Error::UnauthorizedDevice)])
    );
    assert!(Machine::replay(&loaded, &device).unwrap().ready);

    let mut provider = Memory {
        objects: state
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
        ..Memory::default()
    };
    let batch = Discovery::for_state(&loaded)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    for (hash, bytes) in batch.objects {
        Machine::receive(&mut loaded, hash, bytes).unwrap();
    }
    assert!(loaded.objects.contains_key(&head));
    let replay = Machine::replay(&loaded, &device).unwrap();
    assert!(replay.ready);
    assert_eq!(replay.projection.entries()[0].title, "Third");
}

/// Attack: a bucket writer stores one object the gateway answers with an
/// unreadable header or an odd 4xx. That key alone is unavailable this pass:
/// honest heads still arrive and the pass stays complete. A required object
/// stays pending, and provider-wide failures still abort the pass.
#[test]
fn one_unreadable_object_does_not_abort_discovery() {
    let (base, state, _, head) = history();
    let junk = Hash([9; 32]);
    let honest = || Memory {
        objects: state
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .chain([(junk, b"junk".to_vec())])
            .collect(),
        errors: BTreeMap::from([(junk, TransportError::InvalidResponse)]),
        ..Memory::default()
    };

    let mut provider = honest();
    let batch = Discovery::for_state(&base)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert!(batch.objects.iter().any(|(hash, _)| *hash == head));
    assert!(
        !batch.incomplete,
        "an unreadable listed key stalled the pass"
    );
    assert!(
        batch.rejected.is_empty(),
        "the hash itself was never judged"
    );
    assert_eq!(batch.bad_copies, vec![junk]);

    let join = Machine::join(
        state.space,
        state.namespace,
        state.genesis,
        state.owner_root,
    );
    let mut provider = honest();
    provider
        .errors
        .insert(state.genesis, TransportError::InvalidResponse);
    let batch = Discovery::for_state(&join)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert!(batch.incomplete, "a required object must stay pending");
    assert!(!batch.bad_copies.contains(&state.genesis));

    for error in [TransportError::Unavailable, TransportError::AccessDenied] {
        let mut provider = honest();
        provider.errors.insert(junk, error);
        assert_eq!(
            Discovery::for_state(&base)
                .unwrap()
                .download(&mut provider, &Cancellation::default())
                .err(),
            Some(error)
        );
    }
}

/// A canonical, correctly hashed change that names an unknown membership and
/// `MAX_PARENTS` invented parents. It stays a candidate forever.
fn junk_candidate(template: &[u8], seed: u8) -> (Hash, Vec<u8>) {
    use crate::sync::protocol::MAX_PARENTS;
    let fake = |index: usize| {
        let mut bytes = [seed; 32];
        bytes[..8].copy_from_slice(&(index as u64).to_be_bytes());
        Hash(bytes).to_string()
    };
    let mut value: serde_json::Value = serde_json::from_slice(template).unwrap();
    let header = &mut value["object"]["body"]["header"];
    header["membership"] = fake(usize::MAX).into();
    header["previous"] = fake(usize::MAX - 1).into();
    let mut parents: Vec<_> = (0..MAX_PARENTS).map(fake).collect();
    parents.sort();
    header["parents"] = parents.into();
    let bytes = crate::sync::protocol::canonical(&value).unwrap();
    let hash = Hash::digest(b"lockbox.sync.object\0", &bytes);
    crate::sync::protocol::Object::decode(&bytes, hash).unwrap();
    (hash, bytes)
}

/// Attack: a bucket writer plants a few junk candidates whose invented
/// dependencies used to exhaust the pass budget before LIST. Listing must still
/// run, so the replica keeps seeing new heads from other members.
#[test]
fn junk_candidate_hints_cannot_starve_the_listing() {
    let (base, state, _, head) = history();
    let mut receiver = base;
    let template = &state.objects[&head].0;
    for seed in 1..=4u8 {
        let (hash, bytes) = junk_candidate(template, seed);
        receiver
            .candidates
            .insert(hash, crate::sync::protocol::Blob(bytes));
    }
    let mut provider = Memory {
        objects: state
            .objects
            .iter()
            .map(|(hash, bytes)| (*hash, bytes.0.clone()))
            .collect(),
        ..Memory::default()
    };

    let batch = Discovery::for_state(&receiver)
        .unwrap()
        .download(&mut provider, &Cancellation::default())
        .unwrap();
    assert!(
        batch.objects.iter().any(|(hash, _)| *hash == head),
        "the listed head was never downloaded"
    );
    assert!(
        !batch.incomplete,
        "unresolvable hints must not stall the pass"
    );
}
