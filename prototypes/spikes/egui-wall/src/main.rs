//! The React prototype's tab in egui, drawn natively in Rust: the header
//! with the thumbnail width slider, its readout and the transform picker,
//! and the wall of teletext pages below. The flat, tilt and fan transforms
//! are drawn as transformed page meshes, the tilt with the same perspective
//! projection CSS uses, and animate as the prototype's CSS transitions do.

mod app;
mod pages;
mod size;
mod theme;
mod wall;

const WINDOW_TITLE: &str = "NeoTeletext: egui page wall";
const WINDOW_SIZE: [f32; 2] = [1280.0, 820.0];

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title(WINDOW_TITLE).with_inner_size(WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(WINDOW_TITLE, options, Box::new(|creation| Ok(Box::new(app::TabApp::new(&creation.egui_ctx)))))
}
