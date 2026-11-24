use super::*;
use crate::{
    desktop::test_support::{add, reopen, unlocked},
    vault::VaultError,
};

impl Unlocked {
    pub(crate) fn navigate(&mut self, destination: Destination) {
        self.request_navigation(destination);
        self.complete_background_save();
    }

    pub(crate) fn complete_background_save(&mut self) {
        while let WriteState::Writing(receive) = &self.draft_saves.state {
            let reply = receive.recv().map_err(|_| WriterLost);
            self.accept_save_reply(reply);
        }
    }

    pub(crate) fn autosave_draft(&mut self) -> Result<(), String> {
        self.request_autosave();
        self.save_result()
    }

    pub(crate) fn save_draft(&mut self) -> Result<(), String> {
        self.request_save();
        self.save_result()
    }

    pub(crate) fn close_draft(&mut self) -> Result<(), String> {
        self.request_close_editor();
        self.save_result()
    }

    fn save_result(&mut self) -> Result<(), String> {
        self.complete_background_save();
        self.error.clone().map_or(Ok(()), Err)
    }
}

#[test]
fn late_edits_stay_pending_until_the_next_ordered_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let id = add(
        &mut workspace,
        "Original",
        "user",
        "https://example.invalid",
    );
    workspace.start_edit();
    workspace.draft.as_mut().unwrap().entry.password = "original fictional secret".into();
    workspace.save_draft().unwrap();
    workspace.start_edit();
    workspace.draft.as_mut().unwrap().entry.title = "First snapshot".into();
    workspace.draft.as_mut().unwrap().entry.password = "fictional secret".into();
    let (send, jobs) = mpsc::sync_channel(1);
    workspace.draft_saves.writer = Some(send);
    workspace.request_autosave();

    assert!(workspace.saving_draft());
    assert_eq!(workspace.vault.entries()[0].title, "Original");
    assert!(workspace.vault.entries()[0].password.is_empty());
    assert!(matches!(
        workspace.vault.save(),
        Err(VaultError::ProfileInUse)
    ));

    let draft = workspace.draft.as_mut().unwrap();
    draft.entry.title = "Latest snapshot".into();
    draft.schedule_autosave();
    workspace.request_navigation(Destination::Close);
    let first = jobs.try_recv().unwrap();
    assert!(matches!(jobs.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(first.entry.title, "First snapshot");
    first.commit();
    let reply = workspace.try_save_reply().unwrap();
    workspace.accept_save_reply(reply);

    assert_eq!(reopen(&workspace).entries[0].title, "First snapshot");
    assert_eq!(
        workspace.draft.as_ref().unwrap().entry.title,
        "Latest snapshot"
    );
    assert_eq!(workspace.selected.as_deref(), Some(id.as_str()));
    let second = jobs.try_recv().unwrap();
    assert_eq!(second.entry.title, "Latest snapshot");
    second.commit();
    let reply = workspace.try_save_reply().unwrap();
    workspace.accept_save_reply(reply);
    assert!(!workspace.saving_draft());
    assert!(workspace.draft.is_none());
    assert!(workspace.selected.is_none());
    assert_eq!(reopen(&workspace).entries[0].title, "Latest snapshot");
}

#[test]
fn disconnected_writer_cannot_resubmit_the_read_only_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    workspace.start_new();
    workspace.draft.as_mut().unwrap().entry.title = "Pending".into();
    let (send, jobs) = mpsc::sync_channel(1);
    workspace.draft_saves.writer = Some(send);
    workspace.request_save();

    drop(jobs.try_recv().unwrap());
    let reply = workspace.try_save_reply().unwrap();
    assert!(matches!(reply, Err(WriterLost)));
    workspace.accept_save_reply(reply);

    workspace.request_autosave();
    assert!(matches!(jobs.try_recv(), Err(TryRecvError::Empty)));
    assert!(workspace.draft.is_some());
    assert_eq!(workspace.flush_draft_for_lock(), Err(INTERRUPTED.into()));
    assert!(reopen(&workspace).entries.is_empty());
}

#[test]
fn explicit_save_after_autosave_leaves_edit_mode_on_the_same_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let id = add(
        &mut workspace,
        "Original",
        "user",
        "https://example.invalid",
    );
    workspace.start_edit();
    let draft = workspace.draft.as_mut().unwrap();
    draft.entry.title = "Renamed".into();
    draft.schedule_autosave();
    workspace.autosave_draft().unwrap();
    assert!(workspace.draft.as_ref().is_some_and(|draft| draft.edited));

    workspace.save_draft().unwrap();

    assert!(workspace.draft.is_none());
    assert_eq!(workspace.selected.as_deref(), Some(id.as_str()));
    assert_eq!(reopen(&workspace).entries[0].title, "Renamed");
}

#[test]
fn escape_leaves_edit_mode_first_and_closes_the_record_next() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let id = add(
        &mut workspace,
        "Original",
        "user",
        "https://example.invalid",
    );
    workspace.start_edit();
    let draft = workspace.draft.as_mut().unwrap();
    draft.entry.title = "Renamed".into();
    draft.schedule_autosave();

    workspace.escape();
    workspace.complete_background_save();
    assert!(workspace.draft.is_none());
    assert_eq!(workspace.selected.as_deref(), Some(id.as_str()));
    assert_eq!(reopen(&workspace).entries[0].title, "Renamed");

    workspace.escape();
    workspace.complete_background_save();
    assert!(workspace.selected.is_none());
}
