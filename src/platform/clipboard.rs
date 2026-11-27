//! Copying secrets to the system clipboard and wiping them afterwards.

use std::time::{Duration, Instant, SystemTime};

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

/// The real clipboard. Copies stay off other devices and out of clipboard
/// history where the platform allows: the 20 s wipe only reaches this one.
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
        {
            // arboard clears the pasteboard itself, which would drop the
            // current-host-only option. A live arboard handle means the
            // general pasteboard exists.
            let _ = clipboard;
            write_secret(&objc2_app_kit::NSPasteboard::generalPasteboard(), text)
        }
        #[cfg(not(target_os = "macos"))]
        set_excluded(clipboard, text).map_err(|error| error.to_string())
    }

    fn clear(&mut self) {
        if let Some(clipboard) = self.0.as_mut() {
            let _ = clipboard.clear();
        }
    }
}

/// Keeps copies out of Windows clipboard history, cloud clipboard and
/// clipboard monitors.
#[cfg(windows)]
fn set_excluded(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    use arboard::SetExtWindows;
    clipboard
        .set()
        .exclude_from_history()
        .exclude_from_cloud()
        .exclude_from_monitoring()
        .text(text)
}

/// Adds the password-manager hint that KDE and GNOME clipboard managers skip.
#[cfg(not(any(target_os = "macos", windows)))]
fn set_excluded(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    use arboard::SetExtLinux;
    clipboard.set().exclude_from_history().text(text)
}

/// nspasteboard.org markers: clipboard history tools skip concealed and
/// transient items.
#[cfg(target_os = "macos")]
const SECRET_MARKERS: [&str; 2] = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
];

/// Writes `text` so it stays on this Mac: Universal Clipboard (Handoff) would
/// hand it to nearby devices, where the wipe cannot reach.
#[cfg(target_os = "macos")]
fn write_secret(pasteboard: &objc2_app_kit::NSPasteboard, text: &str) -> Result<(), String> {
    use objc2::{runtime::NSObjectProtocol, sel};
    use objc2_app_kit::{NSPasteboardContentsOptions, NSPasteboardTypeString};
    use objc2_foundation::{NSString, ns_string};

    // Both calls clear the old contents; only the first keeps them local.
    if pasteboard.respondsToSelector(sel!(prepareForNewContentsWithOptions:)) {
        pasteboard.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
    } else {
        pasteboard.clearContents();
    }
    let text = NSString::from_str(text);
    if !pasteboard.setString_forType(&text, unsafe { NSPasteboardTypeString }) {
        return Err("NSPasteboard setString:forType: returned false".to_owned());
    }
    for marker in SECRET_MARKERS {
        pasteboard.setString_forType(ns_string!(""), &NSString::from_str(marker));
    }
    Ok(())
}

/// Remembers only a hash of what was copied, never the text itself.
pub struct Guard {
    board: Box<dyn Board>,
    /// Copied text's hash, and when to wipe it by the monotonic and wall clocks.
    pending: Option<([u8; 32], Instant, SystemTime)>,
    wall: fn() -> SystemTime,
}

impl Guard {
    pub fn new(board: Box<dyn Board>) -> Self {
        Self::with_wall_clock(board, SystemTime::now)
    }

    /// `Instant` stops while macOS sleeps, so the wall clock also bounds how
    /// long a secret survives on the clipboard across sleep.
    pub(crate) fn with_wall_clock(board: Box<dyn Board>, wall: fn() -> SystemTime) -> Self {
        Self {
            board,
            pending: None,
            wall,
        }
    }

    pub fn copy(&mut self, text: &str, now: Instant) -> Result<(), String> {
        self.board.set(text)?;
        self.pending = Some((digest(text), now + CLEAR_AFTER, (self.wall)() + CLEAR_AFTER));
        Ok(())
    }

    /// Time until the clipboard is wiped, if lockbox still owns it.
    /// A passed wall-clock deadline (sleep) ends it at once.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        let (_, deadline, wall_deadline) = self.pending?;
        if (self.wall)() >= wall_deadline {
            return Some(Duration::ZERO);
        }
        Some(deadline.saturating_duration_since(now))
    }

    pub fn tick(&mut self, now: Instant) {
        if self.remaining(now).is_some_and(|left| left.is_zero()) {
            self.clear_now();
        }
    }

    /// Wipes the clipboard, unless the user has copied something else since.
    pub fn clear_now(&mut self) {
        let Some((copied, ..)) = self.pending.take() else {
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
