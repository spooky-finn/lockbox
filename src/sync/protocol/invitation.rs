//! Public addressed invitations and recipient consent. No provider credentials,
//! content keys or private identity material belong in these artifacts.

use crate::sync::protocol::{
    Blob, Certificate, Counter, Error, Hash, Result, Role, Signed, SpaceDescriptor, canonical, wire,
};
use serde::{Deserialize, Serialize};

pub const MAX_INVITATION_BYTES: usize = 96 * 1024 * 1024;
pub(super) const OFFER_DOMAIN: &[u8] = b"lockbox.sync.invite\0";
pub(super) const JOIN_DOMAIN: &[u8] = b"lockbox.sync.join\0";
pub(super) const RECEIPT_DOMAIN: &[u8] = b"lockbox.sync.admission-receipt\0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteOffer {
    pub(crate) descriptor: SpaceDescriptor,
    /// Controller-signed display name and S3 routing segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) space_name: Option<String>,
    pub(crate) anchor: Hash,
    pub(crate) id: Hash,
    pub(crate) author: Hash,
    pub(crate) recipient: Signed<Certificate>,
    pub(crate) role: Role,
    /// Owner-side expiry policy only. Replica authority never depends on a clock.
    pub(crate) expires_at: Option<Counter>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    pub(crate) offer: Hash,
    pub(crate) recipient: Hash,
    pub(crate) challenge: Hash,
}

/// Complete signed consent embedded in the membership transition that admits it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admission {
    pub(crate) offer: Signed<InviteOffer>,
    pub(crate) request: Signed<JoinRequest>,
}

/// Public metadata proof permits verification before shared provider access.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub(crate) offer: Signed<InviteOffer>,
    pub(crate) history: Vec<Blob>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionConfirmation {
    pub(crate) descriptor: SpaceDescriptor,
    pub(crate) invitation: Hash,
    pub(crate) acceptance: Hash,
    pub(crate) control: Hash,
    pub(crate) author: Hash,
}

/// Controller-signed current head prevents unsigned truncation of the proof.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionReceipt {
    pub(crate) confirmation: Signed<AdmissionConfirmation>,
    pub(crate) history: Vec<Blob>,
}

impl AdmissionReceipt {
    pub fn descriptor(&self) -> &SpaceDescriptor {
        &self.confirmation.body.descriptor
    }

    pub fn id(&self) -> Hash {
        self.confirmation.body.invitation
    }
    pub fn control(&self) -> Hash {
        self.confirmation.body.control
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        artifact_bytes(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        wire::decode_bounded(bytes, MAX_INVITATION_BYTES)
    }
}

impl Invitation {
    /// Compare through an independent channel. Includes the entire descriptor
    /// and signed control anchor, not only a displayed owner name or root key.
    pub fn verification_code(&self) -> Result<Hash> {
        let metadata = match self.space_name() {
            Some(name) => canonical(&(self.descriptor(), self.anchor(), name))?,
            None => canonical(&(self.descriptor(), self.anchor()))?,
        };
        Ok(Hash::digest(
            b"lockbox.sync.invitation-verification\0",
            &metadata,
        ))
    }

    pub fn id(&self) -> Hash {
        self.offer.body.id
    }
    pub fn descriptor(&self) -> &SpaceDescriptor {
        &self.offer.body.descriptor
    }
    pub fn space_name(&self) -> Option<&str> {
        self.offer.body.space_name.as_deref()
    }
    pub fn anchor(&self) -> Hash {
        self.offer.body.anchor
    }
    pub fn recipient(&self) -> &Signed<Certificate> {
        &self.offer.body.recipient
    }
    pub fn role(&self) -> Role {
        self.offer.body.role
    }
    pub fn expires_at(&self) -> Option<u64> {
        self.offer.body.expires_at.map(|counter| counter.0)
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        artifact_bytes(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        wire::decode_bounded(bytes, MAX_INVITATION_BYTES)
    }
}

impl Admission {
    /// Verifies the recipient's exact signed consent. Controller authority and
    /// single use additionally require the authenticated control history.
    pub fn verify_consent(&self) -> Result<()> {
        self.recipient().verify()?;
        let request = &self.request.body;
        if request.offer != self.offer_hash()? || request.recipient != self.recipient().body.device
        {
            return Err(Error::InvalidInvitation);
        }
        crate::sync::protocol::crypto::verify(
            &self.request,
            &self.recipient().body.signing_key,
            JOIN_DOMAIN,
        )
    }

    pub fn id(&self) -> Hash {
        self.offer.body.id
    }
    pub fn descriptor(&self) -> &SpaceDescriptor {
        &self.offer.body.descriptor
    }
    pub fn recipient(&self) -> &Signed<Certificate> {
        &self.offer.body.recipient
    }
    pub fn role(&self) -> Role {
        self.offer.body.role
    }
    pub fn offer_hash(&self) -> Result<Hash> {
        offer_hash(&self.offer)
    }
    pub fn hash(&self) -> Result<Hash> {
        Ok(Hash::digest(
            b"lockbox.sync.acceptance-object\0",
            &canonical(self)?,
        ))
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        artifact_bytes(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        wire::decode(bytes)
    }
}

pub(super) fn offer_hash(offer: &Signed<InviteOffer>) -> Result<Hash> {
    Ok(Hash::digest(
        b"lockbox.sync.invite-object\0",
        &canonical(offer)?,
    ))
}

pub(super) fn certificate_hash(certificate: &Signed<Certificate>) -> Result<Hash> {
    certificate.hash()
}

fn artifact_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = canonical(value)?;

    if bytes.len() > MAX_INVITATION_BYTES {
        return Err(Error::ResourceBudgetExceeded);
    }
    Ok(bytes)
}
