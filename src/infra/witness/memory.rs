//! Explicit nonpersistent test adapter. Dropping the store loses its witnesses.

use super::{Error, Hash, Result, Witness, WitnessLease, WitnessStore};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Slot {
    leased: bool,
    witness: Option<Witness>,
}

#[derive(Default)]
struct State {
    slots: BTreeMap<Hash, Slot>,
    #[cfg(test)]
    fail_reads: bool,
    #[cfg(test)]
    fail_writes: usize,
    #[cfg(test)]
    skip_writes: usize,
    #[cfg(test)]
    lose_writes: usize,
}

/// Nonpersistent witnesses for examples and injected test hosts. Never substitute
/// this adapter for the system store in a real installation.
#[derive(Clone, Default)]
pub struct MemoryWitness(Arc<Mutex<State>>);

impl WitnessStore for MemoryWitness {
    fn acquire(&self, device: Hash) -> Result<Box<dyn WitnessLease>> {
        let mut state = self.0.lock().map_err(|_| Error::Unavailable)?;
        let slot = state.slots.entry(device).or_default();

        if slot.leased {
            return Err(Error::InUse);
        }
        slot.leased = true;
        Ok(Box::new(Lease {
            store: self.clone(),
            device,
        }))
    }
}

struct Lease {
    store: MemoryWitness,
    device: Hash,
}

impl WitnessLease for Lease {
    fn load(&self) -> Result<Option<Witness>> {
        let state = self.store.0.lock().map_err(|_| Error::Unavailable)?;
        #[cfg(test)]
        if state.fail_reads {
            return Err(Error::Unavailable);
        }

        Ok(state.slots.get(&self.device).and_then(|slot| slot.witness))
    }

    fn replace(&mut self, expected: Option<Witness>, next: Witness) -> Result<()> {
        let mut state = self.store.0.lock().map_err(|_| Error::Unavailable)?;
        #[cfg(test)]
        if state.skip_writes > 0 {
            state.skip_writes -= 1;
        } else if state.fail_writes > 0 {
            state.fail_writes -= 1;
            return Err(Error::Unavailable);
        }

        let slot = state
            .slots
            .get_mut(&self.device)
            .ok_or(Error::Unavailable)?;

        if slot.witness != expected {
            return Err(Error::Rollback);
        }
        slot.witness = Some(next);
        #[cfg(test)]
        if state.lose_writes > 0 {
            state.lose_writes -= 1;
            return Err(Error::Unavailable);
        }

        Ok(())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.store.0.lock()
            && let Some(slot) = state.slots.get_mut(&self.device)
        {
            slot.leased = false;
        }
    }
}

#[cfg(test)]
impl MemoryWitness {
    pub(crate) fn fail_writes(&self, count: usize) {
        self.0.lock().unwrap().fail_writes = count;
    }
    pub(crate) fn fail_writes_after(&self, skip: usize, count: usize) {
        let mut state = self.0.lock().unwrap();
        state.skip_writes = skip;
        state.fail_writes = count;
    }
    pub(crate) fn fail_reads(&self, fail: bool) {
        self.0.lock().unwrap().fail_reads = fail;
    }
    pub(crate) fn lose_writes(&self, count: usize) {
        self.0.lock().unwrap().lose_writes = count;
    }
    pub(crate) fn snapshot(&self, device: Hash) -> Option<Witness> {
        self.0
            .lock()
            .unwrap()
            .slots
            .get(&device)
            .and_then(|slot| slot.witness)
    }
    pub(crate) fn restore(&self, device: Hash, witness: Option<Witness>) {
        self.0
            .lock()
            .unwrap()
            .slots
            .entry(device)
            .or_default()
            .witness = witness;
    }
}
