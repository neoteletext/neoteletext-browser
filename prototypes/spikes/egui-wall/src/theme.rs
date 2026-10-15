//! Colours from the prototype's dark theme (`src/shared/tokens.css`).

use eframe::egui::Color32;

pub const RAISED: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x25);
pub const FIELD: Color32 = Color32::from_rgb(0x0f, 0x11, 0x15);
pub const WELL: Color32 = Color32::from_rgb(0x0c, 0x0e, 0x12);
pub const FRAME: Color32 = Color32::from_rgb(0x11, 0x13, 0x18);
pub const INK: Color32 = Color32::from_rgb(0xe6, 0xe9, 0xef);
pub const MUTED: Color32 = Color32::from_rgb(0x93, 0x99, 0xa6);
pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x2f, 0x38);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x20, 0x24, 0x2b);
pub const ACCENT: Color32 = Color32::from_rgb(0x8f, 0xa1, 0xff);
/// The four corners of the channel mosaic stand-in, as its CSS gradient.
pub const MOSAIC: [Color32; 4] = [
    Color32::from_rgb(0xff, 0x4d, 0x4d),
    Color32::from_rgb(0xff, 0xd8, 0x4d),
    Color32::from_rgb(0x4d, 0xd2, 0xff),
    Color32::from_rgb(0x4d, 0xff, 0x88),
];
