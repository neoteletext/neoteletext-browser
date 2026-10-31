//! REQ-1686: what the page area shows of a redirect.

#![allow(clippy::arithmetic_side_effects, reason = "a test's positions")]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::HEADLINE;
use crate::browser::fixture::{HEIGHT, WIDTH, browser};
use crate::browser::{Browser, MAX_ADDRESS, StripState};
use crate::geometry::Rect;
use crate::test_canvas::Recorder;

/// The smallest window the interface allows (`interface/src/window.rs`).
const SMALLEST_WIDTH: f64 = 1000.0;
const SMALLEST_HEIGHT: f64 = 640.0;

fn redirecting(browser: &mut Browser, address: &str) {
    browser.set_strip(StripState { address: "https://moved.invalid/".into(), redirecting: address.into(), ..StripState::default() });
}

fn drawn(browser: &mut Browser, width: f64, height: f64) -> Recorder {
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, width, height, 0);
    canvas
}

/// The headline's rectangle, and the lines drawn after it.
fn notice(canvas: &Recorder) -> (Rect, Vec<(Rect, String)>) {
    let at = canvas.texts.iter().position(|(_, text)| text == HEADLINE).expect("the notice's first line");
    let below = canvas.texts.get(at + 1..).unwrap_or_default().iter().take_while(|(rect, _)| rect.y0 > canvas.texts[at].0.y0).cloned().collect();
    (canvas.texts[at].0, below)
}

// REQ-1686 item 2: the pages go, whether wall or page, and the address
// starts on the next line.
#[test]
fn the_page_area_is_cleared_and_says_where_to() {
    let (mut browser, _) = browser();
    assert!(!drawn(&mut browser, WIDTH, HEIGHT).pages.is_empty(), "the wall, before");
    redirecting(&mut browser, "https://new.invalid/teletext/");
    let canvas = drawn(&mut browser, WIDTH, HEIGHT);
    assert!(canvas.pages.is_empty(), "nothing of the channel being left");
    let (headline, lines) = notice(&canvas);
    assert_eq!(lines.iter().map(|(_, line)| line.as_str()).collect::<Vec<_>>(), ["https://new.invalid/teletext/"]);
    assert!(lines[0].0.y0 >= headline.y1, "on the next line");
    assert!(canvas.texts.iter().any(|(_, text)| text.contains("moved.invalid")), "REQ-1686 item 6: the strip stays, with the old address");
}

// REQ-1686 item 3 with its longest address, at the smallest window: every
// character, in lines inside the page area, each centred across it and the
// block centred down it.
#[test]
fn the_longest_address_fits_whole_in_the_smallest_window() {
    let (mut browser, _) = browser();
    let start = "https://new.invalid/";
    let address = format!("{start}{}", "a".repeat(MAX_ADDRESS - start.len()));
    assert_eq!(address.len(), MAX_ADDRESS);
    redirecting(&mut browser, &address);
    let canvas = drawn(&mut browser, SMALLEST_WIDTH, SMALLEST_HEIGHT);
    let area = browser.redirecting_area;
    let (headline, lines) = notice(&canvas);
    assert!(lines.len() > 1, "wrapped");
    assert_eq!(lines.iter().map(|(_, line)| line.as_str()).collect::<String>(), address, "never shortened");
    let middle = f64::midpoint(area.x0, area.x1);
    for (rect, line) in lines.iter().take(lines.len() - 1) {
        assert!(rect.x0 >= area.x0, "{line}");
        let width = 2.0 * (middle - rect.x0);
        assert!(rect.x0 + width <= area.x1, "inside the page area: {rect:?} in {area:?}");
    }
    let bottom = lines.last().unwrap().0.y1;
    assert!(bottom <= area.y1 && headline.y0 >= area.y0, "it fits: {} to {bottom} in {area:?}", headline.y0);
    let above = headline.y0 - area.y0;
    let below = area.y1 - bottom;
    assert!((above - below).abs() <= 1.0, "centred down it: {above} above, {below} below");
}

// REQ-1686 item 6: it has no buttons, and the pages it covers take no press.
#[test]
fn a_press_on_the_notice_does_nothing() {
    let (mut browser, _) = browser();
    redirecting(&mut browser, "https://new.invalid/");
    let mut canvas = drawn(&mut browser, WIDTH, HEIGHT);
    let area = browser.redirecting_area;
    let _ = browser.take_outputs();
    browser.pointer_moved(f64::midpoint(area.x0, area.x1), f64::midpoint(area.y0, area.y1), 1);
    browser.pointer_pressed(&mut canvas, false, 1);
    assert_eq!(browser.take_outputs(), []);
}

// REQ-1686 item 4: the trusted core takes it away, and the pages return.
#[test]
fn the_notice_goes_when_the_strip_says_so() {
    let (mut browser, _) = browser();
    redirecting(&mut browser, "https://new.invalid/");
    drawn(&mut browser, WIDTH, HEIGHT);
    browser.set_strip(StripState::default());
    let canvas = drawn(&mut browser, WIDTH, HEIGHT);
    assert!(!canvas.pages.is_empty(), "the pages return");
    assert!(canvas.texts.iter().all(|(_, text)| text != HEADLINE));
}
