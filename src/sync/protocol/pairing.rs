//! Link contact envelopes. Existing addressed consent remains the only admission path.

use super::{
    Blob, Certificate, Counter, DeviceSecret, Error, Hash, Result, Signed, SpaceDescriptor,
    canonical, crypto, wire,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const LINK_PREFIX: &str = "lockbox://invite/";
const SHORT_LINK_PREFIX: &str = "lockbox://i/";
pub(crate) const MAX_LINK_BYTES: usize = 16 * 1024;
pub(crate) const MAX_TICKET_BYTES: usize = MAX_LINK_BYTES / 2;
pub(crate) const MAX_CONTACT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_MESSAGE_BYTES: usize = 128 * 1024 * 1024 + 4096;
pub(super) const TICKET_DOMAIN: &[u8] = b"lockbox.pairing.ticket\0";
const REQUEST_DOMAIN: &[u8] = b"lockbox.pairing.request\0";
const STATUS_DOMAIN: &[u8] = b"lockbox.pairing.status\0";
const ACK_DOMAIN: &[u8] = b"lockbox.pairing.joined\0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TicketBody {
    pub id: Hash,
    pub descriptor: SpaceDescriptor,
    pub anchor: Hash,
    pub proof: Hash,
    pub controller: Signed<Certificate>,
    pub endpoint: String,
    pub bucket: String,
    pub space_name: String,
    pub expires_at: Counter,
}

pub(crate) type Ticket = Signed<TicketBody>;

/// Public locator. Its complete ticket hash pins the downloaded signed document;
/// storage credentials and the independent verification code remain separate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkReference {
    pub space_name: String,
    pub ticket: Hash,
}

impl LinkReference {
    pub fn is_short(link: &str) -> bool {
        link.trim().starts_with(SHORT_LINK_PREFIX)
    }

    pub fn validate(&self) -> bool {
        valid_name(&self.space_name)
    }

    pub fn link(&self) -> String {
        format!(
            "{SHORT_LINK_PREFIX}{}/{}",
            URL_SAFE_NO_PAD.encode(self.space_name.as_bytes()),
            URL_SAFE_NO_PAD.encode(self.ticket.0),
        )
    }

    pub fn parse(link: &str) -> Result<Self> {
        let link = link.trim();
        if link.len() > 512 {
            return Err(Error::ResourceBudgetExceeded);
        }
        let (space, ticket) = link
            .strip_prefix(SHORT_LINK_PREFIX)
            .and_then(|value| value.split_once('/'))
            .ok_or(Error::InvalidInvitation)?;
        let reference = Self {
            space_name: String::from_utf8(
                URL_SAFE_NO_PAD
                    .decode(space)
                    .map_err(|_| Error::InvalidEncoding)?,
            )
            .map_err(|_| Error::InvalidEncoding)?,
            ticket: Hash(
                URL_SAFE_NO_PAD
                    .decode(ticket)
                    .map_err(|_| Error::InvalidEncoding)?
                    .try_into()
                    .map_err(|_| Error::InvalidEncoding)?,
            ),
        };
        if !reference.validate() || reference.link() != link {
            return Err(Error::InvalidEncoding);
        }
        Ok(reference)
    }

    pub fn decode_ticket(&self, bytes: &[u8]) -> Result<Ticket> {
        let ticket: Ticket = wire::decode_bounded(bytes, MAX_TICKET_BYTES)?;
        if ticket.hash()? != self.ticket || ticket.body.space_name != self.space_name {
            return Err(Error::HashMismatch);
        }
        ticket.verify_ticket()?;
        Ok(ticket)
    }
}

impl Ticket {
    pub fn reference(&self) -> Result<LinkReference> {
        Ok(LinkReference {
            space_name: self.body.space_name.clone(),
            ticket: self.hash()?,
        })
    }

    pub fn hash(&self) -> Result<Hash> {
        Hash::digest_canonical(b"lockbox.pairing.ticket-object\0", self)
    }

    pub fn verification_code(&self) -> Result<Hash> {
        Hash::digest_canonical(b"lockbox.pairing.verify\0", self)
    }

