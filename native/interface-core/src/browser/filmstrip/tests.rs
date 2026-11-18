//! The tab filmstrip's tests, one or more for each item of REQ-6640
//! (DEC-6637): each says the item it verifies.

use alloc::format;
use alloc::vec::Vec;

use super::draw::{Layout, MARGIN};
use super::*;
use crate::browser::fixture::{HEIGHT, WIDTH, channel, middle};
use crate::browser::{Frame, Key, Modifiers, TabInfo};
use crate::canvas::REGIS_HEIGHT_PER_WIDTH;
use crate::test_canvas::{Painted, Recorder};
use crate::theme::{BLACK, WHITE};

/// The fixture channel's number: the first tab's.
const FIRST: u32 = 7;

/// A browser of `count` tabs, numbered from [`FIRST`], the first active
/// and showing the fixture's channel, drawn once.
fn browser_of(count: usize) -> (Browser, Recorder) {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
    browser.set_tabs(tabs(count), 0);
    browser.set_channel(channel());
    browser.wall.toggle_single(0);
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    (browser, canvas)
}

fn tabs(count: usize) -> Vec<TabInfo> {
    (0..count)
        .map(|index| TabInfo {
            title: format!("Channel {index}"),
            colour: 0,
            channel: FIRST.saturating_add(u32::try_from(index).unwrap()),
            muted: false,
            private: false,
            address: format!("https://tab{index}.invalid/"),
        })
        .collect()
}

/// Rests the pointer on the first tab at `now` and waits it out: the
/// filmstrip opens and has slid out by the time it is drawn again.
fn open(browser: &mut Browser, canvas: &mut Recorder, now: u64) -> u64 {
    let tab = browser.regions.tabs[0].tab;
    browser.pointer_moved(middle(tab).x, middle(tab).y, now);
    let opened = now.saturating_add(FILMSTRIP_DELAY_MILLISECONDS);
    assert!(browser.settle(opened), "it opens");
    let out = opened.saturating_add(FILMSTRIP_OPEN_MILLISECONDS);
    browser.draw(canvas, WIDTH, HEIGHT, out);
    out
}

/// Moves the pointer to the middle of `rect` at `now`.
fn point_at(browser: &mut Browser, rect: Rect, now: u64) {
    browser.pointer_moved(middle(rect).x, middle(rect).y, now);
}

/// REQ-6640 item 1: a rest of 300 milliseconds on the tabs opens the
/// filmstrip, sliding out from under the strip; a pointer that crosses
/// the strip in less opens nothing.
#[test]
fn a_rest_on_the_tabs_opens_the_filmstrip_and_crossing_does_not() {
    let (mut browser, mut canvas) = browser_of(3);
    let tab = browser.regions.tabs[1].tab;
    let field = browser.regions.field;
    point_at(&mut browser, tab, 0);
    assert_eq!(browser.frame_interval(0), Some(FILMSTRIP_DELAY_MILLISECONDS), "it waits for the rest");
    point_at(&mut browser, field, 100);
    browser.settle(1_000);
    assert!(!browser.filmstrip_open(), "crossed, not rested");
    point_at(&mut browser, tab, 2_000);
    browser.settle(2_000 + FILMSTRIP_DELAY_MILLISECONDS - 1);
    assert!(!browser.filmstrip_open(), "not yet");
    assert!(browser.settle(2_000 + FILMSTRIP_DELAY_MILLISECONDS) && browser.filmstrip_open(), "rested: open");
    let halfway = 2_000 + FILMSTRIP_DELAY_MILLISECONDS + FILMSTRIP_OPEN_MILLISECONDS / 2;
    assert_eq!(browser.frame_interval(halfway), Some(crate::browser::frames::SMOOTH_FRAME_MILLISECONDS), "it slides out");
    browser.draw(&mut canvas, WIDTH, HEIGHT, halfway);
    let part = browser.filmstrip.panel;
    browser.draw(&mut canvas, WIDTH, HEIGHT, halfway + FILMSTRIP_OPEN_MILLISECONDS);
    let whole = browser.filmstrip.panel;
    assert!(part.y0 == STRIP_HEIGHT && whole.y0 == STRIP_HEIGHT && part.y1 < whole.y1, "joined to the strip, sliding down: {part:?} {whole:?}");
    assert_eq!(browser.filmstrip.pictures.len(), 3, "a picture for every tab");
}

