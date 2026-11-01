//! Framework-independent application state and workflows.

pub(crate) mod auth;
pub(crate) mod biometrics;
pub(crate) mod cloud_setup;
pub(crate) mod directories;
pub(crate) mod editor;
pub(crate) mod security;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod sharing;
pub(crate) mod space_sync;
pub(crate) mod time;
pub(crate) mod tree;
pub(crate) mod workspace;

#[cfg(test)]
#[path = "tests/support.rs"]
pub(crate) mod test_support;

// Preserve the desktop entry point.
pub use crate::ui::App;
