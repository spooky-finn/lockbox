//! Application boundaries: domain models, native services and Dioxus presentation.
mod auth;
mod biometrics;
mod context;
mod directories;
mod editor;
mod runtime;
mod security;
mod session;
mod settings;
mod time;
mod tree;
mod ui;
mod workspace;

pub use ui::App;

#[cfg(test)]
mod tests;
