//! Behavioral coverage for the sync facade.

use super::{Machine, PreparedChange, SyncState};
use super::{
    commands::control_at,
    membership::epoch_key,
    protocol::{Change, ChangeHeader},
};
use crate::core::sync::{
    Blob, Certificate, Counter, DeviceSecret, Error, Hash, Object, Projection, Role, Signed,
    Transaction, canonical,
    crypto::{self, CHANGE_DOMAIN, CONTROL_DOMAIN, nonce},
    decode,
};
use crate::core::{
    entry::Entry,
    sync::{Action, Entity, EntityKind, Mutation, Value},
};
use ed25519_dalek::SigningKey;
use std::collections::BTreeSet;

mod support;
use support::*;

mod authority;
mod intents;
mod invitations;
mod membership;
mod replay;
mod wire;
