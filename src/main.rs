use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

fn main() {
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            Config::new()
                .with_window(
                    WindowBuilder::new()
                        .with_title("lockbox")
                        .with_inner_size(LogicalSize::new(960.0, 680.0))
                        .with_min_inner_size(LogicalSize::new(360.0, 480.0)),
                )
                .with_disable_context_menu(true)
                .with_navigation_handler(|_| false),
        )
        .launch(lockbox::app::App);
}