/// REQ-6640 items 1 and 10: a press on the overflow count opens the
/// filmstrip at once and a second shuts it; the overflow panel is gone,
/// every tab region being on the strip.
#[test]
fn the_count_opens_the_filmstrip_at_once_and_the_panel_is_gone() {
    let (mut browser, mut canvas) = browser_of(30);
    let count = browser.tab_strip.count;
    assert!(count != Rect::ZERO, "the tabs overflow");
    point_at(&mut browser, count, 0);
    browser.pointer_pressed(&mut canvas, false, 0);
    browser.pointer_released();
    assert!(browser.filmstrip_open(), "open at once");
    browser.draw(&mut canvas, WIDTH, HEIGHT, FILMSTRIP_OPEN_MILLISECONDS);
    assert!(browser.regions.tabs.iter().all(|region| region.tab.y1 <= STRIP_HEIGHT), "no tab wraps into a panel under the strip");
    assert!(browser.filmstrip.pictures.iter().any(|picture| picture.index == 0), "the pictures instead");
    point_at(&mut browser, count, 1_000);
    browser.pointer_pressed(&mut canvas, false, 1_000);
    assert!(!browser.filmstrip_open(), "a second press shuts it");
}

/// REQ-6640 item 3: a still is taken of the tab left, from the page the
/// wall showed, when the viewer leaves it, and of the active tab when the
/// filmstrip opens; each picture then draws its tab's still, and a tab
/// never shown has the blank stand-in. A closed tab's still is forgotten.
#[test]
fn stills_are_taken_when_a_tab_is_left_and_when_the_filmstrip_opens() {
    let (mut browser, mut canvas) = browser_of(3);
    let shown = browser.wall.shown().next().map(|(key, _)| key).unwrap();
    browser.set_tabs(tabs(3), 1);
    assert!(canvas.taken.is_empty(), "taken at the next frame, from what the canvas holds then");
    browser.draw(&mut canvas, WIDTH, HEIGHT, 10);
    assert_eq!(canvas.taken, [(FIRST, shown)], "the tab left, of the page it showed");
    let mut other = channel();
    other.channel = FIRST + 1;
    browser.set_channel(other);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 20);
    canvas.painted.clear();
    open(&mut browser, &mut canvas, 100);
    assert_eq!(canvas.taken.last().map(|&(still, _)| still), Some(FIRST + 1), "the active tab's, as it opens");
    let stills: Vec<u32> = canvas.painted.iter().filter_map(|painted| if let Painted::Still(still, _) = painted { Some(*still) } else { None }).collect();
    assert_eq!(stills, [FIRST, FIRST + 1], "the two tabs seen draw their stills");
    let never = browser.filmstrip.pictures.iter().find(|picture| picture.index == 2).unwrap();
    assert!(canvas.fills.iter().any(|&(rect, colour)| colour == BLACK && never.rect.contains(middle(rect))), "the tab never shown: a blank page");
    browser.set_tabs(tabs(2), 1);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 2_000);
    browser.set_tabs(tabs(1), 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 2_100);
    assert!(!canvas.stills.iter().any(|&(still, _)| still == FIRST + 1), "a closed tab's still is forgotten");
}

/// REQ-6640 item 3: a page known to be a ReGIS or NAPLPS picture whose
/// image is not held stands in black, in the picture's shape.
#[test]
fn a_picture_page_not_yet_drawn_stands_in_black_in_its_shape() {
    let (mut browser, mut canvas) = browser_of(2);
    let shown = browser.wall.shown().next().map(|(key, _)| key).unwrap();
    browser.set_regis(shown, 1, 1);
    canvas.has_images = false;
    browser.set_tabs(tabs(2), 1);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 10);
    canvas.fills.clear();
    open(&mut browser, &mut canvas, 100);
    let left = browser.filmstrip.pictures.iter().find(|picture| picture.index == 0).unwrap();
    let inside = |rect: Rect| left.rect.contains(Point::new(rect.x0, rect.y0)) && left.rect.contains(Point::new(rect.x1 - 0.5, rect.y1 - 0.5));
    let black = canvas.fills.iter().find(|&&(rect, colour)| colour == BLACK && inside(rect)).map(|&(rect, _)| rect).unwrap();
    assert!((black.height() / black.width() - REGIS_HEIGHT_PER_WIDTH).abs() < 1e-6, "the picture's shape: {black:?}");
}

