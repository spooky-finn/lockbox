//! Session-scoped workers carry ciphertext and provider credentials only. Dropping
//! the session cancels work; no detached result can unlock or restore plaintext.
use crate::{
    core::sync::{Error, Hash},
    infra::{
        storj::Storj,
        sync::{self, Cancellation, Download, TransportError},
        vault::Vault,
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

enum Reply {
    Downloaded(sync::Result<Download>),
    Uploaded(sync::Result<Vec<Hash>>),
}
struct Worker {
    reply: Arc<Mutex<Option<Reply>>>,
    finished: Arc<AtomicBool>,
    cancel: Cancellation,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Status {
    #[default]
    Saved,
    Syncing,
    Synced,
    Offline,
    Incomplete,
    Conflict,
    Removed,
    Failed,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "Сохранено на устройстве",
            Self::Syncing => "Синхронизация…",
            Self::Synced => "Обмен со Storj завершён",
            Self::Offline => "Нет связи · данные сохранены на устройстве",
            Self::Incomplete => "История неполная · ожидаем данные",
            Self::Conflict => "Есть конфликтующие версии",
            Self::Removed => "Доступ к пространству удалён",
            Self::Failed => "Ошибка проверки истории · отправка остановлена",
        }
    }
}

pub(crate) struct SpaceSync {
    worker: Option<Worker>,
    next: Instant,
    attempts: u32,
    pub status: Status,
    incomplete: bool,
    stopped: bool,
}

impl Default for SpaceSync {
    fn default() -> Self {
        Self {
            worker: None,
            next: Instant::now(),
            attempts: 0,
            status: Status::Saved,
            incomplete: false,
            stopped: false,
        }
    }
}

impl SpaceSync {
    pub fn retry(&mut self) {
        self.next = Instant::now();
    }

    pub fn saved(&mut self) {
        self.status = Status::Saved;
        self.retry();
    }

    pub fn needs_tick(&self, editing: bool) -> bool {
        !editing
            && !self.stopped
            && match &self.worker {
                Some(worker) => worker.finished.load(Ordering::Acquire),
                None => Instant::now() >= self.next,
            }
    }

    /// Returns an error suitable for the application notice. Never polls a
    /// downloaded batch into an active draft: its eventual local event must use
    /// the causal state from which the user started editing.
    pub fn tick(&mut self, vault: &mut Vault, editing: bool) -> Option<String> {
        let profile = vault.profile.as_ref()?;
        let state = profile.sync.as_ref()?;
        if state.fault.is_some() {
            self.status = Status::Failed;
            self.stopped = true;
            return None;
        }
        if editing {
            return None;
        }
        if let Some(worker) = &self.worker {
            if !worker.finished.load(Ordering::Acquire) {
                return None;
            }
            let reply = worker
                .reply
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .unwrap_or(Reply::Uploaded(Err(TransportError::Unavailable)));
            self.worker = None;
            match reply {
                Reply::Downloaded(Ok(batch)) => {
                    self.incomplete = batch.incomplete;
                    if let Err(error) = vault.receive_sync(batch) {
                        return self.failed(vault, error.to_string());
                    }
                    let replay = match vault.sync_replay() {
                        Ok(Some(replay)) => replay,
                        Ok(None) => return None,
                        Err(error) => return self.failed(vault, error.to_string()),
                    };
                    if replay.role.is_none() {
                        self.status = Status::Removed;
                        self.next = Instant::now() + Duration::from_secs(30);
                        return None;
                    }
                    if !replay.ready || self.incomplete {
                        self.status = Status::Incomplete;
                        self.next = Instant::now() + Duration::from_secs(2);
                        return None;
                    }
                    let profile = vault.profile.as_ref().unwrap();
                    let state = profile.sync.as_ref().unwrap();
                    let plan = match sync::publication(state, &replay) {
                        Ok(plan) => plan,
                        Err(error) => return self.transport_error(error),
                    };
                    let namespace = state.namespace;
                    let endpoint = profile.provider.base_url.clone();
                    let bucket = profile.provider.bucket.clone();
                    let access = zeroize::Zeroizing::new(profile.provider.access_key.clone());
                    let secret = zeroize::Zeroizing::new(profile.provider.secret_key.clone());
                    self.start(move |cancel| {
                        Reply::Uploaded((|| {
                            let mut storage = Storj::new(&endpoint, &bucket, &access, &secret)?;
                            sync::upload(&mut storage, namespace, &plan, &cancel)
                        })())
                    });
                }
                Reply::Uploaded(Ok(hashes)) => {
                    if let Err(error) = vault.acknowledge_sync(&hashes, true) {
                        return self.failed(vault, error.to_string());
                    }
                    self.attempts = 0;
                    self.next = Instant::now() + Duration::from_secs(15);
                    self.status = match vault.sync_replay() {
                        Ok(Some(replay))
                            if vault
                                .profile
                                .as_ref()
                                .and_then(|p| p.sync.as_ref())
                                .is_some_and(|state| {
                                    sync::publication(state, &replay)
                                        .is_ok_and(|plan| !plan.is_empty())
                                }) =>
                        {
                            self.next = Instant::now();
                            Status::Saved
                        }
                        Ok(Some(replay))
                            if replay.projection.conflicts().next().is_some()
                                || !replay.projection.structural_conflicts().is_empty()
                                || !replay.excluded.is_empty() =>
                        {
                            Status::Conflict
                        }
                        _ => Status::Synced,
                    };
                }
                Reply::Downloaded(Err(error)) | Reply::Uploaded(Err(error)) => {
                    return self.transport_error(error);
                }
            }
            return None;
        }
        if Instant::now() < self.next {
            return None;
        }
        let endpoint = profile.provider.base_url.clone();
        let bucket = profile.provider.bucket.clone();
        let access = zeroize::Zeroizing::new(profile.provider.access_key.clone());
        let secret = zeroize::Zeroizing::new(profile.provider.secret_key.clone());
        let namespace = state.namespace;
        let known = state.objects.keys().copied().collect();
        self.start(move |cancel| {
            Reply::Downloaded((|| {
                let mut storage = Storj::new(&endpoint, &bucket, &access, &secret)?;
                sync::download(&mut storage, namespace, &known, &cancel)
            })())
        });
        None
    }

    fn start(&mut self, run: impl FnOnce(Cancellation) -> Reply + Send + 'static) {
        let reply = Arc::new(Mutex::new(None));
        let finished = Arc::new(AtomicBool::new(false));
        let task_reply = reply.clone();
        let task_finished = finished.clone();
        let cancel = Cancellation::default();
        let task_cancel = cancel.clone();
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(task_cancel.clone())))
                    .unwrap_or(Reply::Uploaded(Err(TransportError::Unavailable)));
            if task_cancel.check().is_ok() {
                *task_reply.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
            }
            task_finished.store(true, Ordering::Release);
        });
        self.worker = Some(Worker {
            reply,
            finished,
            cancel,
        });
        self.status = Status::Syncing;
    }

    fn transport_error(&mut self, error: TransportError) -> Option<String> {
        self.attempts = self.attempts.saturating_add(1);
        let mut jitter = [0];
        let _ = getrandom::fill(&mut jitter);
        self.next = Instant::now()
            + Duration::from_secs(
                (2u64.saturating_pow(self.attempts.min(8)) + (jitter[0] % 5) as u64).min(300),
            );
        self.status = match error {
            TransportError::Verification(_) => Status::Failed,
            _ => Status::Offline,
        };
        Some(format!("Storj: {error}"))
    }

    fn failed(&mut self, vault: &Vault, message: String) -> Option<String> {
        self.status = if matches!(
            vault.sync_replay(),
            Err(crate::infra::vault::VaultError::Sync(
                Error::ControlFork
                    | Error::DeviceFork
                    | Error::CheckpointMismatch
                    | Error::AuthenticationFailed
            ))
        ) {
            Status::Failed
        } else {
            Status::Incomplete
        };
        self.next = Instant::now() + Duration::from_secs(30);
        Some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{
            entry::Entry,
            sync::{Machine, Transaction},
        },
        infra::vault::{KdfParams, ProviderConfiguration},
    };
    use secrecy::SecretString;

    fn vault(dir: &tempfile::TempDir) -> Vault {
        Vault::create_shared(
            dir.path().join("profile.lbx"),
            &SecretString::from("master".to_owned()),
            &SecretString::from("recovery".to_owned()),
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            "Alice".into(),
            "Team".into(),
            ProviderConfiguration {
                base_url: "https://gateway.storjshare.io".into(),
                bucket: "test-bucket".into(),
                access_key: "access".into(),
                secret_key: "secret".into(),
            },
        )
        .unwrap()
    }

    #[test]
    fn downloaded_history_waits_until_the_active_draft_is_finished() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(&dir);
        let profile = vault.profile.as_ref().unwrap();
        let device = profile.sync_secret();
        let mut remote = profile.sync.as_ref().unwrap().clone();
        let mut entry = Entry::new();
        entry.title = "Remote".into();
        remote.intent = Some(
            Machine::prepare(
                &remote,
                &device,
                Transaction::diff(
                    &Machine::replay(&remote, &device).unwrap().projection,
                    &[entry],
                    &[],
                )
                .unwrap(),
            )
            .unwrap(),
        );
        Machine::complete_intent(&mut remote, &device).unwrap();
        let batch = Download {
            objects: remote.objects.into_iter().map(|(h, b)| (h, b.0)).collect(),
            incomplete: true,
        };
        let mut session = SpaceSync {
            worker: Some(Worker {
                reply: Arc::new(Mutex::new(Some(Reply::Downloaded(Ok(batch))))),
                finished: Arc::new(AtomicBool::new(true)),
                cancel: Cancellation::default(),
            }),
            ..SpaceSync::default()
        };
        assert!(!session.needs_tick(true));
        assert!(session.tick(&mut vault, true).is_none());
        assert!(vault.entries.is_empty());
        assert!(session.worker.is_some());
        assert!(session.tick(&mut vault, false).is_none());
        assert_eq!(vault.entries[0].title, "Remote");
        assert_eq!(session.status, Status::Incomplete);
    }

    #[test]
    fn dropping_the_session_cancels_a_late_ciphertext_result() {
        let (release, wait) = std::sync::mpsc::channel();
        let mut session = SpaceSync::default();
        session.start(move |_| {
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            Reply::Uploaded(Ok(vec![]))
        });
        let worker = session.worker.as_ref().unwrap();
        let cancel = worker.cancel.clone();
        let result = worker.reply.clone();
        let finished = worker.finished.clone();
        drop(session);
        assert_eq!(cancel.check(), Err(TransportError::Cancelled));
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !finished.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(finished.load(Ordering::Acquire));
        assert!(result.lock().unwrap().is_none());
    }

    #[test]
    fn new_edits_saved_during_upload_remain_pending_after_old_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = vault(&dir);
        let old_plan: Vec<_> = vault
            .profile
            .as_ref()
            .unwrap()
            .sync
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .copied()
            .collect();
        let mut entry = Entry::new();
        entry.title = "Saved during upload".into();
        vault.entries.push(entry);
        vault.save_changes().unwrap();
        let mut session = SpaceSync {
            worker: Some(Worker {
                reply: Arc::new(Mutex::new(Some(Reply::Uploaded(Ok(old_plan))))),
                finished: Arc::new(AtomicBool::new(true)),
                cancel: Cancellation::default(),
            }),
            ..SpaceSync::default()
        };
        assert!(session.tick(&mut vault, false).is_none());
        assert_eq!(session.status, Status::Saved);
        assert!(session.needs_tick(false));
        assert_eq!(
            vault
                .profile
                .as_ref()
                .unwrap()
                .sync
                .as_ref()
                .unwrap()
                .outbox
                .len(),
            1
        );
    }
}
