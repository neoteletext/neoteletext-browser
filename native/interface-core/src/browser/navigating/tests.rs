//! REQ-7164: what the page area shows while a tab goes to another channel.

#![allow(clippy::arithmetic_side_effects, reason = "a test's positions")]

use alloc::vec::Vec;

use super::{WORDS, WORDS_STYLE};
use crate::browser::fixture::{HEIGHT, WIDTH, browser, channel};
use crate::browser::{Browser, Output, StripState};
use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::test_canvas::Recorder;
use crate::theme::{BLACK, Colour};

/// The address the strip shows while the tab goes there.
const NEW_ADDRESS: &str = "https://new-channel.invalid/";
/// A colour none of the browser's own parts draw in, as every page's.
const SCREEN: Colour = Colour { red: 0x77, green: 0x00, blue: 0x77 };
/// The first line of a redirect's notice: `redirecting.rs` keeps it private.
const HEADLINE: &str = "Redirecting to:";
/// The new channel's number, other than the fixture's 7.
const NEW_CHANNEL: u32 = 9;
/// How far the words may sit from the exact middle: the recorder measures
/// in whole points.
const TOLERANCE: f64 = 0.5;
/// Where a redirect's notice says the tab is going.
const ELSEWHERE: &str = "https://elsewhere.invalid/";

fn navigating(browser: &mut Browser, on: bool) {
    browser.set_strip(StripState { address: NEW_ADDRESS.into(), identity: "new-channel.invalid".into(), navigating: on, ..StripState::default() });
}

/// A strip whose redirect's notice names `destination` (none when empty),
/// with or without the words of a change of channel.
fn redirect(destination: &str, navigating: bool) -> StripState {
    StripState { address: NEW_ADDRESS.into(), redirecting: destination.into(), navigating, ..StripState::default() }
}

fn drawn(browser: &mut Browser) -> Recorder {
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    canvas
}

fn words_drawn(canvas: &Recorder) -> Vec<Rect> {
    canvas.texts.iter().filter(|(_, text)| text == WORDS).map(|&(rect, _)| rect).collect()
}

// REQ-7164 item 1 (TODO-7206): only the strip that starts the words clears
// a page area showing a channel, and so is drawn as it arrives: not a strip
// that says nothing of it, nor one that says so again or stops saying so.
#[test]
fn only_the_strip_that_starts_the_words_clears_the_page_area() {
    let (mut browser, _) = browser();
    let says = |on: bool| StripState { address: NEW_ADDRESS.into(), navigating: on, ..StripState::default() };
    assert!(!browser.drawn_as_it_arrives(&says(false)), "a strip that says nothing of it");
    assert!(browser.drawn_as_it_arrives(&says(true)), "the strip that starts the words");
    navigating(&mut browser, true);
    assert!(!browser.drawn_as_it_arrives(&says(true)), "the page area is cleared already");
    assert!(!browser.drawn_as_it_arrives(&says(false)), "the strip that ends the words");
    navigating(&mut browser, false);
    assert!(browser.drawn_as_it_arrives(&says(true)), "the next change of channel");
}

// REQ-1686 item 5 (TODO-7222): the strip that starts a redirect's notice
// clears a page area showing a channel too, and so is drawn as it arrives.
// Before, only the words of REQ-7164 did, and the page being left stayed
// drawn until the next paced frame. A strip that says the redirect again,
// or brings one during the words while the area is already cleared
// (DEC-7166 item 2), clears nothing more.
#[test]
fn the_strip_that_starts_a_redirects_notice_clears_the_page_area() {
    let (mut browser, _) = browser();
    assert!(browser.drawn_as_it_arrives(&redirect(ELSEWHERE, false)), "the strip that starts the notice");
    browser.set_strip(redirect(ELSEWHERE, false));
    assert!(!browser.drawn_as_it_arrives(&redirect(ELSEWHERE, false)), "the page area is cleared already");
    browser.set_strip(redirect("", true));
    assert!(!browser.drawn_as_it_arrives(&redirect(ELSEWHERE, true)), "a redirect during the words finds it cleared");
    browser.set_strip(redirect("", false));
    assert!(browser.drawn_as_it_arrives(&redirect(ELSEWHERE, false)), "the next redirect");
}

// REQ-1686 (TODO-7232): the notice shows for exactly one second, which the
// trusted core measures, so the strip that ends it is drawn as it arrives,
// whether the words of DEC-7166 item 1 follow it or nothing does. Before,
// that strip counted as clearing nothing more and waited for the next paced
// frame, which on a page that animates lengthened the notice by up to the
// pace. The strip after it, which ends the words, is paced as before.
#[test]
fn the_strip_that_ends_a_redirects_notice_is_drawn_as_it_arrives() {
    let (mut browser, _) = browser();
    browser.set_strip(redirect(ELSEWHERE, false));
    assert!(browser.drawn_as_it_arrives(&redirect("", true)), "the words that follow the notice");
    assert!(browser.drawn_as_it_arrives(&redirect("", false)), "the notice ending with nothing after it");
    browser.set_strip(redirect("", true));
    assert!(!browser.drawn_as_it_arrives(&redirect("", true)), "the words said again");
    assert!(!browser.drawn_as_it_arrives(&redirect("", false)), "the strip that ends the words");
}

