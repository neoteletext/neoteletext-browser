//! The open page's controls in the window (`page-controls.md`, decision
//! 67): the keys of the engine's own viewer while a page is open, and a
//! sub-page control on a page with more than one. The trusted core says
//! what the page has; every key goes to it as strip input.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::controls::{FastextKey, PageControl, PageControls};
use crate::geometry::{Point, Rect};
use crate::theme::{BUTTON, ROW_TITLE, RULE, WHITE};
use crate::ui::{border, centred};

/// The sub-page control: its height, each step's width, the count's width,
/// and its distance from the page's top right corner.
const CHIP_HEIGHT: f64 = 30.0;
const STEP_WIDTH: f64 = 30.0;
const COUNT_WIDTH: f64 = 76.0;
const INSET: f64 = 8.0;
const RADIUS: f64 = 4.0;

/// Where the sub-page control was last drawn: the step back, the count
/// (back to the latest), the step on.
#[derive(Debug, Default)]
pub(super) struct ControlsUi {
    pub(super) controls: PageControls,
    previous: Rect,
    count: Rect,
    next: Rect,
}

impl Browser {
    /// The open page's controls, as the trusted core sent them.
    pub fn set_page_controls(&mut self, controls: PageControls) {
        self.page_controls.controls = controls;
    }

    /// The open page's controls, for the remote control and the tests.
    #[must_use]
    pub fn page_controls(&self) -> PageControls {
        self.page_controls.controls
    }

    /// Asks for `control` on the open page.
    pub(super) fn ask_control(&mut self, control: PageControl) {
        self.outputs.push(Output::PageControl(control));
    }

    /// Whether the controls are those of the page open now.
    fn controls_open(&self) -> bool {
        self.wall.zoomed().is_some_and(|key| self.page_controls.controls.page == Some(key.page))
    }

    /// A key while a page is open: the engine viewer's keys for its
    /// controls (the engine's user guide, "The viewer"); `None` when it is
    /// not one.
    pub(super) fn control_key(&mut self, key: Key<'_>) -> Option<bool> {
        if !self.controls_open() {
            return None;
        }
        let control = match key {
            Key::Text("r") => PageControl::Fastext(FastextKey::Red),
            Key::Text("g") => PageControl::Fastext(FastextKey::Green),
            Key::Text("y") => PageControl::Fastext(FastextKey::Yellow),
            Key::Text("c") => PageControl::Fastext(FastextKey::Cyan),
            Key::Text("i") => PageControl::Fastext(FastextKey::Index),
            Key::Text("?") => PageControl::Reveal,
            Key::Text("H") => PageControl::Hold,
            Key::Text("m") => PageControl::Mix,
            // On a page with sub-pages the arrows step through them, as in
            // the engine's viewer; on others they size the pages.
            Key::Left | Key::Right if self.page_controls.controls.subpages > 1 => PageControl::SubPage(key == Key::Right),
            _ => return None,
        };
        self.ask_control(control);
        Some(true)
    }

    /// A coloured key pressed on the keyboard's own key for it (F1 to F4),
    /// while a page is open; returns whether it was taken.
    pub fn fastext_key(&mut self, key: FastextKey) -> bool {
        if !self.controls_open() {
            return false;
        }
        self.ask_control(PageControl::Fastext(key));
        true
    }

    /// A press on the sub-page control; returns whether it was there.
    pub(super) fn controls_press(&mut self, point: Point) -> bool {
        let ui = &self.page_controls;
        let control = if ui.previous.contains(point) {
            PageControl::SubPage(false)
        } else if ui.next.contains(point) {
            PageControl::SubPage(true)
        } else if ui.count.contains(point) {
            PageControl::Latest
        } else {
            return false;
        };
        self.ask_control(control);
        true
    }

