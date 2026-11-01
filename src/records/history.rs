//! Encrypted record revisions. Local authorship is advisory; cloud authors are verified signers.
use crate::records::entry::Entry;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct RecordChange {
    pub id: String,
    pub revision: String,
    pub actor: String,
    pub device: String,
    pub at: Option<u64>,
    pub action: String,
    pub before: Vec<Entry>,
    pub after: Option<Entry>,
}
