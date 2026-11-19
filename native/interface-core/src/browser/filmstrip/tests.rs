//! The tab filmstrip's tests, one or more for each item of REQ-6640
//! (DEC-6637): each says the item it verifies.

use alloc::format;
use alloc::vec::Vec;

use super::draw::{Layout, MARGIN};
use super::*;
use crate::browser::fixture::{HEIGHT, WIDTH, channel, middle};
use crate::browser::{Frame, Key, Modifiers, TabInfo};
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
            costly: false,
            address: format!("https://tab{index}.invalid/"),
            page: None,
            subpage: None,
            page_title: alloc::string::String::new(),
        })
        .collect()
}

/// Rests the pointer on the first tab at `now` and waits it out, and the
/// first tab's picture shown alone (REQ-6921): the whole row shows and has
/// settled by the time it is drawn again.
fn open(browser: &mut Browser, canvas: &mut Recorder, now: u64) -> u64 {
    let tab = browser.regions.tabs[0].tab;
    browser.pointer_moved(middle(tab).x, middle(tab).y, now);
    let opened = now.saturating_add(FILMSTRIP_DELAY_MILLISECONDS);
    assert!(browser.settle(opened), "it opens");
    browser.draw(canvas, WIDTH, HEIGHT, opened.saturating_add(FILMSTRIP_OPEN_MILLISECONDS));
    let joined = opened.saturating_add(FILMSTRIP_ALONE_MILLISECONDS);
    assert!(browser.settle(joined), "the rest of the row appears");
    let out = joined.saturating_add(FILMSTRIP_SLIDE_MILLISECONDS.max(FILMSTRIP_OPEN_MILLISECONDS));
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
    browser.settle(2_000 + FILMSTRIP_DELAY_MILLISECONDS + FILMSTRIP_ALONE_MILLISECONDS);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 4_000);
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

/// REQ-6640 item 1 (TODO-6758): a hand that comes to rest on the count to
/// click it rests longer than the delay, so the rest opens the filmstrip
/// first; the click that follows keeps it open. Before, that click shut it
/// and kept it shut while the pointer stayed on the strip, so the end-to-end
/// run of 19 November 2026 saw only the count's tip. A second press shuts
/// it, and a rest elsewhere on the tabs is not the count's.
#[test]
fn a_press_on_the_count_the_rest_opened_it_from_keeps_the_filmstrip_open() {
    let (mut browser, mut canvas) = browser_of(30);
    let count = browser.tab_strip.count;
    point_at(&mut browser, count, 0);
    assert!(browser.settle(FILMSTRIP_DELAY_MILLISECONDS) && browser.filmstrip_open(), "the rest opens it");
    browser.draw(&mut canvas, WIDTH, HEIGHT, 400);
    browser.pointer_moved(middle(count).x + 1.0, middle(count).y, 450);
    browser.pointer_pressed(&mut canvas, false, 500);
    browser.pointer_released();
    browser.settle(2_000);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 2_000);
    assert!(browser.filmstrip_open(), "the click the rest came before keeps it open");
    assert_eq!(browser.tip_shown(), None, "and no tip shows over it");
    browser.pointer_pressed(&mut canvas, false, 2_100);
    assert!(!browser.filmstrip_open(), "a second press shuts it");
    // Opened by a rest on a tab, the pointer then going to the count: a
    // press there shuts it, as ever.
    let tab = browser.regions.tabs[0].tab;
    let field = browser.regions.field;
    point_at(&mut browser, field, 3_000);
    point_at(&mut browser, tab, 3_100);
    assert!(browser.settle(3_100 + FILMSTRIP_DELAY_MILLISECONDS) && browser.filmstrip_open());
    browser.draw(&mut canvas, WIDTH, HEIGHT, 3_700);
    point_at(&mut browser, count, 3_800);
    browser.pointer_pressed(&mut canvas, false, 3_900);
    assert!(!browser.filmstrip_open(), "rested on a tab, not the count: the press shuts it");
}

