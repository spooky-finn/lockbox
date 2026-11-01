//! Authenticated control metadata, independent of content keys and projections.

use crate::sync::protocol::{
    Error, Hash, Object, Result, Signed,
    machine::{membership::validate_control, protocol::Control, state::SyncState},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct ControlHistory<'a> {
    pub(super) chain: Vec<(Hash, &'a Signed<Control>)>,
    pub(super) rejected: BTreeMap<Hash, Error>,
}

impl<'a> ControlHistory<'a> {
    pub(super) fn verify(
        controls: &'a BTreeMap<Hash, Signed<Control>>,
        state: &SyncState,
    ) -> Result<Self> {
        let genesis = controls
            .get(&state.genesis)
            .ok_or(Error::MissingDependency)?;
        validate_control(genesis, None, state, &[])?;
        let mut history = Self {
            chain: vec![(state.genesis, genesis)],
            rejected: BTreeMap::new(),
        };
        let mut successors = BTreeMap::<_, Vec<_>>::new();
        for (hash, control) in controls {
            if let Some(previous) = control.body.epoch.previous {
                successors
                    .entry(previous)
                    .or_default()
                    .push((*hash, control));
            }
        }

        let mut seen = BTreeSet::from([state.genesis]);
        loop {
            let (head, previous) = history.tip();
            let mut next = None;

            for &(hash, candidate) in successors.get(&head).into_iter().flatten() {
                match validate_control(candidate, Some(previous), state, &history.chain) {
                    Ok(()) if next.is_some() => return Err(Error::ControlFork),
                    Ok(()) => next = Some((hash, candidate)),
                    Err(error) => {
                        history.rejected.insert(hash, error);
                    }
                }
            }

            let Some((hash, control)) = next else {
                break;
            };

            if !seen.insert(hash) {
                return Err(Error::ControlFork);
            }
            history.chain.push((hash, control));
        }

        Ok(history)
    }

    pub(super) fn tip(&self) -> (Hash, &'a Signed<Control>) {
        *self
            .chain
            .last()
            .expect("verified history contains pinned genesis")
    }

    pub(super) fn contains(&self, hash: Hash) -> bool {
        self.chain.iter().any(|(known, _)| *known == hash)
    }
}

pub(super) fn with_history<T>(
    state: &SyncState,
    visit: impl FnOnce(&ControlHistory<'_>) -> Result<T>,
) -> Result<T> {
    if state.objects.len() > crate::sync::protocol::MAX_OBJECTS {
        return Err(Error::ResourceBudgetExceeded);
    }

    let controls = controls(state)?;
    visit(&ControlHistory::verify(&controls, state)?)
}

pub(super) fn controls(state: &SyncState) -> Result<BTreeMap<Hash, Signed<Control>>> {
    let mut controls = BTreeMap::new();
    for (hash, bytes) in &state.objects {
        let object = Object::decode(&bytes.0, *hash)?;
        if let Object::Control(control) = object {
            controls.insert(*hash, control);
        }
    }
    Ok(controls)
}
