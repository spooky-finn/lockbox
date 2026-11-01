//! Reviewed public artifacts and session-scoped file jobs. No provider or native
//! dialogs. Vault owns every durable admission/membership operation.

use super::workspace::{Page, Unlocked};
use crate::{
    core::sync::{
        Admission, AdmissionReceipt, Certificate, Hash, Invitation, Machine, Role, Signed,
        SpaceDescriptor, canonical, decode,
    },
    infra::{sharing_files, sync::worker::Worker, vault::Vault},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Certificate,
    Invitation,
    Response,
    Receipt,
}

#[cfg(test)]
#[path = "tests/sharing.rs"]
mod tests;

impl Kind {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Certificate => "lockbox-device.json",
            Self::Invitation => "lockbox-invitation.json",
            Self::Response => "lockbox-response.json",
            Self::Receipt => "lockbox-confirmation.json",
        }
    }
}

#[derive(Clone)]
pub(crate) enum Artifact {
    Certificate(Signed<Certificate>),
    Invitation(Invitation),
    Response(Admission),
    Receipt(AdmissionReceipt),
}

impl Artifact {
    fn decode(kind: Kind, bytes: &[u8]) -> Result<Self, String> {
        let result = (|| -> crate::core::sync::Result<Self> {
            Ok(match kind {
                Kind::Certificate => {
                    let certificate: Signed<Certificate> = decode(bytes)?;
                    certificate.verify()?;
                    Self::Certificate(certificate)
                }
                Kind::Invitation => {
                    let invitation = Invitation::decode(bytes)?;
                    Machine::verify_invitation(&invitation, invitation.descriptor())?;
                    Self::Invitation(invitation)
                }
                Kind::Response => {
                    let response = Admission::decode(bytes)?;
                    response.verify_consent()?;
                    Self::Response(response)
                }
                Kind::Receipt => Self::Receipt(AdmissionReceipt::decode(bytes)?),
            })
        })();
        result.map_err(|e| e.to_string())
    }
}

