//! What the window's tests share: a browser with two tabs, a strip and a
//! test channel, drawn once at a desktop window's size, and presses at the
//! middle of what was drawn.

use alloc::format;

use crate::browser::{Browser, Frame, StripState, TabInfo};
use crate::channel::{BANKS, Channel, PAGES, PAGES_PER_BANK, PageInfo, Status};
use crate::geometry::{Point, Rect};
use crate::test_canvas::Recorder;

pub(super) const WIDTH: f64 = 1440.0;
pub(super) const HEIGHT: f64 = 900.0;

pub(super) fn channel() -> Channel {
    let mut channel = Channel::empty();
    channel.channel = 7;
    channel.title = "Test".into();
    for index in (0..PAGES).filter(|index| index % 3 != 2) {
        channel.pages[index] = Some(PageInfo { title: format!("Page {index}"), status: Status::Available });
    }
    channel
}

pub(super) fn browser() -> (Browser, Recorder) {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
    browser.set_tabs(
        alloc::vec![
            TabInfo { title: "one".into(), colour: 0, channel: 7, muted: false, private: false, address: "https://example.invalid/".into() },
            TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false, private: false, address: "https://example.invalid/".into() }
        ],
        0,
    );
    browser.set_strip(StripState {
        address: "https://example.invalid/".into(),
        identity: "example.invalid".into(),
        encrypted: true,
        forward: true,
        ..StripState::default()
    });
    browser.set_channel(channel());
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    (browser, canvas)
}

pub(super) fn press_at(browser: &mut Browser, canvas: &mut Recorder, rect: Rect) {
    browser.pointer_moved(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1), 0);
    browser.pointer_pressed(canvas, false, 0);
}

/// A browser with no tabs and an empty channel, drawn once at the
/// window's size, as most panels' tests start.
pub(super) fn blank_browser() -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    browser
}

/// The middle of `rect`.
pub(super) fn middle(rect: Rect) -> Point {
    Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
}

/// The overview cell of page `index`'s centre.
pub(super) fn cell_centre(browser: &Browser, index: usize) -> Point {
    let cells = browser.regions.cells;
    #[allow(clippy::cast_precision_loss, reason = "small counts")]
    let (width, height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
    #[allow(clippy::cast_precision_loss, reason = "small counts")]
    Point::new(cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * width, cells.y0 + ((index / PAGES_PER_BANK) as f64 + 0.5) * height)
}
