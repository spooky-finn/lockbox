//! Verify public signatures and causal references before relaying closed
//! ciphertext for a new device that has no historical content keys. This proves
//! ciphertext provenance, not historical plaintext correctness.

use super::{
    Machine,
    history::with_history,
    profile::Profile,
    protocol::{Change, Control},
    state::SyncState,
};
use crate::core::sync::{
    Error, Hash, Object, Result, Role, Signed, crypto, decode, wire::sorted_unique,
};
use std::collections::{BTreeMap, BTreeSet};

impl Machine {
    pub fn relayable_closed_history(state: &SyncState) -> Result<BTreeSet<Hash>> {
        with_history(state, |history| {
            let mut result: BTreeSet<_> = history.chain.iter().map(|(hash, _)| *hash).collect();
            for pair in history.chain.windows(2) {
                let (membership, previous) = pair[0];
                let frontier = &pair[1].1.body.closed_frontier;
                let nodes = closed_nodes(state, membership, &previous.body, frontier)?;
                let mut slots = BTreeMap::new();
                for (hash, node) in &nodes {
                    let header = &node.body.header;
                    if slots
                        .insert((header.author, header.sequence), *hash)
                        .is_some()
                    {
                        return Err(Error::DeviceFork);
                    }
                    let past = ancestors(&header.parents, &nodes)?;
                    match (header.sequence.0, header.previous) {
                        (1, None) => {}
                        (sequence, Some(previous)) if sequence > 1 && past.contains(&previous) => {
                            let prior = &nodes[&previous].body.header;
                            if prior.author != header.author
                                || prior.sequence.0.checked_add(1) != Some(sequence)
                            {
                                return Err(Error::InvalidTransaction);
                            }
                        }
                        _ => return Err(Error::InvalidTransaction),
                    }
                    antichain(&header.parents, &nodes)?;
                }
                antichain(frontier, &nodes)?;
                result.extend(nodes.keys());
            }
            Ok(result)
        })
    }
}

fn closed_nodes(
    state: &SyncState,
    membership: Hash,
    control: &Control,
    frontier: &[Hash],
) -> Result<BTreeMap<Hash, Signed<Change>>> {
    let mut pending = frontier.to_vec();
    let mut nodes = BTreeMap::new();
    while let Some(hash) = pending.pop() {
        if nodes.contains_key(&hash) {
            continue;
        }
        if nodes.len() >= crate::core::sync::MAX_OBJECTS {
            return Err(Error::ResourceBudgetExceeded);
        }
        let Object::Change(change) =
            decode::<Object>(&state.objects.get(&hash).ok_or(Error::MissingDependency)?.0)?
        else {
            return Err(Error::InvalidTransaction);
        };
        let h = &change.body.header;
        if h.membership != membership
            || h.space != state.space
            || h.epoch != control.epoch.number
            || h.profile != control.epoch.profile
            || h.sequence.0 == 0
            || h.nonce.0.len() != 24
            || h.parents.len() > crate::core::sync::MAX_PARENTS
            || !sorted_unique(&h.parents)
            || change.body.ciphertext.0.len() < 16
        {
            return Err(Error::InvalidTransaction);
        }
        let member = control
            .members
            .iter()
            .find(|m| m.certificate.body.device == h.author && m.role != Role::Reader)
            .ok_or(Error::UnauthorizedDevice)?;
        crypto::verify(
            &change,
            &member.certificate.body.signing_key,
            Profile::parse(&h.profile)?.change_domain(),
        )?;
        pending.extend(h.parents.iter().copied());
        nodes.insert(hash, change);
    }
    Ok(nodes)
}

fn ancestors(parents: &[Hash], nodes: &BTreeMap<Hash, Signed<Change>>) -> Result<BTreeSet<Hash>> {
    let mut pending = parents.to_vec();
    let mut found = BTreeSet::new();
    while let Some(hash) = pending.pop() {
        if !found.insert(hash) {
            continue;
        }
        if found.len() > crate::core::sync::MAX_OBJECTS {
            return Err(Error::ResourceBudgetExceeded);
        }
        pending.extend(
            nodes
                .get(&hash)
                .ok_or(Error::MissingDependency)?
                .body
                .header
                .parents
                .iter()
                .copied(),
        );
    }
    Ok(found)
}

fn antichain(heads: &[Hash], nodes: &BTreeMap<Hash, Signed<Change>>) -> Result<()> {
    for hash in heads {
        let past = ancestors(
            &nodes
                .get(hash)
                .ok_or(Error::MissingDependency)?
                .body
                .header
                .parents,
            nodes,
        )?;
        if heads.iter().any(|other| past.contains(other)) {
            return Err(Error::InvalidTransaction);
        }
    }
    Ok(())
}
