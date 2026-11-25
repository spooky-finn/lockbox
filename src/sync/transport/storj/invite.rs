//! One guest mailbox per invitation. Each capability signs one method and one
//! object under a random session prefix. Guest writes have a signed fixed size.
use super::{Storj, pairing::Store};
use crate::sync::{
    protocol::{
        Hash,
        pairing::{Kind, MAX_CONTACT_BYTES, Slot},
    },
    transport::{
        Cancellation, Page, Result, TransportError,
        pairing::{Mailbox, MailboxFactory},
    },
};
use aws_sdk_s3::presigning::PresigningConfig;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{io::Read, time::Duration};

mod network;
const FRAME_BYTES: usize = MAX_CONTACT_BYTES + 4;
const ZERO: Hash = Hash([0; 32]);
const ROUTES: [Slot; 8] = [
    Slot::Bootstrap,
    Slot::Status,
    Slot::Offer(ZERO),
    Slot::Receipt(ZERO),
    Slot::Rejected(ZERO),
    Slot::Request(ZERO),
    Slot::Consent(ZERO),
    Slot::Joined(ZERO),
];

fn route(slot: Slot) -> Result<(usize, &'static str)> {
    Ok(match slot {
        Slot::Bootstrap => (0, "bootstrap"),
        Slot::Status => (1, "status"),
        Slot::Offer(_) => (2, "offer"),
        Slot::Receipt(_) => (3, "receipt"),
        Slot::Rejected(_) => (4, "rejection"),
        Slot::Request(_) => (5, "request"),
        Slot::Consent(_) => (6, "consent"),
        Slot::Joined(_) => (7, "joined"),
        Slot::Ticket(_) => return Err(TransportError::AccessDenied),
    })
}
fn guest_writes(slot: Slot) -> bool {
    matches!(slot, Slot::Request(_) | Slot::Consent(_) | Slot::Joined(_))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Grant {
    urls: [String; 8],
}

pub(crate) struct Owner {
    store: Store,
    listed_request: Option<(Hash, Vec<u8>)>,
    session: Hash,
    http: Client,
}

impl Owner {
    pub(crate) fn new(
        endpoint: &str,
        bucket: &str,
        space: &str,
        access: &str,
        secret: &str,
        session: Hash,
    ) -> Result<Self> {
        Ok(Self {
            listed_request: None,
            store: Store(Storj::new(endpoint, bucket, space, access, secret)?),
            session,
            http: network::builder()
                .build()
                .map_err(|_| TransportError::Unavailable)?,
        })
    }

    fn sign(&self, slot: Slot, write: bool, length: Option<i64>, ttl: Duration) -> Result<String> {
        let s = &self.store.0;
        let key = format!(
            "{}invite-v1/{}/{}",
            s.prefix
                .strip_suffix("objects/")
                .ok_or(TransportError::InvalidResponse)?,
            self.session,
            route(slot)?.1
        );
        let config =
            PresigningConfig::expires_in(ttl).map_err(|_| TransportError::InvalidResponse)?;
        let signed = if write {
            s.runtime
                .block_on(
                    s.client
                        .put_object()
                        .bucket(&s.bucket)
                        .key(key)
                        .set_content_length(length)
                        .presigned(config),
                )
                .map_err(|_| TransportError::Unavailable)?
        } else {
            s.runtime
                .block_on(
                    s.client
                        .get_object()
                        .bucket(&s.bucket)
                        .key(key)
                        .presigned(config),
                )
                .map_err(|_| TransportError::Unavailable)?
        };
        let url = signed.uri().to_owned();
        let signed_headers = if length.is_some() {
            "content-length;host"
        } else {
            "host"
        };
        if !url::Url::parse(&url).ok().is_some_and(|u| {
            u.query_pairs()
                .any(|(k, v)| k == "X-Amz-SignedHeaders" && v == signed_headers)
        }) {
            return Err(TransportError::InvalidResponse);
        }
        Ok(url)
    }

    pub(crate) fn grant(&self, ttl: Duration) -> Result<Grant> {
        let mut urls = std::array::from_fn(|_| String::new());
        for (index, slot) in ROUTES.into_iter().enumerate() {
            let write = guest_writes(slot);
            urls[index] = self.sign(slot, write, write.then_some(FRAME_BYTES as i64), ttl)?;
        }
        Ok(Grant { urls })
    }
}

pub(crate) struct Guest {
    urls: Grant,
    session: Hash,
    http: Client,
}

impl Guest {
    pub(crate) fn new(
        grant: &Grant,
        session: Hash,
        endpoint: &str,
        bucket: &str,
        space: &str,
    ) -> Result<Self> {
        let endpoint =
            crate::sync::transport::provider::validate(endpoint, bucket, "public", "public")
                .map_err(|_| TransportError::InvalidResponse)?;
        grant.validate(&endpoint, bucket, space, session)?;
        Ok(Self {
            urls: grant.clone(),
            session,
            http: network::guest_client(&endpoint)?,
        })
    }
}

// Only route an object here. The vault checks the canonical envelope, its
// recipient, signed ticket, challenge and encrypted contents before using it.
// Ignoring ciphertext fields avoids allocating them a second time during routing.
#[derive(Deserialize)]
struct Address {
    kind: Kind,
    request: Hash,
}

fn addressed(slot: Slot, bytes: Option<Vec<u8>>) -> Result<Option<Vec<u8>>> {
    let Some(bytes) = bytes else { return Ok(None) };
    let (request, kind) = match slot {
        Slot::Offer(id) => (id, Kind::Offer),
        Slot::Receipt(id) => (id, Kind::Receipt),
        Slot::Rejected(id) => (id, Kind::Rejected),
        Slot::Request(id) => (id, Kind::Request),
        Slot::Consent(id) => (id, Kind::Consent),
        Slot::Joined(id) => (id, Kind::Joined),
        _ => return Ok(Some(bytes)),
    };
    let address: Address =
        serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
    Ok((address.request == request && address.kind == kind).then_some(bytes))
}

fn check_session(expected: Hash, actual: Hash) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(TransportError::AccessDenied)
    }
}

