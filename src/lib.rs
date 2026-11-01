//! Local password manager organized by feature.

pub mod access;
pub mod browser;
pub mod desktop;
pub mod platform;
pub mod records;
pub mod settings;
pub mod sharing;
pub mod sync;
pub mod vault;

mod compatibility;
pub use compatibility::{app, core, infra, ui};

// Established short imports remain available to library consumers.
pub use desktop::{App, icons};
pub use platform::{autolock, clipboard, touchid};
pub use records::{entry, folder, generator, totp};
pub use settings::config;
