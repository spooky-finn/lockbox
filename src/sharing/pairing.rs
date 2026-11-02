//! Session-bound background mailbox delivery. Workers see ciphertext and provider configuration only.

use crate::{
    platform::jobs::Worker,
    sharing::transport::MailboxFactory,
    sync::{
        protocol::Hash,
        scheduler::SpaceSync,
        transport::{Result as NetworkResult, TransportError},
    },
    vault::{Vault, profile::pairing::Slot},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

struct Scope {
    profile: PathBuf,
    device: Hash,
    session: Hash,
}

#[cfg(test)]
#[path = "tests/pairing.rs"]
mod tests;

struct Reply {
    published: Vec<Slot>,
    incoming: Vec<(Slot, Vec<u8>)>,
    seen: Vec<Hash>,
    next: Option<String>,
}

pub(crate) struct Pairing {
    worker: Option<(Scope, Worker<NetworkResult<Reply>>)>,
    factory: Option<Arc<dyn MailboxFactory>>,
    due: Instant,
    round: usize,
    cursors: BTreeMap<Hash, Option<String>>,
    seen: BTreeMap<Hash, BTreeSet<Hash>>,
    pub link: String,
    pub code: String,
    pub days: u64,
}

impl Default for Pairing {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Pairing {
    pub fn new(factory: Option<Arc<dyn MailboxFactory>>) -> Self {
        Self {
            worker: None,
            factory,
            due: Instant::now(),
            round: 0,
            cursors: BTreeMap::new(),
            seen: BTreeMap::new(),
            link: String::new(),
            code: String::new(),
            days: 7,
        }
    }

    pub fn wake(&mut self) {
        self.due = Instant::now();
    }

    pub fn suspend(&mut self) {
        self.worker = None;
        self.wake();
    }

    pub fn needs_tick(&self, vault: &Vault, editing: bool) -> bool {
        !editing
            && vault.has_pairing()
            && vault.signing_error().is_none()
            && self.worker.as_ref().map_or_else(
                || Instant::now() >= self.due,
                |(_, worker)| worker.is_finished(),
            )
    }

    pub fn tick(
        &mut self,
        vault: &mut Vault,
        sync: &mut SpaceSync,
        editing: bool,
        now: u64,
    ) -> Option<String> {
        if editing || vault.signing_error().is_some() {
            self.worker = None;
            return None;
        }
        self.advance(vault, sync, now).err().map(|error| {
            self.worker = None;
            self.due = Instant::now() + Duration::from_secs(15);
            error.to_string()
        })
    }

    fn advance(
        &mut self,
        vault: &mut Vault,
        sync: &mut SpaceSync,
        now: u64,
    ) -> crate::vault::Result<()> {
        if let Some((scope, worker)) = &mut self.worker {
            if scope.profile != vault.path()
                || vault
                    .device_certificate()
                    .is_none_or(|certificate| certificate.body.device != scope.device)
            {
                self.worker = None;
                self.cursors.clear();
                self.seen.clear();
                self.wake();
                return Ok(());
            }
            let Some(reply) = worker.take() else {
                return Ok(());
            };
            let session = scope.session;
            self.worker = None;
            let reply = reply.map_err(|_| TransportError::Unavailable)??;
            vault.acknowledge_pairing(session, &reply.published)?;
            self.cursors.insert(session, reply.next);
            let seen = self.seen.entry(session).or_default();
            if seen.len() + reply.seen.len() > 512 {
                seen.clear();
            }
            seen.extend(reply.seen);
            let mut failure = None;
            for (slot, bytes) in reply.incoming {
                if matches!(slot, Slot::Consent(_) | Slot::Receipt(_)) {
                    sync.suspend();
                }
                if let Err(error) = vault.receive_pairing(session, slot, &bytes, now) {
                    // One malformed request cannot prevent other valid requests being reviewed.
                    if matches!(slot, Slot::Request(_)) {
                        continue;
                    }
                    failure = Some(error);
                } else if matches!(slot, Slot::Consent(_) | Slot::Receipt(_)) {
                    sync.saved();
                }
            }
            self.due = Instant::now() + Duration::from_secs(3);
            if let Some(error) = failure {
                return Err(error);
            }
            return Ok(());
        }
        if Instant::now() < self.due {
            return Ok(());
        }
        let plans = vault.pairing_plans(now)?;
        if plans.is_empty() {
            self.due = Instant::now() + Duration::from_secs(15);
            return Ok(());
        }
        let plan = plans[self.round % plans.len()].clone();
        self.round = self.round.wrapping_add(1);
        let session = plan.ticket.body.id;
        let scope = Scope {
            profile: vault.path().to_path_buf(),
            device: vault.device_certificate().unwrap().body.device,
            session,
        };
        let factory = match &self.factory {
            Some(factory) => factory.clone(),
            None => vault.pairing_factory(&plan.ticket)?,
        };
        let cursor = self.cursors.get(&session).cloned().flatten();
        let mut seen = self.seen.get(&session).cloned().unwrap_or_default();
        seen.extend(plan.known_requests.iter().copied());
        self.worker = Some((
            scope,
            Worker::spawn(move |cancel| {
                let mut mailbox = factory.connect()?;
                let mut reply = Reply {
                    published: Vec::new(),
                    incoming: Vec::new(),
                    seen: Vec::new(),
                    next: None,
                };
                // Fetch terminal policy first; an old in-flight offer must not bypass cancellation.
                for slot in &plan.probes {
                    cancel.check()?;
                    if let Some(bytes) = mailbox.get(session, *slot, &cancel)? {
                        reply.incoming.push((*slot, bytes));
                    }
                }
                for delivery in &plan.outgoing {
                    cancel.check()?;
                    mailbox.put(session, delivery.slot, &delivery.bytes.0, &cancel)?;
                    reply.published.push(delivery.slot);
                }
                if plan.list_requests {
                    cancel.check()?;
                    let page = mailbox.requests(session, cursor.as_deref())?;
                    if page.objects.len() > 1000
                        || page
                            .next
                            .as_ref()
                            .is_some_and(|next| Some(next) == cursor.as_ref())
                    {
                        return Err(TransportError::InvalidResponse);
                    }
                    reply.next = page.next;
                    for request in page
                        .objects
                        .into_iter()
                        .filter(|request| !seen.contains(request))
                        .take(16)
                    {
                        cancel.check()?;
                        match mailbox.get(session, Slot::Request(request), &cancel) {
                            Ok(Some(bytes)) => {
                                reply.seen.push(request);
                                reply.incoming.push((Slot::Request(request), bytes));
                            }
                            Ok(None) => {}
                            Err(
                                TransportError::Budget
                                | TransportError::InvalidResponse
                                | TransportError::Verification(_),
                            ) => {
                                reply.seen.push(request);
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                Ok(reply)
            }),
        ));
        Ok(())
    }
}