/// REQ-6640 item 1: the filmstrip opens with no tab pressed, so a press on
/// a tab while the rest is being waited out ends the wait. Before, the
/// filmstrip opened 300 milliseconds after the pointer came to the tab,
/// over the tab just clicked.
#[test]
fn a_press_on_a_tab_during_the_rest_opens_nothing() {
    let (mut browser, mut canvas) = browser_of(3);
    let tab = browser.regions.tabs[1].tab;
    point_at(&mut browser, tab, 0);
    browser.pointer_pressed(&mut canvas, false, 100);
    browser.pointer_released();
    browser.settle(1_000);
    assert!(!browser.filmstrip_open(), "the press ended the rest");
    browser.pointer_moved(middle(tab).x + 1.0, middle(tab).y, 2_000);
    assert!(browser.settle(2_000 + FILMSTRIP_DELAY_MILLISECONDS) && browser.filmstrip_open(), "a rest after it opens it");
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
    let out = open(&mut browser, &mut canvas, 100);
    assert_eq!(canvas.taken.last().map(|&(still, _)| still), Some(FIRST + 1), "the active tab's, as it opens");
    // The whole row as drawn, not the first tab's picture alone before it.
    canvas.painted.clear();
    browser.draw(&mut canvas, WIDTH, HEIGHT, out);
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

/// REQ-6887: a tab without a still shows its card, in place of REQ-6640
/// item 3's blank stand-ins: the channel's name on a band in the tab's
/// colour, its page number, its page's title and "No picture yet", in
/// the teletext page's shape, darkened and in a dashed border.
#[test]
fn a_tab_without_a_still_shows_its_card_with_name_page_and_title() {
    let (mut browser, mut canvas) = browser_of(2);
    let mut infos = tabs(2);
    infos[1].page = Some(0x302);
    infos[1].subpage = Some(0x0002);
    infos[1].page_title = alloc::string::String::from("Weather for the whole country");
    infos[1].colour = 4;
    browser.set_tabs(infos, 0);
    canvas.texts.clear();
    canvas.fills.clear();
    canvas.dims.clear();
    open(&mut browser, &mut canvas, 100);
    let card = browser.filmstrip.pictures.iter().find(|picture| picture.index == 1).unwrap().rect;
    let inside = |rect: &Rect| card.contains(middle(*rect));
    let said: Vec<&str> = canvas.texts.iter().filter(|(rect, _)| inside(rect)).map(|(_, text)| text.as_str()).collect();
    for expected in ["Channel 1", "P302/0002", "Weather for the whole country", "No picture yet"] {
        assert!(said.contains(&expected), "{expected:?} on the card: {said:?}");
    }
    let inks: Vec<(&str, crate::theme::Colour)> = canvas.inks.iter().map(|(text, colour)| (text.as_str(), *colour)).collect();
    assert!(inks.contains(&("P302/0002", crate::theme::YELLOW)) && inks.contains(&("No picture yet", crate::theme::CYAN)));
    assert!(canvas.fills.iter().any(|(rect, colour)| inside(rect) && *colour == crate::theme::TAB_COLOURS[4].0), "the band in the tab's colour");
    assert!(canvas.dims.iter().any(|(rect, _)| (rect.x0, rect.y0) == (card.x0, card.y1 - rect.height()) || inside(rect)), "darkened");
    let dashes = canvas.fills.iter().filter(|(rect, colour)| inside(rect) && *colour == crate::theme::SELECTED).count();
    assert!(dashes > 8, "a dashed border: {dashes} dashes");
}

/// REQ-6887 item 1: the card of an address naming no page says
/// "Start page".
#[test]
fn the_card_names_the_start_page_for_an_address_without_a_page() {
    let (mut browser, mut canvas) = browser_of(2);
    canvas.texts.clear();
    open(&mut browser, &mut canvas, 100);
    let card = browser.filmstrip.pictures.iter().find(|picture| picture.index == 1).unwrap().rect;
    assert!(canvas.texts.iter().any(|(rect, text)| card.contains(middle(*rect)) && text == "Start page"));
}

/// REQ-6883: the open filmstrip asks for the tabs in view without stills,
/// left to right, once its slide has settled, and again when they change;
/// and empty when it closes.
#[test]
fn the_filmstrip_asks_for_the_tabs_in_view_without_stills() {
    let (mut browser, mut canvas) = browser_of(3);
    browser.take_outputs();
    let out = open(&mut browser, &mut canvas, 100);
    // The active tab's still is taken as it opens: the others are asked for.
    assert_eq!(browser.take_outputs(), [Output::Thumbnails(alloc::vec![FIRST + 1, FIRST + 2])]);
    browser.draw(&mut canvas, WIDTH, HEIGHT, out + 10);
    assert_eq!(browser.take_outputs(), [], "asked once while nothing changes");
    browser.thumbnail_kept(FIRST + 1, 0x100, false, true);
    browser.draw(&mut canvas, WIDTH, HEIGHT, out + 20);
    assert_eq!(browser.take_outputs(), [Output::Thumbnails(alloc::vec![FIRST + 2])], "a still made: no longer asked");
    assert!(browser.filmstrip_escape(out + 30));
    browser.draw(&mut canvas, WIDTH, HEIGHT, out + 30 + FILMSTRIP_OPEN_MILLISECONDS);
    assert_eq!(browser.take_outputs(), [Output::Thumbnails(Vec::new())], "empty as it closes");
}

/// REQ-6883: a thumbnail becomes a still of its tab, which replaces the
/// card; a still as seen this run replaces it, and is never replaced by one.
#[test]
fn a_thumbnail_becomes_a_still_and_a_seen_still_replaces_it() {
    let (mut browser, mut canvas) = browser_of(2);
    assert!(browser.thumbnail_wanted(FIRST + 1, 0x100, 0));
    browser.thumbnail_kept(FIRST + 1, 0x100, false, true);
    canvas.stills.push((FIRST + 1, PageKey { channel: 0, page: 0 }));
    canvas.painted.clear();
    canvas.texts.clear();
    open(&mut browser, &mut canvas, 100);
    assert!(canvas.painted.iter().any(|painted| matches!(painted, Painted::Still(still, _) if *still == FIRST + 1)), "the still drawn");
    assert!(!canvas.texts.iter().any(|(_, text)| text == "No picture yet"), "no card");
    assert!(browser.thumbnail_wanted(FIRST + 1, 0x100, 0), "a newer snapshot may replace a snapshot's still");
    // Seen: the viewer goes to the tab and leaves it.
    browser.set_tabs(tabs(2), 1);
    let mut other = channel();
    other.channel = FIRST + 1;
    browser.set_channel(other);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 1_000);
    browser.set_tabs(tabs(2), 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 1_100);
    assert!(!browser.filmstrip.still(FIRST + 1).unwrap().snapshot, "the still as seen replaced it");
    assert!(!browser.thumbnail_wanted(FIRST + 1, 0x100, 0), "and no thumbnail replaces it");
}

