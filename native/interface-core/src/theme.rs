//! Colours, text styles and the bank colours, after the owner's mockups
//! (`pagebanks.png`, `500kultur.png`, `currentzoomingui.png`): black
//! panels, grey rules, the teletext colours at full strength, and a strong
//! blue for whatever is selected.

use crate::canvas::Style;

/// An opaque colour, eight bits a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour {
    /// The red channel.
    pub red: u8,
    /// The green channel.
    pub green: u8,
    /// The blue channel.
    pub blue: u8,
}

const fn rgb(red: u8, green: u8, blue: u8) -> Colour {
    Colour { red, green, blue }
}

/// A link's underline, drawn over the page by the browser (`hyperlinks.md`
/// section 9.2): near the web's link blue, light enough to show on black,
/// and edged white or black to show on every teletext background.
pub const LINK: Colour = rgb(0x3d, 0x8b, 0xff);
/// A link's destination and the trusted strip's questions, as ink on the
/// address field.
pub const LINK_INK: Colour = rgb(0x1a, 0x4f, 0xd8);
/// Black: the tab strip, and ink on light tab colours.
pub const BLACK: Colour = rgb(0x00, 0x00, 0x00);
/// White: headings, labels, and ink on dark tab colours.
pub const WHITE: Colour = rgb(0xff, 0xff, 0xff);
/// The window behind every panel, and the panels themselves.
pub const WINDOW: Colour = rgb(0x12, 0x12, 0x14);
/// The inside of each panel.
pub const PANEL: Colour = rgb(0x00, 0x00, 0x00);
/// Panel outlines, button borders and the scroll bar.
pub const RULE: Colour = rgb(0x3a, 0x3c, 0x42);
/// The lighter lines between rows and around unselected banks.
pub const RULE_SOFT: Colour = rgb(0x24, 0x26, 0x2b);
/// A surface raised above the window, as a dialog is: the tab strip and
/// its overflow panel while the panel is open (decision 102).
pub const RAISED: Colour = rgb(0x2a, 0x2d, 0x35);
/// Secondary text: page titles in captions, column heads, the legend.
pub const MUTED: Colour = rgb(0xa8, 0xab, 0xb3);
/// A control that cannot be used now, such as Back with no history.
pub const DIM: Colour = rgb(0x5c, 0x60, 0x68);
/// Whatever is selected: the chosen row, the page size bar's filled part.
pub const SELECTED: Colour = rgb(0x1a, 0x3c, 0xff);
/// The page size bar's unfilled segments.
pub const SEGMENT_OFF: Colour = rgb(0x2c, 0x32, 0x4a);
/// The face of a button.
pub const BUTTON: Colour = rgb(0x0c, 0x0c, 0x10);
/// The address field: black text on white, as in the mockups.
pub const FIELD: Colour = rgb(0xf4, 0xf4, 0xf4);
/// Text in the address field.
pub const FIELD_INK: Colour = rgb(0x10, 0x10, 0x10);
/// Selected text in the address field.
pub const FIELD_SELECTION: Colour = rgb(0xa8, 0xc0, 0xff);
/// The padlock of a channel fetched over an encrypted connection.
pub const SECURE: Colour = rgb(0x10, 0x10, 0x10);
/// The warning on a channel fetched without encryption.
pub const INSECURE: Colour = rgb(0xd0, 0x30, 0x20);
/// The bookmark star when the channel is bookmarked.
pub const STAR_ON: Colour = rgb(0xff, 0xc8, 0x00);

/// Teletext red at full strength (EN 300 706 table 26).
pub const RED: Colour = rgb(0xff, 0x22, 0x22);
/// Teletext green at full strength (EN 300 706 table 26).
pub const GREEN: Colour = rgb(0x22, 0xe8, 0x22);
/// Teletext yellow at full strength (EN 300 706 table 26); also loaded pages in the overview.
pub const YELLOW: Colour = rgb(0xff, 0xf0, 0x22);
/// Teletext blue at full strength (EN 300 706 table 26).
pub const BLUE: Colour = rgb(0x22, 0x44, 0xff);
/// Teletext magenta at full strength (EN 300 706 table 26).
pub const MAGENTA: Colour = rgb(0xff, 0x22, 0xff);
/// Teletext cyan at full strength (EN 300 706 table 26).
pub const CYAN: Colour = rgb(0x22, 0xf0, 0xff);
/// Orange: not a teletext colour; the seventh bank's.
pub const ORANGE: Colour = rgb(0xff, 0x8c, 0x1a);
/// Grey: not a teletext colour; the eighth bank's.
pub const GREY: Colour = rgb(0x9a, 0x9a, 0x9a);

