use super::*;
use crate::browser::fixture::{HEIGHT, WIDTH, browser, middle};
use crate::browser::{Key, Modifiers, Output};
use crate::test_canvas::Recorder;

fn draw(browser: &mut Browser, now: u64) -> Recorder {
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, now);
    canvas
}

/// A browser in full screen since time 0, with the first page open.
fn in_full_screen() -> Browser {
    let (mut browser, _) = browser();
    assert!(browser.toggle_full_screen(0), "the first page on the wall opens");
    assert!(browser.full_screen());
    browser
}

/// Requirement 12.16: the open page alone, the largest it fits at its
/// proportions, centred, with no strip, tabs or panels; how to leave said
/// for three seconds in the margin, never over the page.
#[test]
fn full_screen_shows_the_open_page_alone() {
    let mut browser = in_full_screen();
    let canvas = draw(&mut browser, 1_000);
    assert_eq!(canvas.pages.len(), 1, "one page: {:?}", canvas.pages);
    let (_, image) = canvas.pages[0];
    let centre = middle(image);
    assert!((centre.x - WIDTH / 2.0).abs() < 1.0 && (centre.y - HEIGHT / 2.0).abs() < 1.0, "centred: {image:?}");
    assert!(image.height() > HEIGHT * 0.8 && image.y0 > 0.0, "as tall as the window holds, its screen around it: {image:?}");
    assert_eq!(browser.regions.field, Rect::ZERO, "no toolbar");
    assert!(browser.regions.tabs.is_empty(), "no tabs");
    let notice = canvas.texts.iter().find(|(_, text)| text == NOTICE).map(|(rect, _)| *rect);
    assert!(notice.is_some_and(|rect| rect.x1 <= image.x0), "the notice in the margin beside the page: {notice:?}");
    let later = draw(&mut browser, NOTICE_MILLISECONDS);
    assert!(!later.texts.iter().any(|(_, text)| text == NOTICE), "gone after three seconds");
}

/// Escape leaves, and the page stays open (section 5).
#[test]
fn escape_leaves_full_screen_with_the_page_open() {
    let mut browser = in_full_screen();
    draw(&mut browser, 100);
    let open = browser.wall().zoomed();
    assert!(browser.key(Key::Escape, Modifiers::default(), 200));
    assert!(!browser.full_screen());
    assert_eq!(browser.wall().zoomed(), open, "the same page, open in the window");
    draw(&mut browser, 300);
    assert_ne!(browser.regions.field, Rect::ZERO, "the strip is back");
}

/// Anything that needs the window ends full screen (section 5): a panel
/// of the strip, another tab.
#[test]
fn what_needs_the_window_ends_full_screen() {
    let mut browser = in_full_screen();
    let command = Modifiers { command: true, ..Modifiers::default() };
    browser.key(Key::Text(","), command, 100);
    draw(&mut browser, 100);
    assert!(!browser.full_screen(), "the settings panel");
    let mut browser = in_full_screen();
    let tabs = browser.tabs.clone();
    browser.set_tabs(tabs, 1);
    draw(&mut browser, 100);
    assert!(!browser.full_screen(), "another tab chosen");
    let mut browser = in_full_screen();
    browser.key(Key::Escape, command, 100);
    draw(&mut browser, 100);
    assert!(!browser.full_screen(), "the page closed");
}

