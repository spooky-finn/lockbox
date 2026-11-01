//! Local password manager, organized by responsibility.

pub mod app;
pub mod core;
pub mod infra;
pub mod ui;

// Preserve the public API while implementations live in explicit layers.
pub use core::{entry, folder, generator, totp};
pub use infra::{autolock, clipboard, config, touchid, vault};
pub use ui::{App, icons};