/// One colour per bank, 100 to 800, as `pagebanks.png`.
pub const BANK_COLOURS: [Colour; 8] = [RED, GREEN, YELLOW, CYAN, MAGENTA, BLUE, ORANGE, GREY];

/// Tab colours, chosen by the trusted core by index, each with the ink
/// that reads on it.
pub const TAB_COLOURS: [(Colour, Colour); 8] =
    [(BLUE, WHITE), (GREEN, BLACK), (CYAN, BLACK), (rgb(0x3a, 0x3c, 0x44), WHITE), (YELLOW, BLACK), (RED, WHITE), (MAGENTA, BLACK), (ORANGE, BLACK)];

/// An overview cell for a page the channel lacks.
pub const CELL_EMPTY: Colour = rgb(0x1c, 0x1d, 0x21);
/// An overview cell for a page the channel has but has not yet sent.
pub const CELL_AVAILABLE: Colour = rgb(0x4a, 0x4c, 0x52);
/// The flash of an overview cell whose page has just arrived.
pub const CELL_PULSE: Colour = WHITE;
/// A page that has just had a new version: amber, unlike any bank colour.
pub const UPDATED: Colour = rgb(0xff, 0xb0, 0x20);
/// Typing into a page: the cursor and the strip's "Keys go to" badge
/// (`interaction.md` decision 6), an amber no teletext colour is (EN 300
/// 706 Table 26's eight are full on or off).
pub const TYPING: Colour = rgb(0xff, 0xbf, 0x40);
/// A private tab's word and mark (requirement 20.8): a colour no other part of
/// the strip uses.
pub const PRIVATE: Colour = rgb(0xb0, 0x6c, 0xff);

/// The ink that reads on `background`: black on a light colour, white on a
/// dark one, by its relative luminance (ITU-R BT.709's weights, as WCAG
/// 2.1 section 1.4.3 measures it, without the gamma step, which is enough
/// to tell the bank colours apart).
#[must_use]
pub fn ink_on(background: Colour) -> Colour {
    /// Half of full brightness: lighter than this takes black.
    const LIGHT: f64 = 127.5;
    let luminance = 0.2126 * f64::from(background.red) + 0.7152 * f64::from(background.green) + 0.0722 * f64::from(background.blue);
    if luminance > LIGHT { BLACK } else { WHITE }
}

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

/// A tab's title.
pub const TAB_TITLE: Style = style(16.0, true, true);
/// The address field, monospaced so the caret lands between characters.
pub const ADDRESS: Style = Style { size: 17.0, monospace: true, bold: false, caps: false };
/// A panel's heading.
pub const HEADING: Style = style(19.0, true, true);
/// A bank's page range, such as 100–199.
pub const BANK_RANGE: Style = style(19.0, true, true);
/// A bank's name.
pub const BANK_NAME: Style = style(14.0, true, true);
/// How many pages a bank holds.
pub const BANK_COUNT: Style = style(13.0, false, true);
/// The page list's column heads.
pub const COLUMN_HEAD: Style = style(13.0, false, true);
/// A page number in a list row.
pub const ROW_NUMBER: Style = style(16.0, true, true);
/// Short status words: a list row's state, the address field's security.
pub const ROW_STATUS: Style = style(12.0, false, true);
/// A page title in a list row.
pub const ROW_TITLE: Style = style(15.0, false, true);
/// The page size control's label.
pub const CONTROL: Style = style(20.0, true, true);
/// The page size control's readout.
pub const READOUT: Style = style(22.0, true, true);
/// The page number under a page on the wall.
pub const CAPTION: Style = style(14.0, true, true);
/// The page title beside a caption's page number.
pub const CAPTION_TITLE: Style = style(13.0, false, true);
/// The overview's legend and counts.
pub const LEGEND: Style = style(12.0, false, true);
/// The overview's bank numbers down its side.
pub const AXIS: Style = style(11.0, false, true);
/// Symbols drawn as text on buttons: close and new tab.
pub const GLYPH: Style = style(22.0, true, false);
