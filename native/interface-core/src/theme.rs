//! Colours, text styles and the bank colours, after the owner's mockups
//! (`pagebanks.png`, `500kultur.png`, `currentzoomingui.png`): black
//! panels, grey rules, the teletext colours at full strength, and a strong
//! blue for whatever is selected.

use crate::canvas::Style;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

const fn rgb(red: u8, green: u8, blue: u8) -> Colour {
    Colour { red, green, blue }
}

pub const BLACK: Colour = rgb(0x00, 0x00, 0x00);
pub const WHITE: Colour = rgb(0xff, 0xff, 0xff);
/// The window behind every panel, and the panels themselves.
pub const WINDOW: Colour = rgb(0x12, 0x12, 0x14);
pub const PANEL: Colour = rgb(0x00, 0x00, 0x00);
pub const RULE: Colour = rgb(0x3a, 0x3c, 0x42);
pub const RULE_SOFT: Colour = rgb(0x24, 0x26, 0x2b);
pub const MUTED: Colour = rgb(0xa8, 0xab, 0xb3);
pub const DIM: Colour = rgb(0x5c, 0x60, 0x68);
pub const SELECTED: Colour = rgb(0x1a, 0x3c, 0xff);
pub const SEGMENT_OFF: Colour = rgb(0x2c, 0x32, 0x4a);
pub const BUTTON: Colour = rgb(0x0c, 0x0c, 0x10);
/// The address field: black text on white, as in the mockups.
pub const FIELD: Colour = rgb(0xf4, 0xf4, 0xf4);
pub const FIELD_INK: Colour = rgb(0x10, 0x10, 0x10);
pub const FIELD_SELECTION: Colour = rgb(0xa8, 0xc0, 0xff);
pub const SECURE: Colour = rgb(0x10, 0x10, 0x10);
pub const INSECURE: Colour = rgb(0xd0, 0x30, 0x20);
pub const STAR_ON: Colour = rgb(0xff, 0xc8, 0x00);

// The teletext colours at full strength (EN 300 706 table 26).
pub const RED: Colour = rgb(0xff, 0x22, 0x22);
pub const GREEN: Colour = rgb(0x22, 0xe8, 0x22);
pub const YELLOW: Colour = rgb(0xff, 0xf0, 0x22);
pub const BLUE: Colour = rgb(0x22, 0x44, 0xff);
pub const MAGENTA: Colour = rgb(0xff, 0x22, 0xff);
pub const CYAN: Colour = rgb(0x22, 0xf0, 0xff);
pub const ORANGE: Colour = rgb(0xff, 0x8c, 0x1a);
pub const GREY: Colour = rgb(0x9a, 0x9a, 0x9a);

/// One colour per bank, 100 to 800, as `pagebanks.png`.
pub const BANK_COLOURS: [Colour; 8] = [RED, GREEN, YELLOW, CYAN, MAGENTA, BLUE, ORANGE, GREY];

/// Tab colours, chosen by the trusted core by index, each with the ink
/// that reads on it.
pub const TAB_COLOURS: [(Colour, Colour); 8] =
    [(BLUE, WHITE), (GREEN, BLACK), (CYAN, BLACK), (rgb(0x3a, 0x3c, 0x44), WHITE), (YELLOW, BLACK), (RED, WHITE), (MAGENTA, BLACK), (ORANGE, BLACK)];

/// Overview cells: a page the channel lacks, a page not yet received, and
/// the flash of a page just received.
pub const CELL_EMPTY: Colour = rgb(0x1c, 0x1d, 0x21);
pub const CELL_AVAILABLE: Colour = rgb(0x4a, 0x4c, 0x52);
pub const CELL_PULSE: Colour = WHITE;
/// A page that has just had a new version: amber, unlike any bank colour.
pub const UPDATED: Colour = rgb(0xff, 0xb0, 0x20);

/// `from` moved towards `to` by `amount`, 0 to 1: for highlights that
/// fade, with no transparency needed.
#[must_use]
pub fn mix(from: Colour, to: Colour, amount: f64) -> Colour {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| {
        let value = f64::from(a) + (f64::from(b) - f64::from(a)) * amount;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "between two bytes")]
        let byte = libm::round(value) as u8;
        byte
    };
    rgb(channel(from.red, to.red), channel(from.green, to.green), channel(from.blue, to.blue))
}

const fn style(size: f32, bold: bool, caps: bool) -> Style {
    Style { size, monospace: false, bold, caps }
}

pub const TAB_TITLE: Style = style(16.0, true, true);
pub const ADDRESS: Style = Style { size: 17.0, monospace: true, bold: false, caps: false };
pub const HEADING: Style = style(19.0, true, true);
pub const BANK_RANGE: Style = style(19.0, true, true);
pub const BANK_NAME: Style = style(14.0, true, true);
pub const BANK_COUNT: Style = style(13.0, false, true);
pub const COLUMN_HEAD: Style = style(13.0, false, true);
pub const ROW_NUMBER: Style = style(16.0, true, true);
pub const ROW_STATUS: Style = style(12.0, false, true);
pub const ROW_TITLE: Style = style(15.0, false, true);
pub const CONTROL: Style = style(20.0, true, true);
pub const READOUT: Style = style(22.0, true, true);
pub const OPTION: Style = style(13.0, true, true);
pub const CAPTION: Style = style(14.0, true, true);
pub const CAPTION_TITLE: Style = style(13.0, false, true);
pub const LEGEND: Style = style(12.0, false, true);
pub const AXIS: Style = style(11.0, false, true);
pub const GLYPH: Style = style(22.0, true, false);