/// REQ-6883: a thumbnail of a tab gone, or of a place the tab no longer
/// shows, is dropped.
#[test]
fn a_thumbnail_of_a_tab_gone_or_moved_is_dropped() {
    let (mut browser, _) = browser_of(2);
    assert!(!browser.thumbnail_wanted(FIRST + 5, 0x100, 0), "no such tab");
    let mut infos = tabs(2);
    infos[1].page = Some(0x200);
    browser.set_tabs(infos.clone(), 0);
    assert!(!browser.thumbnail_wanted(FIRST + 1, 0x100, 0), "the tab moved to page 200");
    assert!(browser.thumbnail_wanted(FIRST + 1, 0x200, 3));
    infos[1].subpage = Some(4);
    browser.set_tabs(infos, 0);
    assert!(!browser.thumbnail_wanted(FIRST + 1, 0x200, 3), "another sub-page");
}

/// REQ-5819, DEC-6667: a page known to be a NAPLPS picture is the
/// teletext raster, so it stands in black in the teletext page's shape,
/// not a ReGIS picture's.
#[test]
fn a_naplps_page_not_yet_drawn_stands_in_in_the_teletext_pages_shape() {
    let (mut browser, mut canvas) = browser_of(2);
    let shown = browser.wall.shown().next().map(|(key, _)| key).unwrap();
    browser.set_regis(shown, 1, 1, crate::canvas::PictureLanguage::Naplps);
    canvas.has_images = false;
    browser.set_tabs(tabs(2), 1);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 10);
    canvas.fills.clear();
    open(&mut browser, &mut canvas, 100);
    let left = browser.filmstrip.pictures.iter().find(|picture| picture.index == 0).unwrap();
    let inside = |rect: Rect| left.rect.contains(Point::new(rect.x0, rect.y0)) && left.rect.contains(Point::new(rect.x1 - 0.5, rect.y1 - 0.5));
    let black = canvas.fills.iter().find(|&&(rect, colour)| colour == BLACK && inside(rect)).map(|&(rect, _)| rect).unwrap();
    // The teletext page's image within its screen: 480 dots by 500 lines,
    // drawn wider by at most the aspect correction (DEC-2049).
    let teletext = f64::from(crate::canvas::PAGE_HEIGHT) / f64::from(crate::canvas::PAGE_WIDTH);
    let shape = black.height() / black.width();
    assert!(shape <= teletext + 1e-6 && shape >= teletext * 13.0 / 16.0 - 1e-6, "the teletext page's shape: {black:?}");
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