fn read(client: &Client, url: &str, slot: Slot, cancel: &Cancellation) -> Result<Option<Vec<u8>>> {
    cancel.check()?;
    let response = client
        .get(url)
        .send()
        .map_err(|_| TransportError::Unavailable)?;
    // S3 masks missing keys with 403 when the signing identity lacks listing
    // rights. Bootstrap is required and surfaces denial. Other slots are
    // optional until the matching peer publishes them.
    if response.status() == reqwest::StatusCode::NOT_FOUND
        || (response.status() == reqwest::StatusCode::FORBIDDEN && slot != Slot::Bootstrap)
    {
        return Ok(None);
    }
    super::transport::check(response.status())?;
    let limit = if guest_writes(slot) {
        FRAME_BYTES
    } else {
        slot.budget()
    };
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(TransportError::Budget);
    }
    let mut bytes = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| TransportError::Unavailable)?;
    cancel.check()?;
    if bytes.len() > limit {
        return Err(TransportError::Budget);
    }
    if guest_writes(slot) {
        if bytes.len() != FRAME_BYTES {
            return Err(TransportError::InvalidResponse);
        }
        let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        if length == 0 || length > slot.budget() {
            return Err(TransportError::Budget);
        }
        bytes.copy_within(4..4 + length, 0);
        bytes.truncate(length);
    }
    Ok(Some(bytes))
}

fn write(
    client: &Client,
    url: &str,
    slot: Slot,
    bytes: &[u8],
    cancel: &Cancellation,
) -> Result<()> {
    cancel.check()?;
    if bytes.is_empty() || bytes.len() > slot.budget() {
        return Err(TransportError::Budget);
    }
    let data = if guest_writes(slot) {
        let mut frame = vec![0; FRAME_BYTES];
        frame[..4].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        frame[4..4 + bytes.len()].copy_from_slice(bytes);
        frame
    } else {
        bytes.to_vec()
    };
    let response = client
        .put(url)
        .body(data)
        .send()
        .map_err(|_| TransportError::Unavailable)?;
    super::transport::check(response.status())
}

impl Mailbox for Guest {
    fn get(&mut self, session: Hash, slot: Slot, cancel: &Cancellation) -> Result<Option<Vec<u8>>> {
        check_session(self.session, session)?;
        if guest_writes(slot) {
            return Err(TransportError::AccessDenied);
        }
        addressed(
            slot,
            read(&self.http, &self.urls.urls[route(slot)?.0], slot, cancel)?,
        )
    }

