//! Desktop shell, session coordination and reusable presentation.

pub mod browser;
pub(crate) mod context;
pub(crate) mod i18n;
pub mod icons;
mod root;
pub(crate) mod runtime;
pub(crate) mod security;
pub(crate) mod session;
mod sync_review;
pub(crate) mod time;
pub(crate) mod widgets;
pub mod window;
pub(crate) mod workspace;
pub(crate) mod workspace_view;

pub use root::App;

#[cfg(test)]
#[path = "tests/workflow_support.rs"]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "tests/view_support.rs"]
pub(crate) mod view_test_support;
