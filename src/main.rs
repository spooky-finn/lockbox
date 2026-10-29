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
        Box::new(|_| Ok(Box::new(lockbox::app::LockboxApp::new()))),
    )
}
