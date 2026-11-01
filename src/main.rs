fn main() {
    // Dioxus defaults to DEBUG in development, including verbose AWS SDK traces.
    let _ = dioxus::logger::init(dioxus::logger::tracing::Level::INFO);
    dioxus::LaunchBuilder::desktop()
        .with_cfg(lockbox::ui::window::config(true))
        .launch(lockbox::ui::App);
}
