use super::*;
use crate::browser::fixture::{HEIGHT, WIDTH, browser, cell_centre, press_at};
use crate::browser::window::{BODY_TOP, OVERVIEW_HEIGHT};
use crate::browser::{Cursor, Key, Modifiers, Output};
use crate::canvas::PageKey;
use crate::test_canvas::Recorder;

#[test]
fn a_click_on_the_overview_selects_its_page() {
    let (mut browser, mut canvas) = browser();
    let cells = browser.regions.cells;
    #[allow(clippy::cast_precision_loss, reason = "small counts")]
    let (width, height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
    // Page 610: bank 5, column 10.
    let point = Point::new(cells.x0 + 10.5 * width, cells.y0 + 5.5 * height);
    browser.pointer_moved(point.x, point.y, 0);
    browser.pointer_pressed(&mut canvas, false, 0);
    assert_eq!(browser.selected, page_index(0x610));
    assert_eq!(browser.bank, 5);
}

#[test]
fn a_page_in_the_overview_pops_up_at_native_size_under_a_hand() {
    let (mut browser, mut canvas) = browser();
    browser.add_page(7, 0x610, "Weather".into(), 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    browser.take_outputs();
    // Over a page not yet arrived: the arrow, and nothing pops up.
    let waiting = cell_centre(&browser, page_index(0x611).unwrap());
    assert!(!browser.pointer_moved(waiting.x, waiting.y, 0));
    assert_eq!(browser.cursor(), Cursor::Arrow);
    // Over a page that has arrived: a hand, and the page at 480 by 500.
    let point = cell_centre(&browser, page_index(0x610).unwrap());
    assert!(browser.pointer_moved(point.x, point.y, 0));
    assert_eq!(browser.cursor(), Cursor::Hand);
    canvas.pages.clear();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    let key = PageKey { channel: 7, page: 0x610 };
    let &(_, rect) = canvas.pages.iter().rev().find(|(drawn, _)| *drawn == key).expect("the preview is drawn");
    assert_eq!((rect.width(), rect.height()), (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT)));
    assert!(rect.y1 <= point.y && rect.y0 >= BODY_TOP, "above its cell, below the trusted strip: {rect:?}");
    // It is reported visible, so the trusted core has it rendered.
    let outputs = browser.take_outputs();
    assert!(outputs.iter().any(|output| matches!(output, Output::Visible(visible) if visible.contains(&key))), "{outputs:?}");
    // On a short window it shrinks rather than cover the strip.
    let short = BODY_TOP + OVERVIEW_HEIGHT + 300.0;
    browser.draw(&mut canvas, WIDTH, short, 0);
    let point = cell_centre(&browser, page_index(0x610).unwrap());
    browser.pointer_moved(point.x, point.y, 0);
    canvas.pages.clear();
    browser.draw(&mut canvas, WIDTH, short, 0);
    let &(_, rect) = canvas.pages.iter().rev().find(|(drawn, _)| *drawn == key).expect("the preview is drawn");
    assert!(rect.y0 >= BODY_TOP && rect.height() < f64::from(PAGE_HEIGHT), "{rect:?}");
}

#[test]
fn the_overview_folds_into_the_footer_and_rests_there() {
    let (mut browser, mut canvas) = browser();
    let open_cells = browser.regions.cells;
    let wall_before = browser.regions.list;
    let toggle = browser.regions.overview_toggle;
    press_at(&mut browser, &mut canvas, toggle);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    // Folded: one heading high at the bottom, no cells, the panels above taller.
    assert!((browser.regions.overview.height() - HEADING_HEIGHT).abs() < 1e-9);
    assert_eq!(browser.regions.cells, Rect::ZERO);
    assert!(browser.regions.list.height() > wall_before.height());
    // A page arriving starts no animation and asks for no frames.
    browser.add_page(7, 0x105, "Arrived while folded".into(), 1_000);
    assert!(!browser.animating(1_000));
    assert!(!browser.pointer_moved(open_cells.x0 + 1.0, open_cells.y0 + 1.0, 1_000), "nothing to hover");
    assert_eq!(browser.cursor(), Cursor::Arrow);
    // Opened again: drawn as things are now, the page that arrived loaded.
    let toggle = browser.regions.overview_toggle;
    press_at(&mut browser, &mut canvas, toggle);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 2_000);
    assert_eq!(browser.regions.cells, open_cells);
    let point = cell_centre(&browser, page_index(0x105).unwrap());
    browser.pointer_moved(point.x, point.y, 2_000);
    assert_eq!(browser.cursor(), Cursor::Hand);
}

#[test]
fn the_pointer_is_a_hand_over_the_overview_fold_button() {
    let (mut browser, mut canvas) = browser();
    for folded in [false, true] {
        let toggle = browser.regions.overview_toggle;
        browser.pointer_moved(f64::midpoint(toggle.x0, toggle.x1), f64::midpoint(toggle.y0, toggle.y1), 0);
        assert_eq!(browser.cursor(), Cursor::Hand, "folded: {folded}");
        browser.pointer_moved(toggle.x0 - 1.0, f64::midpoint(toggle.y0, toggle.y1), 0);
        assert_eq!(browser.cursor(), Cursor::Arrow, "folded: {folded}");
        press_at(&mut browser, &mut canvas, toggle);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    }
}

/// Decision 109: the View menu's overview, by Shift-Command-E or the
/// browser's own menu, comes off the window and gives the pages its room.
#[test]
fn the_overview_can_be_hidden_and_shown_again() {
    let (mut browser, mut canvas) = browser();
    let list_bottom = browser.regions.list.y1;
    browser.key(Key::Text("E"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
    assert!(!browser.overview_shown());
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    assert!(browser.regions.list.y1 > list_bottom + OVERVIEW_HEIGHT, "the pages take its room");
    browser.toggle_menu();
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    assert!(canvas.texts.iter().any(|(_, text)| text == "Show overview of all pages"), "the menu offers it back");
    browser.key(Key::Text("e"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    assert!((browser.regions.list.y1 - list_bottom).abs() < 1e-9, "shown again where it was");
}

/// Browser issue #2: the overview's close button, beside its fold button,
/// hides it as the View menu's item does, with the hand over it and a tip
/// that names the menu's shortcut; the menu brings it back.
#[test]
fn the_overview_close_button_hides_it_as_the_view_menu_does() {
    let (mut browser, mut canvas) = browser();
    let list_bottom = browser.regions.list.y1;
    let close = browser.regions.overview_close;
    assert!(close.x1 < browser.regions.overview_toggle.x0, "left of the fold button");
    browser.pointer_moved(f64::midpoint(close.x0, close.x1), f64::midpoint(close.y0, close.y1), 0);
    assert_eq!(browser.cursor(), Cursor::Hand);
    press_at(&mut browser, &mut canvas, close);
    assert!(!browser.overview_shown());
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    assert!(browser.regions.list.y1 > list_bottom + OVERVIEW_HEIGHT, "the pages take its room");
    assert_eq!(browser.regions.overview_close, Rect::ZERO, "no button left behind");
    browser.key(Key::Text("e"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    assert!((browser.regions.list.y1 - list_bottom).abs() < 1e-9, "shown again where it was");
}
