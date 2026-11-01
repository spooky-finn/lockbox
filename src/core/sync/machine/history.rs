//! Authenticated control metadata, independent of content keys and projections.

use super::{membership::validate_control, protocol::Control, state::SyncState};
use crate::core::sync::{Error, Hash, Result, Signed};
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
        validate_control(genesis, None, state)?;
        let mut history = Self {
            chain: vec![(state.genesis, genesis)],
            rejected: BTreeMap::new(),
        };
        let mut seen = BTreeSet::from([state.genesis]);
        loop {
            let (head, previous) = history.tip();
            let mut next = None;
            for (hash, candidate) in controls {
                if candidate.body.epoch.previous != Some(head) {
                    continue;
                }
                match validate_control(candidate, Some(previous), state) {
                    Ok(()) if next.is_some() => return Err(Error::ControlFork),
                    Ok(()) => next = Some((*hash, candidate)),
                    Err(error) => {
                        history.rejected.insert(*hash, error);
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
