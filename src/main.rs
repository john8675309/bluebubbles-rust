mod app;
mod composer;
mod display;
mod theme;
mod views;
mod widgets;

use eframe::egui;

fn main() -> eframe::Result {
    bluebubbles_linux::initialize_tls();
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--push-worker")) {
        if let Some(path) = args.next() {
            if bluebubbles_linux::push::run_worker(std::path::Path::new(&path)).is_err() {
                eprintln!("BlueBubbles notification receiver could not start. Check notification settings in the app.");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    bluebubbles_linux::push::resume_workers();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("bundled icon"),
            )
            .with_app_id("app.bluebubbles.RustLinux"),
        ..Default::default()
    };
    eframe::run_native(
        "BlueBubbles · Rust",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
