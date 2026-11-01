//! Bounded, deterministic parent selection for projection-preserving merge events.

use super::state::{Replay, SyncState};
use crate::core::sync::{Error, Hash, MAX_PARENTS, Object, Result, decode};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn fold_parents(
    state: &SyncState,
    replay: &Replay,
    previous: Option<Hash>,
) -> Result<Vec<Hash>> {
    let mut parents: BTreeSet<_> = replay.frontier.iter().take(MAX_PARENTS).copied().collect();
    if let Some(previous) = previous {
        // The next sequence must observe this device's last event even when its
        // head sorts outside the first bounded group. Pick one maximal descendant
        // of it so the selected parents remain a causal antichain.
        let mut children: BTreeMap<Hash, Vec<Hash>> = BTreeMap::new();
        for hash in &replay.accepted {
            let Object::Change(change) = decode::<Object>(&state.objects[hash].0)? else {
                return Err(Error::InvalidTransaction);
            };
            for parent in change.body.header.parents {
                children.entry(parent).or_default().push(*hash);
            }
        }
        let mut descendants = BTreeSet::new();
        let mut stack = vec![previous];
        while let Some(hash) = stack.pop() {
            if descendants.insert(hash) {
                stack.extend(children.get(&hash).into_iter().flatten().copied());
            }
        }
        if parents.is_disjoint(&descendants) {
            let head = replay
                .frontier
                .intersection(&descendants)
                .next()
                .ok_or(Error::InvalidTransaction)?;
            parents.pop_last();
            parents.insert(*head);
        }
    }
    Ok(parents.into_iter().collect())
}