    pub fn link(&self) -> Result<String> {
        let bytes = canonical(self)?;
        if bytes.len() > MAX_TICKET_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        Ok(format!("{LINK_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
    }

    pub fn parse_link(link: &str) -> Result<Self> {
        let link = link.trim();
        if link.len() > MAX_LINK_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        let encoded = link
            .strip_prefix(LINK_PREFIX)
            .ok_or(Error::InvalidInvitation)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| Error::InvalidEncoding)?;
        let ticket: Self = wire::decode_bounded(&bytes, MAX_LINK_BYTES)?;
        ticket.verify_ticket()?;
        if ticket.link()? != link {
            return Err(Error::InvalidEncoding);
        }
        Ok(ticket)
    }

    pub fn verify_ticket(&self) -> Result<()> {
        let body = &self.body;
        body.controller.verify()?;
        let endpoint = url::Url::parse(&body.endpoint).map_err(|_| Error::InvalidInvitation)?;
        let bucket = body.bucket.as_bytes();
        if body.controller.body.root_key != body.descriptor.owner_root
            || !valid_name(&body.space_name)
            || body.endpoint.len() > 2048
            || body
                .endpoint
                .chars()
                .any(|value| value.is_whitespace() || value.is_control())
            || endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.path() != "/"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !(3..=63).contains(&bucket.len())
            || !bucket.first().is_some_and(u8::is_ascii_alphanumeric)
            || !bucket.last().is_some_and(u8::is_ascii_alphanumeric)
            || !bucket
                .iter()
                .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || *value == b'-')
        {
            return Err(Error::InvalidInvitation);
        }
        crypto::verify(self, &body.controller.body.signing_key, TICKET_DOMAIN)
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

pub(crate) fn proof_hash(bytes: &[u8]) -> Hash {
    Hash::digest(b"lockbox.pairing.proof\0", bytes)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContactBody {
    pub ticket: Hash,
    pub username: String,
    pub certificate: Signed<Certificate>,
    pub challenge: Hash,
}

pub(crate) type Contact = Signed<ContactBody>;

impl Contact {
    pub fn create(
        ticket: &Ticket,
        username: &str,
        certificate: Signed<Certificate>,
        device: &DeviceSecret,
    ) -> Result<Self> {
        if !valid_name(username) || !device.matches_certificate(&certificate.body) {
            return Err(Error::InvalidInvitation);
        }
        crypto::sign(
            ContactBody {
                ticket: ticket.hash()?,
                username: username.trim().to_owned(),
                certificate,
                challenge: Hash(*crypto::random32()?),
            },
            &device.signer(),
            REQUEST_DOMAIN,
        )
    }

    pub fn hash(&self) -> Result<Hash> {
        Hash::digest_canonical(b"lockbox.pairing.request-object\0", self)
    }

    pub fn verify_contact(&self, ticket: &Ticket) -> Result<()> {
        if self.body.ticket != ticket.hash()?
            || !valid_name(&self.body.username)
            || canonical(self)?.len() > MAX_CONTACT_BYTES
        {
            return Err(Error::InvalidInvitation);
        }
        self.body.certificate.verify()?;
        crypto::verify(
            self,
            &self.body.certificate.body.signing_key,
            REQUEST_DOMAIN,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Request,
    Offer,
    Consent,
    Receipt,
    Joined,
    Rejected,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub ticket: Hash,
    pub request: Hash,
    pub kind: Kind,
    enc: Blob,
    ciphertext: Blob,
}

impl Envelope {
    pub fn seal<T: Serialize>(
        ticket: &Ticket,
        request: Hash,
        kind: Kind,
        recipient: &Certificate,
        value: &T,
    ) -> Result<Self> {
        let bytes = canonical(value)?;
        if bytes.len() > super::MAX_INVITATION_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        let ticket = ticket.hash()?;
        let aad = canonical(&(ticket, request, kind, recipient.device))?;
        let (enc, ciphertext) = crypto::seal_pairing(&recipient.hpke_key, &bytes, &aad)?;
        Ok(Self {
            ticket,
            request,
            kind,
            enc,
            ciphertext,
        })
    }

    pub fn open<T: DeserializeOwned + Serialize>(
        &self,
        ticket: &Ticket,
        request: Hash,
        kind: Kind,
        device: &DeviceSecret,
    ) -> Result<T> {
        if self.ticket != ticket.hash()? || self.request != request || self.kind != kind {
            return Err(Error::InvalidInvitation);
        }
        let aad = canonical(&(self.ticket, request, kind, device.id()))?;
        let bytes = device.open_pairing(&self.enc, &self.ciphertext, &aad)?;
        wire::decode_bounded(&bytes, super::MAX_INVITATION_BYTES)
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        let bytes = canonical(self)?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        wire::decode_bounded(bytes, MAX_MESSAGE_BYTES)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClosedReason {
    Used,
    Cancelled,
    Expired,
    Rejected,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClosureBody {
    pub ticket: Hash,
    pub reason: ClosedReason,
    pub request: Option<Hash>,
}
pub(crate) type Closure = Signed<ClosureBody>;

impl Closure {
    pub fn create(
        ticket: &Ticket,
        reason: ClosedReason,
        request: Option<Hash>,
        device: &DeviceSecret,
    ) -> Result<Self> {
        if device.id() != ticket.body.controller.body.device {
            return Err(Error::UnauthorizedDevice);
        }
        crypto::sign(
            ClosureBody {
                ticket: ticket.hash()?,
                reason,
                request,
            },
            &device.signer(),
            STATUS_DOMAIN,
        )
    }

    pub fn verify_closure(&self, ticket: &Ticket) -> Result<()> {
        if self.body.ticket != ticket.hash()? {
            return Err(Error::InvalidInvitation);
        }
        crypto::verify(
            self,
            &ticket.body.controller.body.signing_key,
            STATUS_DOMAIN,
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JoinedBody {
    pub ticket: Hash,
    pub request: Hash,
    pub invitation: Hash,
    pub control: Hash,
}
pub(crate) type Joined = Signed<JoinedBody>;

impl Joined {
    pub fn create(
        ticket: &Ticket,
        request: Hash,
        invitation: Hash,
        control: Hash,
        device: &DeviceSecret,
    ) -> Result<Self> {
        crypto::sign(
            JoinedBody {
                ticket: ticket.hash()?,
                request,
                invitation,
                control,
            },
            &device.signer(),
            ACK_DOMAIN,
        )
    }

    pub fn verify_joined(
        &self,
        ticket: &Ticket,
        request: &Contact,
        invitation: Hash,
        control: Hash,
    ) -> Result<()> {
        if self.body.ticket != ticket.hash()?
            || self.body.request != request.hash()?
            || self.body.invitation != invitation
            || self.body.control != control
        {
            return Err(Error::InvalidInvitation);
        }
        crypto::verify(self, &request.body.certificate.body.signing_key, ACK_DOMAIN)
    }
}
