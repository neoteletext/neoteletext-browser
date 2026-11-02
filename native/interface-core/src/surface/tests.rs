//! REQ-740 item 4: what is drawn on a screen colour reads on it.

#![allow(clippy::arithmetic_side_effects, reason = "a test's small numbers")]

use neoteletext_core::colour::ColourMap;
use neoteletext_core::consts::COLOUR_MAP_ENTRIES;

use super::*;
use crate::theme::{BANK_COLOURS, BLACK, BUTTON, LINK, SELECTED, TEXT_CONTRAST, UPDATED, YELLOW};

/// Each of a Colour Map entry's three primaries takes one of this many
/// values (EN 300 706 Table 30's twelve bits of colour).
const PRIMARY_VALUES: u8 = 16;
/// A four-bit primary in eight bits, as the engine's `rgb888` scales it.
const PRIMARY_SCALE: u8 = 17;

/// Every colour a channel can give a Colour Map entry by X/28 or M/29.
fn all_colours() -> impl Iterator<Item = Colour> {
    let primary = 0..PRIMARY_VALUES;
    primary.clone().flat_map(move |red| {
        let primary = primary.clone();
        primary.clone().flat_map(move |green| {
            primary.clone().map(move |blue| Colour { red: red * PRIMARY_SCALE, green: green * PRIMARY_SCALE, blue: blue * PRIMARY_SCALE })
        })
    })
}

/// The defaults of Table 30, as the engine holds them.
fn default_colours() -> impl Iterator<Item = Colour> {
    (0..COLOUR_MAP_ENTRIES).map(|index| {
        let [red, green, blue] = ColourMap::DEFAULT.rgb888(u8::try_from(index).unwrap());
        Colour { red, green, blue }
    })
}

fn assert_reads(screen: Colour) {
    let surface = Surface::on(screen);
    assert_eq!(surface.back, screen);
    assert!(contrast(surface.ink, screen) >= TEXT_CONTRAST, "the ink on {screen:?}: {:.2}", contrast(surface.ink, screen));
    assert!(contrast(surface.muted, screen) >= MARK_CONTRAST, "secondary text on {screen:?}: {:.2}", contrast(surface.muted, screen));
    for colour in BANK_COLOURS.into_iter().chain([SELECTED, UPDATED, LINK, YELLOW]) {
        let mark = surface.mark(colour);
        assert!(contrast(mark, screen) >= MARK_CONTRAST, "{colour:?} on {screen:?}: {:.2}", contrast(mark, screen));
    }
}

#[test]
fn every_default_of_the_colour_map_reads() {
    for screen in default_colours().filter(|&screen| screen != BLACK) {
        assert_reads(screen);
    }
}

/// A channel can redefine the entries, so the defaults are not all there is.
#[test]
fn every_colour_a_channel_can_choose_reads() {
    for screen in all_colours().filter(|&screen| screen != BLACK) {
        assert_reads(screen);
    }
}

/// A chosen magazine stands on a button's face on every colour.
#[test]
fn a_magazines_own_colour_reads_on_a_button() {
    for colour in BANK_COLOURS {
        assert!(contrast(colour, BUTTON) >= MARK_CONTRAST, "{colour:?}: {:.2}", contrast(colour, BUTTON));
    }
}

/// REQ-740 item 7, and the design's starting point: on black every colour is
/// the browser's own, each mark included.
#[test]
fn black_is_the_browsers_own() {
    assert_eq!(Surface::on(BLACK), Surface::BROWSER);
    for colour in BANK_COLOURS.into_iter().chain([SELECTED, UPDATED, LINK, YELLOW]) {
        assert_eq!(Surface::BROWSER.mark(colour), colour);
    }
}

/// WCAG 2.1's formula, against what it gives for colours worked by hand.
#[test]
fn contrast_is_the_standards_ratio() {
    const SAME: f64 = 1.0;
    const WIDEST: f64 = 21.0;
    /// The grey 0x777, in the standard's formula.
    const GREY_ON_WHITE: f64 = 4.48;
    const TOLERANCE: f64 = 0.01;
    let grey = Colour { red: 0x77, green: 0x77, blue: 0x77 };
    assert!((contrast(BLACK, WHITE) - WIDEST).abs() < TOLERANCE);
    assert!((contrast(WHITE, BLACK) - WIDEST).abs() < TOLERANCE, "either way round");
    assert!((contrast(grey, grey) - SAME).abs() < TOLERANCE);
    assert!((contrast(grey, WHITE) - GREY_ON_WHITE).abs() < TOLERANCE, "{}", contrast(grey, WHITE));
}
