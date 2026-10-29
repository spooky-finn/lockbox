use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("lockbox")
            .with_inner_size([820.0, 520.0])
            .with_min_inner_size([560.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "lockbox",
        options,
        Box::new(|creation| {
            let mut app = lockbox::app::LockboxApp::new();
            app.watch_screen_lock(&creation.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
