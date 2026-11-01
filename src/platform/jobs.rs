//! A cancellable thread owns only its task and one eventual result.

use crate::sync::transport::Cancellation;
use std::sync::{Arc, Mutex};

/// Failure to produce a result, including a panic in a transport adapter.
#[derive(Debug)]
pub(crate) struct WorkerFailed;

enum Mailbox<T> {
    Pending,
    Finished(Option<T>),
}

/// Session-owned result mailbox. Dropping it cancels subsequent task requests.
/// A request already in progress remains subject to its provider timeout.
pub(crate) struct Worker<T> {
    mailbox: Arc<Mutex<Mailbox<T>>>,
    cancel: Cancellation,
}

impl<T: Send + 'static> Worker<T> {
    pub(crate) fn spawn(task: impl FnOnce(Cancellation) -> T + Send + 'static) -> Self {
        let mailbox = Arc::new(Mutex::new(Mailbox::Pending));
        let cancel = Cancellation::default();
        let task_mailbox = mailbox.clone();
        let task_cancel = cancel.clone();

        std::thread::spawn(move || {
            let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                task(task_cancel.clone())
            }));

            let reply = reply.ok().filter(|_| task_cancel.check().is_ok());
            *task_mailbox.lock().unwrap_or_else(|p| p.into_inner()) = Mailbox::Finished(reply);
        });

        Self { mailbox, cancel }
    }

    pub(crate) fn is_finished(&self) -> bool {
        matches!(
            *self.mailbox.lock().unwrap_or_else(|p| p.into_inner()),
            Mailbox::Finished(_)
        )
    }

    /// `None` means pending; an absent result after completion is a worker fault.
    pub(crate) fn take(&mut self) -> Option<Result<T, WorkerFailed>> {
        match &mut *self.mailbox.lock().unwrap_or_else(|p| p.into_inner()) {
            Mailbox::Pending => None,
            Mailbox::Finished(result) => Some(result.take().ok_or(WorkerFailed)),
        }
    }

    #[cfg(test)]
    pub(crate) fn ready(result: T) -> Self {
        Self {
            mailbox: Arc::new(Mutex::new(Mailbox::Finished(Some(result)))),
            cancel: Cancellation::default(),
        }
    }
}

impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use crate::platform::jobs::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn pending_and_consumed_mailboxes_keep_distinct_outcomes() {
        let (release, wait) = mpsc::channel();
        let mut worker = Worker::spawn(move |_| {
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            42
        });
        assert!(!worker.is_finished());
        assert!(worker.take().is_none());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(matches!(worker.take(), Some(Ok(42))));
        assert!(worker.is_finished());
        assert!(matches!(worker.take(), Some(Err(WorkerFailed))));
    }

    #[test]
    fn dropping_a_mailbox_cancels_and_discards_a_late_result() {
        let (release, wait) = mpsc::channel();
        let worker = Worker::spawn(move |_| {
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            42
        });
        let cancel = worker.cancel.clone();
        let mailbox = worker.mailbox.clone();
        drop(worker);
        assert!(cancel.check().is_err());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);

        while matches!(*mailbox.lock().unwrap(), Mailbox::Pending) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(matches!(*mailbox.lock().unwrap(), Mailbox::Finished(None)));
    }

    #[test]
    fn provider_panic_becomes_a_worker_failure() {
        let mut worker = Worker::<()>::spawn(|_| panic!("fixture provider failure"));
        let deadline = Instant::now() + Duration::from_secs(2);

        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(matches!(worker.take(), Some(Err(WorkerFailed))));
    }
}