/// REQ-6640 item 4: each picture is captioned above it with its tab's
/// title and the page number of its still.
#[test]
fn each_picture_is_captioned_with_its_title_and_page_number() {
    let (mut browser, mut canvas) = browser_of(2);
    let shown = browser.wall.shown().next().map(|(key, _)| key).unwrap();
    canvas.texts.clear();
    open(&mut browser, &mut canvas, 0);
    for title in ["Channel 0", "Channel 1"] {
        assert!(canvas.texts.iter().any(|(_, text)| text == title), "{title}");
    }
    assert!(canvas.coloured_text(&format!("P{:03X}", shown.page)).is_some(), "the active tab's page number");
    let picture = browser.filmstrip.pictures[0].rect;
    let caption = canvas.texts.iter().find(|(rect, text)| text == "Channel 0" && rect.y0 >= STRIP_HEIGHT).map(|&(rect, _)| rect).unwrap();
    assert!(middle(caption).y < picture.y0 + 30.0, "above the picture: {caption:?} {picture:?}");
}

/// REQ-6640 items 5 and 6: each picture is about a fifth of the window
/// wide; the wheel slides the row with easing, reaching its target, and
/// stops at either end with the end picture brought all the way across
/// and still whole in the window; nothing beneath scrolls.
#[test]
fn the_wheel_slides_the_row_with_easing_to_bounds_at_both_ends() {
    let (mut browser, mut canvas) = browser_of(12);
    let now = open(&mut browser, &mut canvas, 0);
    let first = browser.filmstrip.pictures[0].rect;
    assert!((first.width() - WIDTH / 5.0).abs() < WIDTH / 25.0, "about a fifth: {}", first.width());
    point_at(&mut browser, first, now);
    let scrolled = browser.list_scroll;
    let layout = Layout::of(WIDTH, 12, browser.wall.shape().height_per_width());
    // Towards the viewer: later pictures, the row to the left.
    assert!(browser.wheel_lines(1.0, now));
    let (at, target) = browser.filmstrip_slide_at(now + FILMSTRIP_SLIDE_MILLISECONDS / 4);
    assert!((target + layout.step).abs() < 1e-3 && at > target && at < 0.0, "eased on its way: {at} to {target}");
    assert!((browser.filmstrip_slide_at(now + FILMSTRIP_SLIDE_MILLISECONDS).0 - target).abs() < 1e-3, "it reaches it");
    for notch in 0..40 {
        browser.wheel_lines(1.0, now + notch);
    }
    let (least, most) = browser.filmstrip_bounds();
    let end = now + 1_000;
    assert!((browser.filmstrip_slide_at(end).0 - least).abs() < 1e-3, "stopped at the left end");
    browser.draw(&mut canvas, WIDTH, HEIGHT, end);
    let final_picture = browser.filmstrip.pictures.iter().find(|picture| picture.index == 11).unwrap().rect;
    assert!((final_picture.x0 - MARGIN).abs() < 1e-3, "the last picture all the way across, whole: {final_picture:?}");
    browser.pointer = middle(final_picture);
    browser.scroll_by(-1.0e6, end);
    let far = end + 1_000;
    assert!((browser.filmstrip_slide_at(far).0 - most).abs() < 1e-3, "stopped at the right end");
    browser.draw(&mut canvas, WIDTH, HEIGHT, far);
    let first = browser.filmstrip.pictures.iter().find(|picture| picture.index == 0).unwrap().rect;
    assert!((first.x1 - (WIDTH - MARGIN)).abs() < 1e-3, "the first picture all the way across, whole: {first:?}");
    assert!((browser.list_scroll - scrolled).abs() < f64::EPSILON, "nothing beneath scrolled");
}

/// REQ-6640 item 6: opening places the row so the active tab's picture is
/// whole in view, and outlines it.
#[test]
fn the_active_tabs_picture_is_in_view_and_outlined() {
    let (mut browser, mut canvas) = browser_of(12);
    browser.set_tabs(tabs(12), 10);
    canvas.fills.clear();
    open(&mut browser, &mut canvas, 0);
    let active = browser.filmstrip.pictures.iter().find(|picture| picture.index == 10).unwrap().rect;
    assert!(active.x0 >= 0.0 && active.x1 <= WIDTH, "whole in view: {active:?}");
    assert!(canvas.fills.iter().any(|&(rect, colour)| colour == WHITE && rect.x1 <= active.x0 + 0.5 && rect.y1 > active.y0 + 1.0), "outlined at its edge");
}

