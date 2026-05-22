mod app;

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([1040.0, 680.0])
            .with_title("Framer"),
        ..Default::default()
    };

    eframe::run_native(
        "Framer",
        options,
        Box::new(|_cc| Ok(Box::<app::FramerApp>::default())),
    )
}
