use crate::platform::clipboard::*;
use std::{cell::RefCell, rc::Rc};

/// In-memory clipboard whose contents the test can inspect and change.
#[derive(Clone, Default)]
pub(crate) struct FakeBoard(pub(crate) Rc<RefCell<Option<String>>>);

impl Board for FakeBoard {
    fn get(&mut self) -> Option<Zeroizing<String>> {
        self.0.borrow().clone().map(Zeroizing::new)
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        *self.0.borrow_mut() = Some(text.to_owned());
        Ok(())
    }

    fn clear(&mut self) {
        *self.0.borrow_mut() = None;
    }
}

fn guard() -> (Guard, FakeBoard) {
    let board = FakeBoard::default();
    (Guard::new(Box::new(board.clone())), board)
}

#[test]
fn copied_secret_is_wiped_after_the_timeout() {
    let (mut guard, board) = guard();
    let start = Instant::now();
    guard.copy("s3cret", start).unwrap();
    guard.tick(start + CLEAR_AFTER - Duration::from_millis(1));
    assert_eq!(board.0.borrow().as_deref(), Some("s3cret"));
    guard.tick(start + CLEAR_AFTER);
    assert_eq!(*board.0.borrow(), None);
    assert_eq!(guard.remaining(start), None);
}

#[test]
fn something_the_user_copied_later_is_left_alone() {
    let (mut guard, board) = guard();
    let start = Instant::now();
    guard.copy("s3cret", start).unwrap();
    *board.0.borrow_mut() = Some("my own text".into());
    guard.tick(start + CLEAR_AFTER);
    assert_eq!(board.0.borrow().as_deref(), Some("my own text"));
}

#[test]
fn clearing_now_wipes_before_the_timeout() {
    let (mut guard, board) = guard();
    guard.copy("s3cret", Instant::now()).unwrap();
    guard.clear_now();
    assert_eq!(*board.0.borrow(), None);
}

#[test]
fn dropping_the_guard_wipes_the_clipboard() {
    let (mut guard, board) = guard();
    guard.copy("s3cret", Instant::now()).unwrap();
    drop(guard);
    assert_eq!(*board.0.borrow(), None);
}

#[test]
fn a_new_copy_restarts_the_countdown() {
    let (mut guard, board) = guard();
    let start = Instant::now();
    guard.copy("first", start).unwrap();
    let later = start + Duration::from_secs(15);
    guard.copy("second", later).unwrap();
    guard.tick(start + CLEAR_AFTER);
    assert_eq!(board.0.borrow().as_deref(), Some("second"));
    assert_eq!(
        guard.remaining(start + CLEAR_AFTER),
        Some(Duration::from_secs(15))
    );
}

thread_local! {
    static WALL: std::cell::Cell<std::time::SystemTime> =
        const { std::cell::Cell::new(std::time::SystemTime::UNIX_EPOCH) };
}

/// macOS sleep stops `Instant` but not the wall clock: a secret copied before
/// sleep must be gone right after wake, not 20 awake seconds later.
#[test]
fn copied_secret_is_wiped_after_the_timeout_even_across_sleep() {
    let board = FakeBoard::default();
    let mut guard =
        Guard::with_wall_clock(Box::new(board.clone()), || WALL.with(|wall| wall.get()));
    let start = Instant::now();
    guard.copy("s3cret", start).unwrap();

    // An hour asleep: the monotonic clock moved one second, the wall an hour.
    WALL.with(|wall| wall.set(wall.get() + Duration::from_secs(3600)));
    guard.tick(start + Duration::from_secs(1));
    assert_eq!(*board.0.borrow(), None);
    assert_eq!(guard.remaining(start), None);
}
