//! Legacy public paths. Internal code uses the feature modules directly.

pub mod app {
    pub use crate::desktop::App;
}

pub mod core {
    pub use crate::access::mnemonic;
    pub use crate::records::{entry, folder, generator, history, import_plan as import, totp};
    pub use crate::settings::language;
    pub use crate::sync::protocol as sync;
    pub use crate::vault::anchor;
}

pub mod infra {
    pub use crate::platform::{autolock, clipboard, touchid};
    pub use crate::settings::config;
    pub use crate::sync::transport as sync;
    pub use crate::sync::transport::storj;
    pub use crate::vault;
    pub use crate::vault::witness;
}

pub mod ui {
    pub use crate::desktop::{App, icons, window};
}
