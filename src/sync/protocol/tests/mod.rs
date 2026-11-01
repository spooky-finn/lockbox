//! Behavioral coverage for the sync facade.

use crate::{
    records::entry::Entry,
    sync::protocol::{
        Action, Blob, Certificate, Counter, DeviceSecret, Entity, EntityKind, Error, Hash,
        Mutation, Object, Projection, Role, Signed, Transaction, Value, canonical,
        crypto::{self, CHANGE_DOMAIN, CONTROL_DOMAIN, nonce},
        decode,
        machine::{
            Machine, PreparedChange, SyncState,
            commands::control_at,
            membership::epoch_key,
            protocol::{Change, ChangeHeader},
        },
    },
};
use ed25519_dalek::SigningKey;
use std::collections::BTreeSet;

mod support;
use support::*;

mod authority;
mod candidates;
mod history;
mod intents;
mod invitations;
mod membership;
mod replay;
mod wire;
