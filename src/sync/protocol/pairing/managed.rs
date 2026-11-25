//! Controller-signed mailbox routes and device-specific storage delivery.
use super::*;
use crate::sync::{protocol::AdmissionReceipt, transport::storj::invite::Grant};
use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};
use std::io::{Read, Write};
use zeroize::{Zeroize, ZeroizeOnDrop};

const PREFIX: &str = "lockbox://pair/";
const MAILBOX_DOMAIN: &[u8] = b"lockbox.pairing.mailbox\0";
const STORAGE_DOMAIN: &[u8] = b"lockbox.pairing.storage\0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedBody {
    pub ticket: Ticket,
    pub grant: Grant,
}
pub(crate) type ManagedLink = Signed<ManagedBody>;

impl ManagedLink {
    pub fn create(ticket: Ticket, grant: Grant, device: &DeviceSecret) -> Result<Self> {
        crypto::sign(
            ManagedBody { ticket, grant },
            &device.signer(),
            MAILBOX_DOMAIN,
        )
    }

    pub fn link(&self) -> Result<String> {
        let bytes = canonical(self)?;
        if bytes.len() > MAX_LINK_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        // The eight URLs share long path and signing fields. Keep one portable
        // link without an external URL shortener or additional network lookup.
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder
            .write_all(&bytes)
            .map_err(|_| Error::InvalidEncoding)?;
        let compressed = encoder.finish().map_err(|_| Error::InvalidEncoding)?;
        let link = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(compressed));
        if link.len() > MAX_LINK_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        Ok(link)
    }

    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.len() > MAX_LINK_BYTES {
            return Err(Error::ResourceBudgetExceeded);
        }
        let raw = URL_SAFE_NO_PAD
            .decode(value.strip_prefix(PREFIX).ok_or(Error::InvalidInvitation)?)
            .map_err(|_| Error::InvalidEncoding)?;
        let mut decoder = ZlibDecoder::new(raw.as_slice());
        let mut bytes = Vec::new();
        decoder
            .by_ref()
            .take(MAX_LINK_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::InvalidEncoding)?;
        if decoder.total_in() != raw.len() as u64 {
            return Err(Error::InvalidEncoding);
        }
        let link: Self = wire::decode_bounded(&bytes, MAX_LINK_BYTES)?;
        link.verify_mailbox()?;
        Ok(link)
    }

    pub fn verify_mailbox(&self) -> Result<()> {
        let ticket = &self.body.ticket;
        ticket.verify_ticket()?;
        crypto::verify(
            self,
            &ticket.body.controller.body.signing_key,
            MAILBOX_DOMAIN,
        )?;
        let endpoint =
            url::Url::parse(&ticket.body.endpoint).map_err(|_| Error::InvalidInvitation)?;
        self.body
            .grant
            .validate(
                &endpoint,
                &ticket.body.bucket,
                &ticket.body.space_name,
                ticket.body.id,
            )
            .map_err(|_| Error::InvalidInvitation)
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeviceStorage {
    ticket: Hash,
    request: Hash,
    control: Hash,
    pub access: String,
    pub secret: String,
}

pub(crate) type SignedStorage = Signed<DeviceStorage>;

impl SignedStorage {
    pub fn issue(
        ticket: &Ticket,
        request: Hash,
        receipt: &AdmissionReceipt,
        access: String,
        secret: String,
        device: &DeviceSecret,
    ) -> Result<Self> {
        crypto::sign(
            DeviceStorage {
                ticket: ticket.hash()?,
                request,
                control: receipt.control(),
                access,
                secret,
            },
            &device.signer(),
            STORAGE_DOMAIN,
        )
    }

    pub fn verify_storage(
        &self,
        ticket: &Ticket,
        request: Hash,
        receipt: &AdmissionReceipt,
    ) -> Result<()> {
        if self.body.ticket != ticket.hash()?
            || self.body.request != request
            || self.body.control != receipt.control()
        {
            return Err(Error::InvalidInvitation);
        }
        crypto::verify(
            self,
            &ticket.body.controller.body.signing_key,
            STORAGE_DOMAIN,
        )
    }
}