    /// The sub-page control, over the open page's top right corner, while
    /// it has more than one sub-page.
    pub(super) fn draw_page_controls<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let (mut previous, mut count, mut next) = (Rect::ZERO, Rect::ZERO, Rect::ZERO);
        let controls = self.page_controls.controls;
        if let Some((key, page)) = self.wall.open()
            && controls.page == Some(key.page)
            && controls.subpages > 1
        {
            let right = page.x1 - INSET;
            let top = page.y0 + INSET;
            next = Rect::new(right - STEP_WIDTH, top, right, top + CHIP_HEIGHT);
            count = Rect::new(next.x0 - COUNT_WIDTH, top, next.x0, top + CHIP_HEIGHT);
            previous = Rect::new(count.x0 - STEP_WIDTH, top, count.x0, top + CHIP_HEIGHT);
            let chip = Rect::new(previous.x0, top, right, top + CHIP_HEIGHT);
            canvas.fill(chip, RADIUS, BUTTON);
            border(canvas, chip, 1.0, RULE);
            centred(canvas, previous, "\u{2039}", ROW_TITLE, WHITE);
            centred(canvas, next, "\u{203A}", ROW_TITLE, WHITE);
            // "2/4" while one is chosen; a mark while the latest rotates in.
            let words = match controls.chosen {
                Some(place) => format!("{}/{}", u16::from(place) + 1, controls.subpages),
                None => format!("{} \u{21BB}", controls.subpages),
            };
            centred(canvas, count, &words, ROW_TITLE, WHITE);
        }
        let ui = &mut self.page_controls;
        (ui.previous, ui.count, ui.next) = (previous, count, next);
    }

    /// The sub-page control's tips.
    pub(super) fn controls_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let ui = &self.page_controls;
        if ui.count == Rect::ZERO {
            return;
        }
        tips.push((ui.previous, String::from("The sub-page before (Left)")));
        tips.push((ui.next, String::from("The next sub-page (Right)")));
        let count = if ui.controls.chosen.is_some() { "Back to the sub-pages as they arrive" } else { "Sub-pages: the latest shows as it arrives" };
        tips.push((ui.count, String::from(count)));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use crate::browser::{Frame, Modifiers, StripState, TabInfo};
    use crate::canvas::PageKey;
    use crate::channel::{Channel, PAGES, PageInfo, Status};
    use crate::test_canvas::Recorder;
    use alloc::vec;

    const WIDTH: f64 = 1440.0;
    const HEIGHT: f64 = 900.0;
    const OPEN: u64 = 1_000;

    /// A browser with a page open, its controls as the trusted core sent
    /// them with `subpages` sub-pages.
    fn opened(subpages: u8) -> (Browser, PageKey) {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_tabs(vec![TabInfo { title: "one".into(), colour: 0, channel: 7, muted: false, private: false }], 0);
        browser.set_strip(StripState { address: "https://news.invalid/".into(), identity: "news.invalid".into(), ..StripState::default() });
        let mut channel = Channel::empty();
        channel.channel = 7;
        for index in 0..PAGES {
            channel.pages[index] = Some(PageInfo { title: format!("Page {index}"), status: Status::Loaded });
        }
        browser.set_channel(channel);
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
        let (key, _) = browser.wall().shown().next().unwrap();
        browser.wall_mut().zoom(key, 0);
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, OPEN);
        browser.set_page_controls(PageControls { page: Some(key.page), subpages, fastext: [true; 5], ..PageControls::default() });
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, OPEN);
        browser.take_outputs();
        (browser, key)
    }

    /// Decision 67: the engine viewer's keys ask for the open page's
    /// controls; the arrows step sub-pages only where there are some.
    #[test]
    fn the_viewers_keys_ask_for_the_controls() {
        let (mut browser, _) = opened(3);
        for (text, control) in [
            ("r", PageControl::Fastext(FastextKey::Red)),
            ("c", PageControl::Fastext(FastextKey::Cyan)),
            ("i", PageControl::Fastext(FastextKey::Index)),
            ("?", PageControl::Reveal),
            ("H", PageControl::Hold),
            ("m", PageControl::Mix),
        ] {
            browser.key(Key::Text(text), Modifiers::default(), OPEN);
            assert_eq!(browser.take_outputs(), [Output::PageControl(control)], "{text}");
        }
        browser.key(Key::Right, Modifiers::default(), OPEN);
        browser.key(Key::Left, Modifiers::default(), OPEN);
        assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::SubPage(true)), Output::PageControl(PageControl::SubPage(false))]);
        let (mut single, _) = opened(1);
        single.key(Key::Right, Modifiers::default(), OPEN);
        assert!(single.take_outputs().is_empty(), "one sub-page: the arrow sizes the pages as before");
    }

    /// The sub-page control shows on a page with sub-pages, and its parts
    /// ask for a step or the latest.
    #[test]
    fn the_sub_page_control_steps_and_returns_to_the_latest() {
        let (mut browser, key) = opened(4);
        browser.set_page_controls(PageControls { page: Some(key.page), subpages: 4, chosen: Some(1), ..PageControls::default() });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
        assert!(canvas.texts.iter().any(|(_, text)| text == "2/4"), "the second of four");
        let middle = |rect: Rect| Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
        let ui = &browser.page_controls;
        let (previous, count, next) = (ui.previous, ui.count, ui.next);
        for (rect, control) in [(previous, PageControl::SubPage(false)), (next, PageControl::SubPage(true)), (count, PageControl::Latest)] {
            assert!(browser.controls_press(middle(rect)));
            assert_eq!(browser.take_outputs(), [Output::PageControl(control)]);
        }
        let (mut single, _) = opened(1);
        single.draw(&mut Recorder::new(), WIDTH, HEIGHT, OPEN);
        assert_eq!(single.page_controls.count, Rect::ZERO, "no control on a page of one sub-page");
    }
}
