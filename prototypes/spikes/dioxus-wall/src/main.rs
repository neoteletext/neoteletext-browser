//! The React prototype's tab, in Dioxus with its native renderer: Blitz lays
//! out and draws the HTML and CSS itself, in Rust, with no web engine and no
//! JavaScript. The header holds the transform picker, the thumbnail width
//! slider and its readout; below it is the wall of teletext pages.
//!
//! Blitz has no range input, and Dioxus native reports pointer positions
//! only in window coordinates, so the slider is drawn from elements at a
//! fixed place in the header and computes its value from that place.

mod pages;
mod size;
mod tab;

use dioxus::native::{Config, LogicalSize, WindowAttributes};

const WINDOW_TITLE: &str = "NeoTeletext: Dioxus page wall";
const WINDOW_WIDTH: f64 = 1280.0;
const WINDOW_HEIGHT: f64 = 820.0;

fn main() {
    let window = WindowAttributes::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
    // Dioxus 0.7.10's general launch builder warns that no renderer is
    // enabled when only the native one is, so the native launch is called.
    dioxus::native::launch_cfg(tab::TabApp, vec![], vec![Box::new(Config::new().with_window_attributes(window))]);
}
