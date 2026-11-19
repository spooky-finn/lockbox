//! The writer borrows ownership of the live vault for one draft commit at a time.

use crate::{
    desktop::workspace::{Destination, Unlocked},
    records::entry::Entry,
    vault::Vault,
};
use std::sync::{
    Arc,
    mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub(super) struct SaveWake(Arc<Notify>);

impl SaveWake {
    pub(super) async fn notified(&self) {
        self.0.notified().await;
    }
}

const INTERRUPTED: &str = "Saving the record was interrupted";

pub(super) struct WriteCompleted {
    vault: Vault,
    result: crate::vault::Result<()>,
}

pub(super) struct WriterLost;

type Reply = Result<WriteCompleted, WriterLost>;

struct Job {
    vault: Vault,
    entry: Entry,
    reply: SyncSender<WriteCompleted>,
}

impl Job {
    fn commit(self) {
        let Self {
            mut vault,
            entry,
            reply,
        } = self;
        let result = vault.upsert_entry(entry);
        let _ = reply.send(WriteCompleted { vault, result });
    }
}

enum AfterSave {
    SaveAndClose,
    CloseEditor,
    Navigate(Destination),
}

#[derive(Default)]
enum WriteState {
    #[default]
    Idle,
    Writing(Receiver<WriteCompleted>),
    Lost,
}

#[derive(Default)]
pub(super) struct DraftSaves {
    writer: Option<SyncSender<Job>>,
    state: WriteState,
    after: Option<AfterSave>,
    wake: Option<SaveWake>,
}

impl DraftSaves {
    fn submit(&mut self, vault: &mut Vault, entry: Entry) -> Result<(), String> {
        debug_assert!(matches!(self.state, WriteState::Idle));
        if self.writer.is_none() {
            self.writer = Some(
                writer(self.wake.clone()).map_err(|e| format!("Could not start saving: {e}"))?,
            );
        }
        let snapshot = vault.editor_snapshot();
        let (reply, receive) = mpsc::sync_channel(1);
        let job = Job {
            vault: std::mem::replace(vault, snapshot),
            entry,
            reply,
        };
        if let Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) =
            self.writer.as_ref().unwrap().try_send(job)
        {
            *vault = job.vault;
            self.writer = None;
            return Err(INTERRUPTED.into());
        }
        self.state = WriteState::Writing(receive);
        Ok(())
    }
}

fn writer(wake: Option<SaveWake>) -> std::io::Result<SyncSender<Job>> {
    let (send, receive) = mpsc::sync_channel::<Job>(1);
    std::thread::Builder::new()
        .name("lockbox-vault-writer".into())
        .spawn(move || {
            while let Ok(job) = receive.recv() {
                job.commit();
                if let Some(wake) = &wake {
                    wake.0.notify_one();
                }
            }
        })?;
    Ok(send)
}

impl Unlocked {
    pub(crate) fn saving_draft(&self) -> bool {
        !matches!(self.draft_saves.state, WriteState::Idle)
    }

    pub(super) fn set_save_wake(&mut self, wake: Option<SaveWake>) {
        self.draft_saves.wake = wake;
    }

    pub(super) fn autosave_due(&self) -> bool {
        !self.saving_draft()
            && self
                .draft
                .as_ref()
                .and_then(|draft| draft.autosave_at)
                .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    }

    pub(crate) fn request_autosave(&mut self) {
        if !self.saving_draft() {
            self.error = self.start_write(None).err();
        }
    }

    pub(crate) fn request_save(&mut self) {
        self.request_after(AfterSave::SaveAndClose);
    }

    pub(crate) fn request_close_editor(&mut self) {
        self.request_after(AfterSave::CloseEditor);
    }

    pub(crate) fn request_navigation(&mut self, destination: Destination) {
        if self.directories.draft.is_none() {
            self.request_after(AfterSave::Navigate(destination));
        }
    }

    fn request_after(&mut self, action: AfterSave) {
        if self.draft_saves.after.is_some() {
            return;
        }
        self.draft_saves.after = Some(action);
        self.continue_after_save();
    }

    fn continue_after_save(&mut self) {
        if self.saving_draft() {
            return;
        }
        let Some(action) = self.draft_saves.after.take() else {
            return;
        };
        match self.start_write(Some(&action)) {
            Ok(()) if self.saving_draft() => self.draft_saves.after = Some(action),
            Ok(()) => {
                if let Some(draft) = self.draft.take().filter(|draft| !draft.is_empty()) {
                    self.finish_saved_draft(draft.entry.id.clone());
                }
                match action {
                    AfterSave::Navigate(destination) => self.finish_navigation(destination),
                    AfterSave::SaveAndClose | AfterSave::CloseEditor => self.error = None,
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn start_write(&mut self, action: Option<&AfterSave>) -> Result<(), String> {
        let Some(draft) = &mut self.draft else {
            return Ok(());
        };
        // A failed write does not retry every tick. Further edits and explicit
        // requests can retry. Input during a write sets a fresh deadline.
        draft.autosave_at = None;
        if draft.is_empty() && !matches!(action, Some(AfterSave::SaveAndClose)) {
            return Ok(());
        }
        let entry = draft.prepare_save()?;
        if self.vault.entries().contains(&entry) {
            draft.is_new = false;
            return Ok(());
        }
        self.draft_saves.submit(&mut self.vault, entry)
    }

    pub(super) fn try_save_reply(&self) -> Option<Reply> {
        let WriteState::Writing(receive) = &self.draft_saves.state else {
            return None;
        };
        match receive.try_recv() {
            Ok(reply) => Some(Ok(reply)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(WriterLost)),
        }
    }

    pub(super) fn accept_save_reply(&mut self, reply: Reply) {
        match self.accept_reply(reply) {
            Ok(()) => self.continue_after_save(),
            Err(error) => {
                // A failed commit keeps the draft open for retry.
                self.error = Some(error);
                self.draft_saves.after = None;
            }
        }
    }

    fn accept_reply(&mut self, reply: Reply) -> Result<(), String> {
        let Ok(WriteCompleted { vault, result }) = reply else {
            self.draft_saves.state = WriteState::Lost;
            return Err(INTERRUPTED.into());
        };
        self.draft_saves.state = WriteState::Idle;
        self.vault = vault;
        result.map_err(|error| format!("Could not save: {error}"))?;
        self.record_draft_saved();
        Ok(())
    }

    fn wait_for_write(&mut self) -> Result<(), String> {
        let reply = match &self.draft_saves.state {
            WriteState::Idle => return Ok(()),
            WriteState::Lost => return Err(INTERRUPTED.into()),
            WriteState::Writing(receive) => receive.recv().map_err(|_| WriterLost),
        };
        self.accept_reply(reply)
    }

    /// Lock and shutdown drain the same writer before releasing the file lease.
    /// At most one final snapshot is queued after an already submitted write.
    pub(super) fn flush_draft_for_lock(&mut self) -> Result<(), String> {
        self.draft_saves.after = None;
        let previous = self.wait_for_write();
        if matches!(self.draft_saves.state, WriteState::Lost)
            || (previous.is_err() && self.draft.as_ref().is_none_or(|d| d.autosave_at.is_none()))
        {
            return previous;
        }
        self.start_write(None)?;
        self.wait_for_write()
    }
}

#[cfg(test)]
#[path = "tests/draft_save.rs"]
mod tests;
