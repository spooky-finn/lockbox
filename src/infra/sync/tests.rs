use super::*;
use crate::core::sync::{DeviceSecret, Machine};
use ed25519_dalek::SigningKey;
use std::collections::BTreeMap;

#[derive(Default)]
struct Memory {
    objects: BTreeMap<Hash, Vec<u8>>,
    lost_response: bool,
    repeated_cursor: bool,
    missing: bool,
}

impl Storage for Memory {
    fn list(&mut self, _: Hash, _: Option<&str>) -> Result<Page> {
        Ok(Page {
            objects: self.objects.keys().rev().copied().collect(),
            next: self.repeated_cursor.then(|| "same".into()),
        })
    }
    fn get(&mut self, _: Hash, hash: Hash) -> Result<Vec<u8>> {
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
