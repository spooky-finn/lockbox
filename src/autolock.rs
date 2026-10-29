//! Locking an unattended vault: after a period without input, after the
//! computer slept for longer than that, and when the screen is locked.

use std::time::{Duration, Instant, SystemTime};

/// Choices offered in the toolbar, in minutes; 0 means never.
pub const CHOICES: [u32; 5] = [1, 5, 15, 30, 0];
pub const DEFAULT_MINUTES: u32 = 5;
/// How long before locking the window warns that unsaved edits will be lost.
pub const WARNING: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Idle(Duration),
    Warning(Duration),
    Lock,
}

pub struct AutoLock {
    timeout: Option<Duration>,
    activity: Instant,
    /// Wall-clock time of the last activity: `Instant` stops while the Mac
    /// sleeps, the wall clock does not.
    activity_wall: SystemTime,
}

pub fn timeout(minutes: u32) -> Option<Duration> {
    (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60))
}

impl AutoLock {
    pub fn new(minutes: u32, now: Instant, wall: SystemTime) -> Self {
        Self {
            timeout: timeout(minutes),
            activity: now,
            activity_wall: wall,
        }
    }

    pub fn set_minutes(&mut self, minutes: u32, now: Instant, wall: SystemTime) {
        self.timeout = timeout(minutes);
        self.activity(now, wall);
    }

    pub fn activity(&mut self, now: Instant, wall: SystemTime) {
        self.activity = now;
        self.activity_wall = wall;
    }

    pub fn check(&self, now: Instant, wall: SystemTime) -> State {
        let Some(timeout) = self.timeout else {
            return State::Off;
        };
        // A clock set backwards counts as no time passed; forwards locks early, which is safe.
        let idle = now
            .saturating_duration_since(self.activity)
            .max(wall.duration_since(self.activity_wall).unwrap_or_default());
        match timeout.checked_sub(idle) {
            None | Some(Duration::ZERO) => State::Lock,
            Some(left) if left <= WARNING => State::Warning(left),
            Some(left) => State::Idle(left),
        }
    }
}

/// Whether the screen was locked since the last check; on other systems never.
#[derive(Default)]
pub struct ScreenWatch {
    #[cfg(target_os = "macos")]
    inner: Option<screen::ScreenLock>,
}

impl ScreenWatch {
    /// Starts listening; `wake` asks the window to run its checks.
    pub fn start(wake: impl Fn() + 'static) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                inner: Some(screen::ScreenLock::watch(wake)),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = wake;
            Self {}
        }
    }

    pub fn take(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.inner.as_ref().is_some_and(screen::ScreenLock::take)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

/// Raised by macOS when the screen locks.
#[cfg(target_os = "macos")]
mod screen {
    use std::{
        ptr::NonNull,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::ProtocolObject};
    use objc2_foundation::{
        NSDistributedNotificationCenter, NSNotification, NSObjectProtocol, NSString,
    };

    pub struct ScreenLock {
        locked: Arc<AtomicBool>,
        _observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }

    impl ScreenLock {
        pub fn watch(on_lock: impl Fn() + 'static) -> Self {
            let locked = Arc::new(AtomicBool::new(false));
            let flag = locked.clone();
            let block = RcBlock::new(move |_: NonNull<NSNotification>| {
                flag.store(true, Ordering::SeqCst);
                on_lock();
            });
            let observer = unsafe {
                NSDistributedNotificationCenter::defaultCenter()
                    .addObserverForName_object_queue_usingBlock(
                        Some(&NSString::from_str("com.apple.screenIsLocked")),
                        None,
                        None,
                        &block,
                    )
            };
            Self {
                locked,
                _observer: observer,
            }
        }

        /// True once after each screen lock.
        pub fn take(&self) -> bool {
            self.locked.swap(false, Ordering::SeqCst)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn start(minutes: u32) -> (AutoLock, Instant, SystemTime) {
        let (now, wall) = (Instant::now(), SystemTime::now());
        (AutoLock::new(minutes, now, wall), now, wall)
    }

    #[test]
    fn locks_exactly_when_the_time_is_up() {
        let (lock, now, wall) = start(5);
        assert_eq!(
            lock.check(now + 4 * MINUTE, wall + 4 * MINUTE),
            State::Idle(MINUTE)
        );
        assert_eq!(lock.check(now + 5 * MINUTE, wall + 5 * MINUTE), State::Lock);
    }

    #[test]
    fn warns_during_the_last_thirty_seconds() {
        let (lock, now, wall) = start(1);
        let at = Duration::from_secs(40);
        assert_eq!(
            lock.check(now + at, wall + at),
            State::Warning(Duration::from_secs(20))
        );
    }

    #[test]
    fn any_input_restarts_the_countdown() {
        let (mut lock, now, wall) = start(5);
        lock.activity(now + 4 * MINUTE, wall + 4 * MINUTE);
        assert_eq!(
            lock.check(now + 8 * MINUTE, wall + 8 * MINUTE),
            State::Idle(MINUTE)
        );
    }

    #[test]
    fn sleeping_longer_than_the_timeout_locks_on_wake() {
        let (lock, now, wall) = start(5);
        // Instant barely moved (it stops during sleep), the wall clock moved 8 hours.
        let after_sleep = wall + 8 * 60 * MINUTE;
        assert_eq!(
            lock.check(now + Duration::from_secs(1), after_sleep),
            State::Lock
        );
    }

    #[test]
    fn clock_set_backwards_does_not_lock() {
        let (lock, now, wall) = start(5);
        let earlier = wall - 60 * MINUTE;
        assert_eq!(lock.check(now + MINUTE, earlier), State::Idle(4 * MINUTE));
    }

    #[test]
    fn never_means_never() {
        let (lock, now, wall) = start(0);
        assert_eq!(
            lock.check(now + 600 * MINUTE, wall + 600 * MINUTE),
            State::Off
        );
    }

    #[test]
    fn changing_the_timeout_restarts_it() {
        let (mut lock, now, wall) = start(5);
        lock.set_minutes(1, now + 4 * MINUTE, wall + 4 * MINUTE);
        assert_eq!(
            lock.check(
                now + 4 * MINUTE + Duration::from_secs(10),
                wall + 4 * MINUTE + Duration::from_secs(10)
            ),
            State::Idle(Duration::from_secs(50))
        );
    }
}