    fn put(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: &[u8],
        cancel: &Cancellation,
    ) -> Result<()> {
        check_session(self.session, session)?;
        if !guest_writes(slot) {
            return Err(TransportError::AccessDenied);
        }
        write(
            &self.http,
            &self.urls.urls[route(slot)?.0],
            slot,
            bytes,
            cancel,
        )
    }

    fn requests(&mut self, _: Hash, _: Option<&str>) -> Result<Page> {
        Err(TransportError::AccessDenied)
    }
}

impl Mailbox for Owner {
    fn get(&mut self, session: Hash, slot: Slot, cancel: &Cancellation) -> Result<Option<Vec<u8>>> {
        if matches!(slot, Slot::Ticket(_)) {
            return self.store.get(session, slot, cancel);
        }
        check_session(self.session, session)?;
        if let Slot::Request(expected) = slot
            && let Some((actual, bytes)) = self.listed_request.take()
            && expected == actual
        {
            cancel.check()?;
            return Ok(Some(bytes));
        }
        let url = self.sign(slot, false, None, Duration::from_secs(300))?;
        match read(&self.http, &url, slot, cancel).and_then(|bytes| addressed(slot, bytes)) {
            // An invite bearer can replace these slots with arbitrary bytes.
            // Ignore invalid replacements so receipt publication can proceed.
            Err(TransportError::InvalidResponse | TransportError::Budget) if guest_writes(slot) => {
                Ok(None)
            }
            other => other,
        }
    }
    fn put(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: &[u8],
        cancel: &Cancellation,
    ) -> Result<()> {
        if matches!(slot, Slot::Ticket(_)) {
            return self.store.put(session, slot, bytes, cancel);
        }
        check_session(self.session, session)?;
        if guest_writes(slot) {
            return Err(TransportError::AccessDenied);
        }
        let url = self.sign(
            slot,
            true,
            Some(bytes.len() as i64),
            Duration::from_secs(300),
        )?;
        write(&self.http, &url, slot, bytes, cancel)
    }
    fn requests(&mut self, session: Hash, cursor: Option<&str>) -> Result<Page> {
        if cursor.is_some() {
            return Err(TransportError::InvalidResponse);
        }
        check_session(self.session, session)?;
        let slot = Slot::Request(ZERO);
        let url = self.sign(slot, false, None, Duration::from_secs(300))?;
        let mut objects = Vec::new();
        match read(&self.http, &url, slot, &Cancellation::default()) {
            Ok(Some(bytes)) => {
                if let Ok(address) = serde_json::from_slice::<Address>(&bytes)
                    && address.kind == Kind::Request
                {
                    self.listed_request = Some((address.request, bytes));
                    objects.push(address.request);
                }
            }
            // A malformed inbox write must not roll back acknowledgements for
            // unrelated publications made earlier in this same worker pass.
            Ok(None) | Err(TransportError::InvalidResponse | TransportError::Budget) => {}
            Err(error) => return Err(error),
        }
        Ok(Page {
            objects,
            next: None,
        })
    }
}

pub(crate) struct OwnerFactory {
    provider: super::StorjFactory,
    session: Hash,
}

impl OwnerFactory {
    pub(crate) fn new(
        endpoint: &str,
        bucket: &str,
        space: &str,
        access: &str,
        secret: &str,
        session: Hash,
    ) -> Self {
        Self {
            provider: super::StorjFactory::new(endpoint, bucket, space, access, secret),
            session,
        }
    }
}

impl MailboxFactory for OwnerFactory {
    fn connect(&self) -> Result<Box<dyn Mailbox>> {
        let p = &self.provider;
        Ok(Box::new(Owner::new(
            &p.endpoint,
            &p.bucket,
            &p.space_name,
            &p.access,
            &p.secret,
            self.session,
        )?))
    }
}

pub(crate) struct GuestFactory {
    grant: Grant,
    session: Hash,
    endpoint: String,
    bucket: String,
    space: String,
}

impl GuestFactory {
    pub(crate) fn new(
        grant: &Grant,
        session: Hash,
        endpoint: &str,
        bucket: &str,
        space: &str,
    ) -> Self {
        Self {
            grant: grant.clone(),
            session,
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            space: space.into(),
        }
    }
}

impl MailboxFactory for GuestFactory {
    fn connect(&self) -> Result<Box<dyn Mailbox>> {
        Ok(Box::new(Guest::new(
            &self.grant,
            self.session,
            &self.endpoint,
            &self.bucket,
            &self.space,
        )?))
    }
}

#[cfg(test)]
mod tests;
