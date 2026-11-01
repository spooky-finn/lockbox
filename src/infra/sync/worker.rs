//! A cancellable thread owns only its task and one eventual result.

use super::Cancellation;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Failure to produce a result, including a panic in a transport adapter.
#[derive(Debug)]
pub(crate) struct WorkerFailed;

/// Session-owned result mailbox. Dropping it cancels subsequent task requests.
/// A request already in progress remains subject to its provider timeout.
pub(crate) struct Worker<T> {
    result: Arc<Mutex<Option<T>>>,
    finished: Arc<AtomicBool>,
    cancel: Cancellation,
}

impl<T: Send + 'static> Worker<T> {
    pub(crate) fn spawn(task: impl FnOnce(Cancellation) -> T + Send + 'static) -> Self {
        let result = Arc::new(Mutex::new(None));
        let finished = Arc::new(AtomicBool::new(false));
        let cancel = Cancellation::default();
        let task_result = result.clone();
        let task_finished = finished.clone();
        let task_cancel = cancel.clone();

        std::thread::spawn(move || {
            let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                task(task_cancel.clone())
            }));

            if task_cancel.check().is_ok()
                && let Ok(reply) = reply
            {
                *task_result.lock().unwrap_or_else(|p| p.into_inner()) = Some(reply);
            }
            task_finished.store(true, Ordering::Release);
        });

        Self {
            result,
            finished,
            cancel,
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// `None` means pending; an absent result after completion is a worker fault.
    pub(crate) fn take(&mut self) -> Option<Result<T, WorkerFailed>> {
        self.is_finished().then(|| {
            self.result
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .ok_or(WorkerFailed)
        })
    }

    #[cfg(test)]
    pub(crate) fn ready(result: T) -> Self {
        Self {
            result: Arc::new(Mutex::new(Some(result))),
            finished: Arc::new(AtomicBool::new(true)),
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
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn dropping_a_mailbox_cancels_and_discards_a_late_result() {
        let (release, wait) = mpsc::channel();
        let worker = Worker::spawn(move |_| {
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            42
        });
        let cancel = worker.cancel.clone();
        let result = worker.result.clone();
        let finished = worker.finished.clone();
        drop(worker);
        assert!(cancel.check().is_err());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);

        while !finished.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(finished.load(Ordering::Acquire));
        assert!(result.lock().unwrap().is_none());
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
