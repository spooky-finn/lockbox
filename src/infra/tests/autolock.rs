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
