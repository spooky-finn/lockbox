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
    let hashes: Vec<_> = publication(&state, &Machine::replay(&state, &device).unwrap())
        .unwrap()
        .into_iter()
        .map(|(hash, _)| hash)
        .collect();
    assert_eq!(hashes, vec![genesis, first, control, last]);
}
