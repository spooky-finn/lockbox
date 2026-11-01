//! Copying secrets to the system clipboard and wiping them afterwards.

use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// How long a copied password or code stays on the clipboard.
pub const CLEAR_AFTER: Duration = Duration::from_secs(20);

/// The clipboard operations lockbox needs; tests use an in-memory one.
pub trait Board {
    fn get(&mut self) -> Option<Zeroizing<String>>;
    fn set(&mut self, text: &str) -> Result<(), String>;
    fn clear(&mut self);
}

/// The real clipboard. On macOS copies are marked as concealed, so clipboard
/// history tools that honour `org.nspasteboard.ConcealedType` skip them.
pub struct SystemBoard(Option<arboard::Clipboard>);

impl SystemBoard {
    pub fn new() -> Self {
        Self(arboard::Clipboard::new().ok())
    }
}

impl Default for SystemBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl Board for SystemBoard {
    fn get(&mut self) -> Option<Zeroizing<String>> {
        self.0.as_mut()?.get_text().ok().map(Zeroizing::new)
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        let clipboard = self
            .0
            .as_mut()
            .ok_or_else(|| "clipboard is unavailable".to_owned())?;
        #[cfg(target_os = "macos")]
        let result = {
            use arboard::SetExtApple;
            clipboard.set().exclude_from_history().text(text)
        };
        #[cfg(not(target_os = "macos"))]
        let result = clipboard.set_text(text);
        result.map_err(|error| error.to_string())
    }

    fn clear(&mut self) {
        if let Some(clipboard) = self.0.as_mut() {
            let _ = clipboard.clear();
        }
    }
}

/// Remembers only a hash of what was copied, never the text itself.
pub struct Guard {
    board: Box<dyn Board>,
    pending: Option<([u8; 32], Instant)>,
}

impl Guard {
    pub fn new(board: Box<dyn Board>) -> Self {
        Self {
            board,
            pending: None,
        }
    }

    pub fn copy(&mut self, text: &str, now: Instant) -> Result<(), String> {
        self.board.set(text)?;
        self.pending = Some((digest(text), now + CLEAR_AFTER));
        Ok(())
    }

    /// Time until the clipboard is wiped, if lockbox still owns it.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.pending
            .map(|(_, deadline)| deadline.saturating_duration_since(now))
    }

    pub fn tick(&mut self, now: Instant) {
        if self.remaining(now).is_some_and(|left| left.is_zero()) {
            self.clear_now();
        }
    }

    /// Wipes the clipboard, unless the user has copied something else since.
    pub fn clear_now(&mut self) {
        let Some((copied, _)) = self.pending.take() else {
            return;
        };

        if self
            .board
            .get()
            .is_some_and(|current| digest(&current) == copied)
        {
            self.board.clear();
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.clear_now();
    }
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

#[cfg(test)]
#[path = "tests/clipboard.rs"]
pub(crate) mod tests;
