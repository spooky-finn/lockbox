fn main() {
    // Dioxus defaults to DEBUG in development, including verbose AWS SDK traces.
    let _ = dioxus::logger::init(dioxus::logger::tracing::Level::INFO);
    dioxus::LaunchBuilder::desktop()
        .with_cfg(lockbox::desktop::window::config(true))
        .launch(lockbox::desktop::App);
}