// REQ-7164 items 1 and 2: the pages go, the page area is filled, and the
// words are drawn once, centred both ways in it.
#[test]
fn the_page_area_is_cleared_and_says_navigating_to_new_channel_in_its_middle() {
    let (mut browser, _) = browser();
    assert!(!drawn(&mut browser).pages.is_empty(), "the wall, before");
    navigating(&mut browser, true);
    let mut canvas = drawn(&mut browser);
    assert!(canvas.pages.is_empty(), "nothing of a channel is painted: {:?}", canvas.pages);
    let area = browser.navigating_area;
    assert!(area.width() > 0.0 && area.height() > 0.0, "{area:?}");
    assert!(canvas.fills.contains(&(area, BLACK)), "the page area is filled");
    let words = words_drawn(&canvas);
    assert_eq!(words.len(), 1, "{words:?}");
    let width = canvas.text_width(WORDS, WORDS_STYLE);
    let height = canvas.line_height(WORDS_STYLE);
    let (left, right) = (words[0].x0 - area.x0, area.x1 - (words[0].x0 + width));
    let (above, below) = (words[0].y0 - area.y0, area.y1 - (words[0].y0 + height));
    assert!(left > 0.0 && (left - right).abs() <= TOLERANCE, "centred across: {left} left, {right} right");
    assert!(above > 0.0 && (above - below).abs() <= TOLERANCE, "centred down: {above} above, {below} below");
}

// REQ-7164 item 5: the strip stays, with the address the tab goes to.
#[test]
fn the_strip_is_still_drawn_with_its_address() {
    let (mut browser, _) = browser();
    navigating(&mut browser, true);
    let canvas = drawn(&mut browser);
    assert!(canvas.texts.iter().any(|(_, text)| text.contains("new-channel.invalid")), "{:?}", canvas.texts);
}

// REQ-7164 item 3: while the words show, the interface still tells the
// trusted core which of the new channel's pages are on screen, so that the
// page the words wait for is made ready.
#[test]
fn the_new_channels_visible_pages_are_reported_while_the_words_show() {
    let (mut browser, _) = browser();
    let _ = browser.take_outputs();
    navigating(&mut browser, true);
    let mut next = channel();
    next.channel = NEW_CHANNEL;
    browser.set_channel(next);
    drawn(&mut browser);
    let outputs = browser.take_outputs();
    let visible = outputs.iter().find_map(|output| if let Output::Visible(visible) = output { Some(visible) } else { None });
    let visible = visible.unwrap_or_else(|| panic!("no visible pages told: {outputs:?}"));
    assert!(!visible.is_empty() && visible.iter().all(|key| key.channel == NEW_CHANNEL), "{visible:?}");
}

// REQ-7164 item 5: the words have no buttons, and the pages under them take
// no press.
#[test]
fn a_press_on_the_words_does_nothing() {
    let (mut browser, _) = browser();
    navigating(&mut browser, true);
    let mut canvas = drawn(&mut browser);
    let area = browser.navigating_area;
    let _ = browser.take_outputs();
    let selected = browser.selected;
    // On a page of the new channel laid out under the words, which would
    // otherwise be zoomed to.
    let (_, page) = browser.wall.shown().next().expect("the new channel's pages are laid out");
    assert!(area.contains(crate::browser::fixture::middle(page)));
    browser.pointer_moved(f64::midpoint(page.x0, page.x1), f64::midpoint(page.y0, page.y1), 1);
    browser.pointer_pressed(&mut canvas, false, 1);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), []);
    assert_eq!(browser.selected, selected, "no page under the words is chosen");
    assert_eq!(browser.wall.zoomed(), None, "nor zoomed to");
}

// REQ-7164 item 3: the trusted core ends the notice, and the pages return.
#[test]
fn the_pages_return_when_the_strip_stops_saying_so() {
    let (mut browser, _) = browser();
    navigating(&mut browser, true);
    drawn(&mut browser);
    navigating(&mut browser, false);
    let canvas = drawn(&mut browser);
    assert!(!canvas.pages.is_empty(), "the pages return");
    assert!(words_drawn(&canvas).is_empty(), "the words are gone");
    assert_eq!(browser.navigating_area, Rect::ZERO);
}

// DEC-7166 item 2: while a redirect's notice and the words both apply, the
// redirect's notice is drawn and not the words.
#[test]
fn a_redirects_notice_is_drawn_in_place_of_the_words() {
    let (mut browser, _) = browser();
    browser.set_strip(StripState { address: NEW_ADDRESS.into(), redirecting: "https://elsewhere.invalid/".into(), navigating: true, ..StripState::default() });
    let canvas = drawn(&mut browser);
    assert!(canvas.texts.iter().any(|(_, text)| text == HEADLINE), "the redirect's notice");
    assert!(words_drawn(&canvas).is_empty(), "not the words");
    assert!(canvas.pages.is_empty(), "nothing of a channel is painted");
}

// REQ-7164 item 1 with REQ-740 item 6: the window's colour below the strip
// is not taken from a page of either channel while the words show.
#[test]
fn the_screen_colour_is_not_a_pages_while_the_words_show() {
    let (mut browser, _) = browser();
    browser.toggle_wall(0);
    let mut before = Recorder::new();
    before.screen = Some(SCREEN);
    browser.draw(&mut before, WIDTH, HEIGHT, 0);
    assert!(before.fills.iter().any(|&(_, colour)| colour == SCREEN), "the page alone gives its colour, before");
    navigating(&mut browser, true);
    let mut canvas = Recorder::new();
    canvas.screen = Some(SCREEN);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    assert!(canvas.fills.iter().all(|&(_, colour)| colour != SCREEN), "no fill in the page's colour");
    assert_eq!(words_drawn(&canvas).len(), 1);
}