/// REQ-6640 items 7 and 8: a press on a picture selects its tab and
/// closes the filmstrip, and the strip then slides to show the tab chosen.
#[test]
fn choosing_a_picture_selects_its_tab_closes_and_slides_the_strip() {
    let (mut browser, mut canvas) = browser_of(30);
    let now = open(&mut browser, &mut canvas, 0);
    let first = browser.filmstrip.pictures[0].rect;
    point_at(&mut browser, first, now);
    browser.wheel_lines(25.0, now);
    browser.draw(&mut canvas, WIDTH, HEIGHT, now + 2_000);
    let picture = *browser.filmstrip.pictures.iter().find(|picture| picture.index == 25).unwrap();
    point_at(&mut browser, Rect::new(picture.rect.x0, picture.rect.y1 - 40.0, picture.rect.x0 + 40.0, picture.rect.y1), now + 2_000);
    let _ = browser.take_outputs();
    browser.pointer_pressed(&mut canvas, false, now + 2_000);
    assert_eq!(browser.take_outputs(), [Output::SelectTab(25)], "its tab is selected");
    assert!(!browser.filmstrip_open(), "the filmstrip closes");
    browser.set_tabs(tabs(30), 25);
    browser.draw(&mut canvas, WIDTH, HEIGHT, now + 2_100);
    assert!(browser.filmstrip.strip.moving(now + 2_150), "the strip slides with easing");
    browser.draw(&mut canvas, WIDTH, HEIGHT, now + 5_000);
    assert!(browser.filmstrip.panel == Rect::ZERO, "slid back under the strip");
    let tab = browser.regions.tabs.iter().find(|region| region.index == 25).unwrap().tab;
    assert!((tab.width() - crate::browser::tab_strip::TAB_WIDTH).abs() < 1e-9, "the tab chosen whole on the strip");
}

/// REQ-6640 item 7: resting on a picture shows its close and mute
/// buttons, which close and mute its tab and keep the filmstrip open; a
/// press where the close button would be on a picture not rested on
/// chooses it.
#[test]
fn a_rested_picture_shows_close_and_mute_buttons() {
    let (mut browser, mut canvas) = browser_of(3);
    let now = open(&mut browser, &mut canvas, 0);
    let picture = browser.filmstrip.pictures[1];
    point_at(&mut browser, picture.rect, now);
    canvas.texts.clear();
    browser.draw(&mut canvas, WIDTH, HEIGHT, now);
    assert!(canvas.texts.iter().any(|(rect, text)| text == "\u{00D7}" && picture.close.contains(middle(*rect))), "its close button");
    let _ = browser.take_outputs();
    point_at(&mut browser, picture.close, now);
    browser.pointer_pressed(&mut canvas, false, now);
    assert_eq!(browser.take_outputs(), [Output::CloseTab(1)]);
    point_at(&mut browser, picture.speaker, now);
    browser.pointer_pressed(&mut canvas, false, now);
    assert_eq!(browser.take_outputs(), [Output::MuteTab(1)]);
    assert!(browser.filmstrip_open(), "still open");
    browser.filmstrip.hovered = None;
    browser.pointer = middle(picture.close);
    browser.pointer_pressed(&mut canvas, false, now);
    assert_eq!(browser.take_outputs(), [Output::SelectTab(1)], "not rested on: the picture is chosen");
}

