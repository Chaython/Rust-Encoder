mod app;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Rust Encoder")
            .with_inner_size([1040.0, 780.0])
            .with_min_inner_size([740.0, 550.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rust Encoder",
        options,
        Box::new(|_creation_context| Ok(Box::new(app::EncoderApp::new()))),
    )
}