/// The strip comes down after the pointer rests half a second at the top
/// edge, and goes when the pointer leaves it; a key the browser keeps
/// shows it for three seconds (section 4).
#[test]
fn the_strip_comes_down_from_the_top_edge() {
    let mut browser = in_full_screen();
    draw(&mut browser, 0);
    browser.pointer_moved(400.0, 1.0, 1_000);
    assert!(!browser.settle(1_000 + EDGE_WAIT_MILLISECONDS - 1) || browser.regions.field == Rect::ZERO);
    draw(&mut browser, 1_000 + EDGE_WAIT_MILLISECONDS - 1);
    assert_eq!(browser.regions.field, Rect::ZERO, "not yet");
    assert!(browser.settle(1_000 + EDGE_WAIT_MILLISECONDS));
    draw(&mut browser, 1_000 + EDGE_WAIT_MILLISECONDS);
    assert_ne!(browser.regions.field, Rect::ZERO, "the strip is down");
    assert!(!browser.tabs_strip_hit_is_page(), "a press there is the strip's");
    browser.pointer_moved(400.0, 600.0, 2_000);
    draw(&mut browser, 2_000);
    assert_eq!(browser.regions.field, Rect::ZERO, "gone when the pointer leaves it");
    browser.key(Key::Text("d"), Modifiers { command: true, ..Modifiers::default() }, 3_000);
    draw(&mut browser, 3_000);
    assert_ne!(browser.regions.field, Rect::ZERO, "a browser key shows it");
    assert!(browser.settle(3_000 + KEY_STRIP_MILLISECONDS));
    draw(&mut browser, 3_000 + KEY_STRIP_MILLISECONDS);
    assert_eq!(browser.regions.field, Rect::ZERO, "for three seconds");
}

impl Browser {
    /// Whether a press at the pointer now would be the page's.
    fn tabs_strip_hit_is_page(&self) -> bool {
        !self.full_screen_press_is_strip(u64::MAX / 2)
    }
}

/// With the strip up, a press where the window's address field was is
/// the page's: nothing unseen takes it.
#[test]
fn a_press_in_full_screen_is_the_page_s() {
    let (mut window, _) = browser();
    let field = window.regions.field;
    let mut browser = in_full_screen();
    draw(&mut browser, 100);
    browser.take_outputs();
    browser.pointer_moved(middle(field).x, middle(field).y, 200);
    browser.pointer_pressed(&mut Recorder::new(), false, 200);
    assert!(!browser.editing, "not the address field");
    assert!(browser.take_outputs().iter().all(|output| matches!(output, Output::Cell(_))), "only the page's cell");
    window.take_outputs();
}

/// A double click on the open page enters full screen; a single click is
/// the page's cell (section 2).
#[test]
fn a_double_click_on_the_open_page_enters_full_screen() {
    let (mut browser, _) = browser();
    let key = browser.wall().visible().first().copied().unwrap();
    browser.wall_mut().zoom(key, 0);
    draw(&mut browser, 1_000);
    let (_, image) = browser.wall().open().expect("the page is open");
    let point = middle(image);
    browser.take_outputs();
    browser.pointer_moved(point.x, point.y, 2_000);
    browser.pointer_pressed(&mut Recorder::new(), false, 2_000);
    browser.pointer_released();
    assert!(!browser.full_screen(), "one click");
    assert!(matches!(browser.take_outputs().as_slice(), [Output::Cell(_)]));
    browser.pointer_pressed(&mut Recorder::new(), false, 2_000 + DOUBLE_CLICK_MILLISECONDS);
    assert!(browser.full_screen(), "a double click");
    browser.leave_full_screen();
    browser.pointer_pressed(&mut Recorder::new(), false, 10_000);
    browser.pointer_pressed(&mut Recorder::new(), false, 10_000 + DOUBLE_CLICK_MILLISECONDS + 1);
    assert!(!browser.full_screen(), "too slow for a double click");
}

/// The pointer hides two seconds after it last moved (section 1).
#[test]
fn the_pointer_hides_when_still() {
    let mut browser = in_full_screen();
    assert!(!browser.pointer_hidden(POINTER_HIDE_MILLISECONDS - 1));
    assert!(browser.pointer_hidden(POINTER_HIDE_MILLISECONDS));
    assert_eq!(browser.frame_interval(0).map(|wait| wait <= POINTER_HIDE_MILLISECONDS), Some(true), "a frame is asked for then");
    browser.pointer_moved(10.0, 500.0, 5_000);
    assert!(!browser.pointer_hidden(5_000), "moving shows it");
    assert!(!(Browser::new(crate::browser::Frame { left_inset: 0.0, window_buttons: false })).pointer_hidden(10_000), "never in the window");
}