/// REQ-6640 item 9: the filmstrip closes 300 milliseconds after the
/// pointer has left both the strip and it, not if the pointer comes back
/// first, and on Escape.
#[test]
fn it_closes_after_the_pointer_leaves_and_on_escape() {
    let (mut browser, mut canvas) = browser_of(3);
    let now = open(&mut browser, &mut canvas, 0);
    let below = Rect::new(600.0, 780.0, 620.0, 800.0);
    point_at(&mut browser, below, now);
    assert!(browser.frame_interval(now + 100).is_some(), "the wait to close");
    let first = browser.filmstrip.pictures[0].rect;
    point_at(&mut browser, first, now + 200);
    browser.settle(now + 1_000);
    assert!(browser.filmstrip_open(), "came back in time: open");
    point_at(&mut browser, below, now + 2_000);
    browser.settle(now + 2_000 + FILMSTRIP_CLOSE_MILLISECONDS - 1);
    assert!(browser.filmstrip_open(), "not yet");
    assert!(browser.settle(now + 2_000 + FILMSTRIP_CLOSE_MILLISECONDS) && !browser.filmstrip_open(), "closed");
    browser.draw(&mut canvas, WIDTH, HEIGHT, now + 5_000);
    let again = open(&mut browser, &mut canvas, 10_000);
    assert!(browser.key(Key::Escape, Modifiers::default(), again));
    assert!(!browser.filmstrip_open(), "Escape closes it");
}

/// REQ-6640 item 3: at most `MAX_STILLS` stills are held; past that the
/// one taken longest ago is forgotten.
#[test]
fn at_most_the_cap_of_stills_is_held() {
    let count = MAX_STILLS + 3;
    let (mut browser, mut canvas) = browser_of(count);
    for index in 0..count {
        let mut shown = channel();
        shown.channel = FIRST.saturating_add(u32::try_from(index).unwrap());
        browser.set_channel(shown);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 10);
        browser.set_tabs(tabs(count), (index + 1) % count);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 20);
    }
    assert_eq!(browser.filmstrip.stills.len(), MAX_STILLS);
    assert_eq!(canvas.stills.len(), MAX_STILLS, "the canvas holds as many");
    assert!(browser.filmstrip.still(FIRST).is_none(), "the first taken has gone");
}

/// REQ-996 with REQ-6640: while the filmstrip is open the strip's tips do
/// not cover it; a picture rested on shows its tab's tip, the title over
/// the address.
#[test]
fn a_rested_picture_shows_its_tabs_tip_and_the_strips_do_not_cover_it() {
    let (mut browser, mut canvas) = browser_of(3);
    let now = open(&mut browser, &mut canvas, 0);
    let picture = browser.filmstrip.pictures[1].rect;
    point_at(&mut browser, Rect::new(picture.x0, picture.y1 - 20.0, picture.x0 + 20.0, picture.y1), now);
    browser.draw(&mut canvas, WIDTH, HEIGHT, now);
    browser.settle(now + 1_000);
    assert_eq!(browser.tip_shown(), Some("Channel 1\nhttps://tab1.invalid/"));
}

/// REQ-6640 item 7: the picture the pointer rests on is the one under it
/// after the row slides beneath a still pointer, and only it shows its
/// buttons. Seen on a private display: the picture rested on before the
/// slide kept its buttons.
#[test]
fn the_rested_picture_follows_the_slide_under_a_still_pointer() {
    let (mut browser, mut canvas) = browser_of(8);
    let now = open(&mut browser, &mut canvas, 0);
    let first = browser.filmstrip.pictures[0].rect;
    point_at(&mut browser, first, now);
    assert_eq!(browser.filmstrip.hovered, Some(0));
    browser.wheel_lines(1.0, now);
    browser.draw(&mut canvas, WIDTH, HEIGHT, now.saturating_add(1_000));
    assert_eq!(browser.filmstrip.hovered, Some(1), "the next picture has slid under the pointer");
}

/// REQ-6640 item 6 with REQ-6632: over the open filmstrip the one decision
/// of whose the wheel is says the filmstrip's, and below it the page
/// area's again; the pointer shows no wheel indicator there.
#[test]
fn the_wheels_owner_over_the_open_filmstrip_is_the_filmstrip() {
    use crate::browser::pointer::wheel_owner::WheelOwner;
    let (mut browser, mut canvas) = browser_of(3);
    let now = open(&mut browser, &mut canvas, 0);
    let first = browser.filmstrip.pictures[0].rect;
    point_at(&mut browser, first, now);
    assert_eq!(browser.wheel_owner(), WheelOwner::Filmstrip);
    assert!(!matches!(browser.cursor(), crate::browser::Cursor::Wheel(_)), "no wheel indicator over it");
    point_at(&mut browser, Rect::new(10.0, HEIGHT - 20.0, 20.0, HEIGHT - 10.0), now);
    assert_ne!(browser.wheel_owner(), WheelOwner::Filmstrip, "below it, not the filmstrip's");
}