#[derive(Clone)]
pub(crate) struct Output {
    pub kind: Kind,
    pub bytes: Arc<Vec<u8>>,
    pub code: Option<Hash>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Ticket {
    session: u64,
    space: Option<SpaceDescriptor>,
}

enum FileReply {
    Imported(Box<Artifact>),
    Exported,
}

pub(crate) struct Sharing {
    session: u64,
    job: Option<(Ticket, Worker<Result<FileReply, String>>)>,
    pub review: Option<Artifact>,
    pub output: Option<Output>,
    pub recipient_verified: bool,
    pub code: String,
    pub expiry_days: u64,
    pub confirm_remove: Option<Hash>,
    pub notice: Option<&'static str>,
}

impl Default for Sharing {
    fn default() -> Self {
        static SESSIONS: AtomicU64 = AtomicU64::new(1);
        Self {
            session: SESSIONS.fetch_add(1, Ordering::Relaxed),
            job: None,
            review: None,
            output: None,
            recipient_verified: false,
            code: String::new(),
            expiry_days: 7,
            confirm_remove: None,
            notice: None,
        }
    }
}

impl Sharing {
    pub fn ticket(&self, vault: &Vault) -> Ticket {
        Ticket {
            session: self.session,
            space: vault.space_descriptor(),
        }
    }
    pub fn matches(&self, vault: &Vault, ticket: &Ticket) -> bool {
        self.ticket(vault) == *ticket
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn needs_poll(&self) -> bool {
        self.job.as_ref().is_some_and(|(_, job)| job.is_finished())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Certificate,
    Issue(Role),
    Accept,
    Admit,
    Join,
    Cancel(Hash),
    Reexport(Hash),
    Response(Hash),
    Refresh(Hash),
    Forget(Hash),
    Remove(Hash),
    Role(Hash, Role),
}

impl Unlocked {
    pub fn open_sharing(&mut self) {
        self.page = Page::Sharing;
        self.show_password = false;
        self.confirm_delete = false;
    }

    pub fn is_controller(&self) -> bool {
        self.vault.device_certificate().is_some_and(|certificate| {
            self.vault
                .space_controller()
                .is_ok_and(|id| id == certificate.body.device)
        })
    }

    pub fn import_sharing(&mut self, ticket: Ticket, path: PathBuf, kind: Kind) {
        if !self.sharing.matches(&self.vault, &ticket) || self.sharing.busy() {
            return;
        }
        self.sharing.job = Some((
            ticket,
            Worker::spawn(move |cancel| {
                let bytes = sharing_files::read(&path, &cancel)?;
                Artifact::decode(kind, &bytes)
                    .map(Box::new)
                    .map(FileReply::Imported)
            }),
        ));
        self.error = None;
    }

    pub fn export_sharing(&mut self, ticket: Ticket, path: PathBuf, output: Output) {
        if !self.sharing.matches(&self.vault, &ticket) || self.sharing.busy() {
            return;
        }
        if self
            .sharing
            .output
            .as_ref()
            .is_none_or(|current| !Arc::ptr_eq(&current.bytes, &output.bytes))
        {
            return;
        }
        self.sharing.job = Some((
            ticket,
            Worker::spawn(move |cancel| {
                sharing_files::write(&path, &output.bytes, &cancel).map(|()| FileReply::Exported)
            }),
        ));
        self.error = None;
    }

    pub fn poll_sharing(&mut self, now: u64) {
        if !self.sharing.needs_poll() {
            return;
        }
        let (ticket, mut job) = self.sharing.job.take().unwrap();
        if !self.sharing.matches(&self.vault, &ticket) {
            return;
        }
        let result = job
            .take()
            .unwrap()
            .map_err(|_| "Sharing file operation failed".to_owned())
            .and_then(|reply| reply);
        self.error = (|| -> Result<(), String> {
            match result? {
                FileReply::Exported => self.sharing.notice = Some("Sharing file saved"),
                FileReply::Imported(artifact) => {
                    match &*artifact {
                        Artifact::Response(response) => self
                            .vault
                            .verify_invitation_response(response, now)
                            .map_err(|e| e.to_string())?,
                        Artifact::Receipt(receipt) => {
                            self.vault
                                .verify_join_receipt(receipt)
                                .map_err(|e| e.to_string())?;
                        }
                        _ => {}
                    }
                    self.sharing.review = Some(*artifact);
                    self.sharing.output = None;
                    self.sharing.recipient_verified = false;
                    self.sharing.code.clear();
                    self.sharing.notice = None;
                }
            }
            Ok(())
        })()
        .err();
    }

    pub fn sharing_action(&mut self, action: Action, now: u64) {
        let result = self.apply_sharing(action, now);
        self.error = result.as_ref().err().cloned();
        if result.is_ok() {
            self.sync.saved();
        }
    }

    fn apply_sharing(&mut self, action: Action, now: u64) -> Result<(), String> {
        if self.is_editing() {
            return Err("Save or cancel the current edit before managing sharing".into());
        }
        if self.sharing.busy() {
            return Err("Wait for the sharing file operation to finish".into());
        }
        if !matches!(action, Action::Certificate) {
            self.vault.reconcile_profile().map_err(|e| e.to_string())?;
        }
        self.sync.suspend();
        let mut code = None;
        let (kind, bytes) = match action {
            Action::Certificate => (
                Kind::Certificate,
                canonical(
                    &self
                        .vault
                        .device_certificate()
                        .ok_or("This profile has no device certificate")?,
                )
                .map_err(|e| e.to_string())?,
            ),
            Action::Issue(role) => {
                if !self.sharing.recipient_verified {
                    return Err(
                        "Verify the recipient fingerprints through a trusted channel first".into(),
                    );
                }
                let Some(Artifact::Certificate(certificate)) = self.sharing.review.clone() else {
                    return Err("Import the recipient device file first".into());
                };
                let seconds = self
                    .sharing
                    .expiry_days
                    .checked_mul(86400)
                    .ok_or("Invalid invitation deadline")?;
                let expires = now
                    .checked_add(seconds)
                    .ok_or("Invalid invitation deadline")?;
                let invitation = self
                    .vault
                    .issue_invitation(certificate, role, Some(expires))
                    .map_err(|e| e.to_string())?;
                code = Some(invitation.verification_code().map_err(|e| e.to_string())?);
                (
                    Kind::Invitation,
                    invitation.bytes().map_err(|e| e.to_string())?,
                )
            }
            Action::Accept => {
                let Some(Artifact::Invitation(invitation)) = self.sharing.review.clone() else {
                    return Err("Import an invitation first".into());
                };
                let supplied = self
                    .sharing
                    .code
                    .trim()
                    .to_ascii_lowercase()
                    .parse::<Hash>()
                    .map_err(|_| "Enter the verification code received from the owner")?;
                if supplied != invitation.verification_code().map_err(|e| e.to_string())? {
                    return Err("The verification code does not match the invitation".into());
                }
                let expected = invitation.descriptor().clone();
                let response = self
                    .vault
                    .accept_invitation(invitation, &expected)
                    .map_err(|e| e.to_string())?;
                (Kind::Response, response.bytes().map_err(|e| e.to_string())?)
            }
            Action::Admit => {
                let Some(Artifact::Response(response)) = self.sharing.review.clone() else {
                    return Err("Import a signed response first".into());
                };
                let receipt = self
                    .vault
                    .admit_invitation(response, now)
                    .map_err(|e| e.to_string())?;
                (Kind::Receipt, receipt.bytes().map_err(|e| e.to_string())?)
            }
            Action::Join => {
                let Some(Artifact::Receipt(receipt)) = self.sharing.review.clone() else {
                    return Err("Import an admission confirmation first".into());
                };
                self.vault
                    .join_invitation(receipt)
                    .map_err(|e| e.to_string())?;
                self.selected = None;
                self.directories = Default::default();
                self.sharing = Sharing::default();
                self.sharing.notice = Some("This device joined the space");
                return Ok(());
            }
            Action::Reexport(id) => {
                let invitation = self
                    .vault
                    .export_invitation(id)
                    .map_err(|e| e.to_string())?;
                code = Some(invitation.verification_code().map_err(|e| e.to_string())?);
                (
                    Kind::Invitation,
                    invitation.bytes().map_err(|e| e.to_string())?,
                )
            }
            Action::Response(id) => (
                Kind::Response,
                self.vault
                    .export_invitation_response(id)
                    .map_err(|e| e.to_string())?
                    .bytes()
                    .map_err(|e| e.to_string())?,
            ),
            Action::Refresh(id) => (
                Kind::Receipt,
                self.vault
                    .refresh_invitation_receipt(id)
                    .map_err(|e| e.to_string())?
                    .bytes()
                    .map_err(|e| e.to_string())?,
            ),
            Action::Cancel(id) => {
                self.vault
                    .cancel_invitation(id)
                    .map_err(|e| e.to_string())?;
                self.sharing.notice = Some("Invitation cancelled");
                return Ok(());
            }
            Action::Forget(id) => {
                self.vault
                    .forget_invitation(id, now)
                    .map_err(|e| e.to_string())?;
                self.sharing.notice = Some("Invitation details forgotten");
                return Ok(());
            }
            Action::Remove(device) => {
                if self.sharing.confirm_remove != Some(device) {
                    return Err("Confirm which device to remove".into());
                }
                if self.vault.space_controller().map_err(|e| e.to_string())? == device {
                    return Err("The controlling device cannot be removed".into());
                }
                let members = self
                    .vault
                    .space_members()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .filter(|(c, _)| c.body.device != device)
                    .collect();
                self.vault
                    .set_space_members(members)
                    .map_err(|e| e.to_string())?;
                self.sharing.confirm_remove = None;
                self.sharing.notice = Some("Device access removed");
                return Ok(());
            }
            Action::Role(user, role) => {
                let members = self
                    .vault
                    .space_members()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|(c, old)| {
                        let new = if c.body.user == user { role } else { old };
                        (c, new)
                    })
                    .collect();
                self.vault
                    .set_space_members(members)
                    .map_err(|e| e.to_string())?;
                self.sharing.notice = Some("Participant role updated");
                return Ok(());
            }
        };
        self.sharing.output = Some(Output {
            kind,
            bytes: Arc::new(bytes),
            code,
        });
        self.sharing.review = None;
        self.sharing.code.clear();
        self.sharing.recipient_verified = false;
        self.sharing.notice = Some("Sharing file ready to save");
        Ok(())
    }
}
