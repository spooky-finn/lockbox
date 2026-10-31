//! Framework-independent application state and workflows.
pub(crate) mod auth;
pub(crate) mod biometrics;
pub(crate) mod directories;
pub(crate) mod editor;
pub(crate) mod security;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod time;
pub(crate) mod tree;
pub(crate) mod workspace;

#[cfg(test)]
pub(crate) mod tests;

// Preserve the desktop entry point.
pub use crate::ui::App;