/// REQ-6640 item 6: opening the whole row at once, by a rest on the count,
/// places it so the active tab's picture is whole in view, and outlines
/// it. Opened by a rest on a tab, the row is laid out around that tab's
/// picture instead (REQ-6921, DEC-6920 item 6).
#[test]
fn the_active_tabs_picture_is_in_view_and_outlined() {
    let (mut browser, mut canvas) = browser_of(12);
    browser.set_tabs(tabs(12), 10);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    canvas.fills.clear();
    let count = browser.tab_strip.count;
    point_at(&mut browser, count, 0);
    assert!(browser.settle(FILMSTRIP_DELAY_MILLISECONDS) && browser.filmstrip_open(), "the rest on the count opens it");
    browser.draw(&mut canvas, WIDTH, HEIGHT, FILMSTRIP_DELAY_MILLISECONDS + FILMSTRIP_SLIDE_MILLISECONDS.max(FILMSTRIP_OPEN_MILLISECONDS));
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

/// Rests the pointer on tab `index` at `now` and waits out the rest: the
/// first stage has slid out by the time returned, when it is drawn.
fn rest_on(browser: &mut Browser, canvas: &mut Recorder, index: usize, now: u64) -> u64 {
    let tab = browser.regions.tabs.iter().find(|region| region.index == index).unwrap().tab;
    point_at(browser, tab, now);
    let opened = now.saturating_add(FILMSTRIP_DELAY_MILLISECONDS);
    assert!(browser.settle(opened) && browser.filmstrip_open(), "the rest opens it");
    let out = opened.saturating_add(FILMSTRIP_OPEN_MILLISECONDS);
    browser.draw(canvas, WIDTH, HEIGHT, out);
    out
}

/// The middle of tab `index` on the strip, across.
fn tab_middle(browser: &Browser, index: usize) -> f64 {
    middle(browser.regions.tabs.iter().find(|region| region.index == index).unwrap().tab).x
}

/// REQ-6921 item 1 (DEC-6920): a rest on a tab first shows that tab's
/// picture alone, centred directly under the tab, on a panel no wider than
/// it and its margins; under the first tab it is kept within the window's
/// margin. Before, every tab's picture showed at once in the row.
#[test]
fn a_rest_on_a_tab_shows_its_picture_alone_directly_under_it() {
    let (mut browser, mut canvas) = browser_of(5);
    rest_on(&mut browser, &mut canvas, 2, 0);
    let indices: Vec<usize> = browser.filmstrip.pictures.iter().map(|picture| picture.index).collect();
    assert_eq!(indices, [2], "the rested tab's picture alone");
    let picture = browser.filmstrip.pictures[0].rect;
    assert!((middle(picture).x - tab_middle(&browser, 2)).abs() < 1e-3, "centred under its tab: {picture:?}");
    let panel = browser.filmstrip.panel;
    assert!((panel.x0 - (picture.x0 - MARGIN)).abs() < 1e-6 && (panel.x1 - (picture.x1 + MARGIN)).abs() < 1e-6, "a panel only as wide as it: {panel:?}");
    // Under the first tab, nearer the window's edge than half a picture.
    let (mut browser, mut canvas) = browser_of(5);
    rest_on(&mut browser, &mut canvas, 0, 0);
    let picture = browser.filmstrip.pictures[0].rect;
    assert_eq!(browser.filmstrip.pictures[0].index, 0);
    assert!(tab_middle(&browser, 0) - picture.width() / 2.0 < MARGIN, "the fixture's first tab is near the edge");
    assert!((picture.x0 - MARGIN).abs() < 1e-6, "kept whole within the margin: {picture:?}");
}

/// REQ-6921 items 3 and 4 (DEC-6920): 500 milliseconds after the picture
/// appeared alone, with the pointer still on the strip, every tab's picture
/// shows. Before, the row showed from the start.
#[test]
fn the_rest_of_the_row_appears_500_milliseconds_later() {
    let (mut browser, mut canvas) = browser_of(5);
    rest_on(&mut browser, &mut canvas, 2, 0);
    let shown = FILMSTRIP_DELAY_MILLISECONDS;
    assert_eq!(browser.frame_interval(shown + 300), Some(FILMSTRIP_ALONE_MILLISECONDS - 300), "it waits for the rest");
    assert!(!browser.settle(shown + FILMSTRIP_ALONE_MILLISECONDS - 1), "not yet");
    browser.draw(&mut canvas, WIDTH, HEIGHT, shown + FILMSTRIP_ALONE_MILLISECONDS - 1);
    assert_eq!(browser.filmstrip.pictures.len(), 1, "still alone");
    let joined = shown + FILMSTRIP_ALONE_MILLISECONDS;
    assert!(browser.settle(joined), "the rest appear");
    browser.draw(&mut canvas, WIDTH, HEIGHT, joined);
    let indices: Vec<usize> = browser.filmstrip.pictures.iter().map(|picture| picture.index).collect();
    assert_eq!(indices, [0, 1, 2, 3, 4], "every tab's picture");
    assert!((browser.filmstrip.panel.width() - WIDTH).abs() < 1e-6, "the whole width");
}

/// The rectangles of the pictures drawn, by tab.
fn picture_rects(browser: &Browser) -> Vec<(usize, Rect)> {
    browser.filmstrip.pictures.iter().map(|picture| (picture.index, picture.rect)).collect()
}

/// DEC-6920 item 6 (REQ-6921 item 4): when the rest of the row appears,
/// the picture already shown does not move at all, in the frame they
/// appear or any after, and the others appear in their places in the row
/// beside it and stay there; resting on a middle tab and on the end tab
/// of a row longer than the window. Before (browser e24ab67), the picture
/// glided from under its tab to the row's usual place as the others
/// appeared: the owner, "when the others appeared, they moved the one that
/// was already there".
#[test]
fn the_picture_already_shown_stays_exactly_where_it_is_when_the_row_appears() {
    const TABS: usize = 9;
    let on_strip: Vec<usize> = browser_of(TABS).0.regions.tabs.iter().map(|region| region.index).collect();
    let (middle_tab, end_tab) = (on_strip[on_strip.len() / 2], on_strip[on_strip.len() - 1]);
    assert!(middle_tab > 0 && end_tab > middle_tab, "the fixture shows a middle and an end tab: {on_strip:?}");
    for rested in [middle_tab, end_tab] {
        let (mut browser, mut canvas) = browser_of(TABS);
        rest_on(&mut browser, &mut canvas, rested, 0);
        let joined = FILMSTRIP_DELAY_MILLISECONDS + FILMSTRIP_ALONE_MILLISECONDS;
        browser.draw(&mut canvas, WIDTH, HEIGHT, joined - 1);
        let last_alone = picture_rects(&browser);
        assert_eq!(last_alone.len(), 1, "tab {rested}: the last frame of the picture alone");
        let alone = last_alone[0].1;
        assert!(browser.settle(joined), "tab {rested}: the rest appear");
        let layout = Layout::of(WIDTH, TABS, browser.wall.shape().height_per_width());
        let mut first_row: Option<Vec<(usize, Rect)>> = None;
        for after in [0, 1, 50, 100, FILMSTRIP_SLIDE_MILLISECONDS, FILMSTRIP_SLIDE_MILLISECONDS * 2, 1_000] {
            let now = joined + after;
            browser.draw(&mut canvas, WIDTH, HEIGHT, now);
            let row = picture_rects(&browser);
            assert!(row.len() > 1, "tab {rested}, {after} ms on: the others show: {row:?}");
            let still = row.iter().find(|&&(index, _)| index == rested).map(|&(_, rect)| rect);
            assert_eq!(still, Some(alone), "tab {rested}, {after} ms on: exactly where it was alone");
            for &(index, rect) in &row {
                #[allow(clippy::cast_precision_loss, reason = "a tab's place, below ten")]
                let expected = alone.x0 + (index as f64 - rested as f64) * layout.step;
                assert!((rect.x0 - expected).abs() < 1e-6, "tab {rested}, {after} ms on: picture {index} in its place beside it: {rect:?}");
            }
            match &first_row {
                Some(first) => assert_eq!(&row, first, "tab {rested}, {after} ms on: nothing moves after the row appears"),
                None => first_row = Some(row),
            }
            assert_eq!(browser.filmstrip_slide_at(now).0, browser.filmstrip_slide_at(now).1, "tab {rested}: the row is not sliding");
        }
        let (least, most) = browser.filmstrip_bounds();
        let (slide, _) = browser.filmstrip_slide_at(joined);
        assert!(slide >= least && slide <= most, "tab {rested}: the slide is within the wheel's bounds: {slide} {least} {most}");
    }
}

/// REQ-6921 item 2 (DEC-6920): moving to another tab while the picture
/// shows alone moves it under that tab, showing that tab's picture, and the
/// rest still appear 500 milliseconds after it first appeared.
#[test]
fn moving_to_another_tab_moves_the_lone_picture_under_it() {
    let (mut browser, mut canvas) = browser_of(5);
    let out = rest_on(&mut browser, &mut canvas, 1, 0);
    let tab = browser.regions.tabs.iter().find(|region| region.index == 3).unwrap().tab;
    point_at(&mut browser, tab, out + 50);
    browser.draw(&mut canvas, WIDTH, HEIGHT, out + 60);
    let indices: Vec<usize> = browser.filmstrip.pictures.iter().map(|picture| picture.index).collect();
    assert_eq!(indices, [3], "the other tab's picture alone");
    let picture = browser.filmstrip.pictures[0].rect;
    assert!((middle(picture).x - tab_middle(&browser, 3)).abs() < 1e-3, "under the other tab: {picture:?}");
    assert!(browser.settle(FILMSTRIP_DELAY_MILLISECONDS + FILMSTRIP_ALONE_MILLISECONDS), "the wait was not restarted");
    browser.draw(&mut canvas, WIDTH, HEIGHT, FILMSTRIP_DELAY_MILLISECONDS + FILMSTRIP_ALONE_MILLISECONDS);
    assert_eq!(browser.filmstrip.pictures.len(), 5);
}
