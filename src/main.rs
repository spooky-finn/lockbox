fn main() {
    for link in std::env::args_os()
        .skip(1)
        .filter_map(|arg| arg.into_string().ok())
    {
        lockbox::desktop::links::receive(&link);
    }
    // Dioxus defaults to DEBUG in development, including verbose AWS SDK traces.
    let _ = dioxus::logger::init(dioxus::logger::tracing::Level::INFO);
    dioxus::LaunchBuilder::desktop()
        .with_cfg(lockbox::desktop::window::config(true))
        .launch(lockbox::desktop::App);
}
