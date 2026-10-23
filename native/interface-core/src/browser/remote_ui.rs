//! The remote control (`remote-control.md`, decision 68): a teletext set's
//! remote as a panel docked along the window's right edge, sliding in and
//! out, or floating over the page area. Every button is an input that
//! already exists, a digit, a page step or a page control, so the remote
//! adds a place to press, never a power.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Browser, Output};
use crate::canvas::Canvas;
use crate::controls::{FastextKey, PageControl, PageControls};
use crate::geometry::{Point, Rect};
use crate::settings::{MAX_REMOTE_PLACE, RemoteMode, RemotePlace, SettingChange};
use crate::theme::{BUTTON, CONTROL, CYAN, DIM, GREEN, HEADING, PANEL, READOUT, RED, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE, YELLOW};
use crate::tween::STANDARD;
use crate::ui::{border, centred, left_centred, outline};

/// The panel's width, as a remote is held: three keys of the number pad
/// across, with room round them.
pub(super) const REMOTE_WIDTH: f64 = 232.0;
const PADDING: f64 = 12.0;
const GAP: f64 = 6.0;
const TITLE_HEIGHT: f64 = 34.0;
const READOUT_HEIGHT: f64 = 46.0;
const KEY_HEIGHT: f64 = 40.0;
const SMALL_KEY_HEIGHT: f64 = 32.0;
const TITLE_BUTTON: f64 = 30.0;
const RADIUS: f64 = 6.0;
/// The handle at the window's right edge that brings the remote out.
const HANDLE_WIDTH: f64 = 14.0;
const HANDLE_HEIGHT: f64 = 72.0;
/// How long the docked remote takes to slide in or out.
const SLIDE_MILLISECONDS: u64 = 200;
/// The number pad: three keys a row, 1 to 9 then 0 below the 8, as on a
/// remote and a telephone.
const PAD_COLUMNS: usize = 3;
const PAD: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];

/// A button of the remote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Button {
    Digit(char),
    PageUp,
    PageDown,
    Control(PageControl),
    /// The count between the sub-page steps: back to the latest.
    Latest,
    Dock,
    Close,
}

/// The remote's state in the browser.
#[derive(Debug, Default)]
pub(super) struct RemoteUi {
    /// Where it is, as the user last put it.
    place: RemotePlace,
    /// The docked remote's slide: how far out it was and when it started
    /// moving, towards its place's mode.
    slide: Option<(f32, u64)>,
    /// While a floating remote is dragged: the pointer's offset from its
    /// corner.
    drag: Option<(f64, f64)>,
    /// The floating remote's corner while dragged.
    dragged: Option<(f64, f64)>,
    panel: Rect,
    title: Rect,
    handle: Rect,
    buttons: Vec<(Button, Rect)>,
    /// The time of the last frame drawn: a place the trusted core sends
    /// slides from it.
    drawn_at: u64,
}

impl RemoteUi {
    /// How far out the docked remote is at `now`, from 0 (away) to 1.
    fn out(&self, now: u64) -> f32 {
        let target = if self.place.mode == RemoteMode::Docked { 1.0 } else { 0.0 };
        let Some((from, start)) = self.slide else { return target };
        #[allow(clippy::cast_precision_loss, reason = "a fraction of a short slide")]
        let progress = (now.saturating_sub(start) as f32 / SLIDE_MILLISECONDS as f32).min(1.0);
        from + (target - from) * STANDARD.at(progress)
    }
}

impl Browser {
    /// Where the remote is, as the trusted core keeps it.
    pub fn set_remote(&mut self, place: RemotePlace) {
        if self.remote.place != place && self.remote.drag.is_none() {
            self.move_remote(place, self.remote.drawn_at);
        }
    }

    /// Puts the remote at `place` from `now`, sliding when it docks or
    /// leaves the dock.
    fn move_remote(&mut self, place: RemotePlace, now: u64) {
        let from = self.remote.out(now);
        self.remote.place = place;
        self.remote.slide = Some((from, now));
    }

    /// The user put the remote somewhere: kept by the trusted core.
    fn put_remote(&mut self, place: RemotePlace, now: u64) {
        self.move_remote(place, now);
        self.outputs.push(Output::Setting(SettingChange::Remote(place)));
    }

    /// Its shortcut, the menu and the handle: shown docked, or put away.
    pub(super) fn toggle_remote(&mut self, now: u64) {
        let mode = if self.remote.place.mode == RemoteMode::Hidden { RemoteMode::Docked } else { RemoteMode::Hidden };
        self.put_remote(RemotePlace { mode, ..self.remote.place }, now);
    }

    /// The time of the last frame drawn, for a change made from a menu.
    pub(super) const fn remote_time(&self) -> u64 {
        self.remote.drawn_at
    }

    /// Whether the remote shows, docked or floating.
    #[must_use]
    pub fn remote_shown(&self) -> bool {
        self.remote.place.mode != RemoteMode::Hidden
    }

    /// While the docked remote slides, frames at the display's pace.
    pub(super) fn remote_animating(&self, now: u64) -> bool {
        self.remote.slide.is_some_and(|(_, start)| now.saturating_sub(start) < SLIDE_MILLISECONDS)
    }

    /// The width the docked remote takes from the window's content at
    /// `now`, with its margin.
    pub(super) fn remote_docked_width(&self, now: u64) -> f64 {
        f64::from(self.remote.out(now)) * (REMOTE_WIDTH + super::MARGIN)
    }

    /// A press on the remote or its handle; returns whether it was there.
    pub(super) fn remote_press(&mut self, point: Point, now: u64) -> bool {
        if self.remote.handle.contains(point) {
            self.toggle_remote(now);
            return true;
        }
        if !self.remote.panel.contains(point) {
            return false;
        }
        let Some(&(button, _)) = self.remote.buttons.iter().find(|(_, rect)| rect.contains(point)) else {
            // The title row of a floating remote moves it.
            if self.remote.place.mode == RemoteMode::Floating && self.remote.title.contains(point) {
                let panel = self.remote.panel;
                self.remote.drag = Some((point.x - panel.x0, point.y - panel.y0));
                self.remote.dragged = Some((panel.x0, panel.y0));
            }
            return true;
        };
        match button {
            Button::Digit(digit) => {
                self.stop_editing();
                self.type_digit(digit);
            }
            Button::PageUp => self.step_page(false, now),
            Button::PageDown => self.step_page(true, now),
            Button::Control(control) => self.ask_control(control),
            Button::Latest => self.ask_control(PageControl::Latest),
            Button::Dock => {
                let place = self.remote.place;
                let mode = if place.mode == RemoteMode::Floating { RemoteMode::Docked } else { RemoteMode::Floating };
                // Undocked, it floats where it stood.
                let (x, y) = if mode == RemoteMode::Floating { corner(self.remote.panel) } else { (place.x, place.y) };
                self.put_remote(RemotePlace { mode, x, y }, now);
            }
            Button::Close => self.put_remote(RemotePlace { mode: RemoteMode::Hidden, ..self.remote.place }, now),
        }
        true
    }

    /// The pointer moved: a floating remote being dragged follows it.
    pub(super) fn remote_moved(&mut self, point: Point) -> bool {
        let Some((dx, dy)) = self.remote.drag else { return false };
        self.remote.dragged = Some((point.x - dx, point.y - dy));
        true
    }

    /// The button was released: a drag ends where the remote stands.
    pub(super) fn remote_released(&mut self) {
        if self.remote.drag.take().is_none() {
            return;
        }
        let (x, y) = corner(self.remote.panel);
        self.remote.dragged = None;
        self.put_remote(RemotePlace { mode: RemoteMode::Floating, x, y }, self.remote.drawn_at);
    }

    /// The remote, docked at the right of the body or floating over it,
    /// and its handle; `top` is the body's top, below the strip and the
    /// bookmarks bar, which it never covers (requirement 11.4).
    pub(super) fn draw_remote<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, height: f64, top: f64, now: u64) {
        self.remote.drawn_at = now;
        if !self.remote_animating(now) {
            self.remote.slide = None;
        }
        self.remote.buttons.clear();
        self.remote.panel = Rect::ZERO;
        self.remote.title = Rect::ZERO;
        self.remote.handle = Rect::ZERO;
        let bottom = height - super::MARGIN;
        let out = self.remote_docked_width(now);
        let panel_height = panel_height().min(bottom - top);
        let panel = match self.remote.place.mode {
            RemoteMode::Floating => {
                let (x, y) = self.remote.dragged.unwrap_or((f64::from(self.remote.place.x), f64::from(self.remote.place.y)));
                // Kept inside the window, below the strip.
                let x = x.clamp(super::MARGIN, (width - super::MARGIN - REMOTE_WIDTH).max(super::MARGIN));
                let y = y.clamp(top, (bottom - panel_height).max(top));
                Some(Rect::new(x, y, x + REMOTE_WIDTH, y + panel_height))
            }
            _ if out > 0.0 => {
                let x = width - out;
                Some(Rect::new(x, top, x + REMOTE_WIDTH, bottom))
            }
            _ => None,
        };
        // The handle, while the remote is not floating: beside the docked
        // remote, or at the window's edge.
        if self.remote.place.mode != RemoteMode::Floating {
            let x = width - out - HANDLE_WIDTH;
            let y = f64::midpoint(top, bottom) - HANDLE_HEIGHT / 2.0;
            self.remote.handle = Rect::new(x, y, x + HANDLE_WIDTH, y + HANDLE_HEIGHT);
            canvas.fill(self.remote.handle, RADIUS, BUTTON);
            border(canvas, self.remote.handle, 1.0, RULE);
            let mark = if self.remote_shown() { "\u{203A}" } else { "\u{2039}" };
            centred(canvas, self.remote.handle, mark, ROW_TITLE, WHITE);
        }
        let Some(panel) = panel else { return };
        self.remote.panel = panel;
        canvas.fill(panel, RADIUS, PANEL);
        outline(canvas, panel, RULE);
        canvas.push_clip(panel);
        self.draw_remote_keys(canvas, panel);
        canvas.pop_clip();
    }

    #[allow(clippy::too_many_lines, reason = "one row of the remote after another, top to bottom")]
    fn draw_remote_keys<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect) {
        let controls = self.page_controls();
        let open = self.wall.zoomed().is_some_and(|key| controls.page == Some(key.page));
        let (left, right) = (panel.x0 + PADDING, panel.x1 - PADDING);
        let mut y = panel.y0 + PADDING;
        let mut buttons: Vec<(Button, Rect)> = Vec::new();

        // The title row: its name, dock or float, and away.
        let title = Rect::new(panel.x0, panel.y0, panel.x1, y + TITLE_HEIGHT);
        self.remote.title = title;
        left_centred(canvas, Rect::new(left, y, right, y + TITLE_HEIGHT), 0.0, "Remote", HEADING, WHITE);
        let close = Rect::new(right - TITLE_BUTTON, y + 2.0, right, y + 2.0 + TITLE_BUTTON);
        let dock = Rect::new(close.x0 - GAP - TITLE_BUTTON, close.y0, close.x0 - GAP, close.y1);
        let floating = self.remote.place.mode == RemoteMode::Floating;
        key(canvas, dock, if floating { "\u{21E5}" } else { "\u{2197}" }, ROW_TITLE, false, true);
        key(canvas, close, "\u{00D7}", ROW_TITLE, false, true);
        buttons.push((Button::Dock, dock));
        buttons.push((Button::Close, close));
        y += TITLE_HEIGHT + GAP;

        // The page number: the digits being entered, or the open page.
        let readout = Rect::new(left, y, right, y + READOUT_HEIGHT);
        canvas.fill(readout, RADIUS, BUTTON);
        border(canvas, readout, 1.0, RULE);
        let number = if self.digits.is_empty() {
            controls.page.filter(|_| open).map_or_else(|| String::from("P---"), |page| format!("P{page:03X}"))
        } else {
            format!("P{:-<3}", self.digits)
        };
        centred(canvas, readout, &number, READOUT, WHITE);
        y += READOUT_HEIGHT + GAP;

        // The number pad.
        let key_width = (right - left - GAP * 2.0) / 3.0;
        for (index, digit) in PAD.iter().enumerate() {
            // 0 sits below the 8, alone on its row.
            let (row, column) = if index == PAD.len() - 1 { (3, 1) } else { (index / PAD_COLUMNS, index % PAD_COLUMNS) };
            #[allow(clippy::cast_precision_loss, reason = "a few keys")]
            let (x, top) = (left + column as f64 * (key_width + GAP), y + row as f64 * (KEY_HEIGHT + GAP));
            let rect = Rect::new(x, top, x + key_width, top + KEY_HEIGHT);
            key(canvas, rect, digit, CONTROL, false, true);
            buttons.push((Button::Digit(digit.chars().next().unwrap_or('0')), rect));
        }
        y += 4.0 * (KEY_HEIGHT + GAP);

        // Page up and down.
        let half = (right - left - GAP) / 2.0;
        let up = Rect::new(left, y, left + half, y + SMALL_KEY_HEIGHT);
        let down = Rect::new(up.x1 + GAP, y, right, y + SMALL_KEY_HEIGHT);
        key(canvas, up, "Page \u{25B2}", ROW_TITLE, false, true);
        key(canvas, down, "Page \u{25BC}", ROW_TITLE, false, true);
        buttons.push((Button::PageUp, up));
        buttons.push((Button::PageDown, down));
        y += SMALL_KEY_HEIGHT + GAP * 2.0;

        // Fastext: the four coloured keys, and Index below.
        let quarter = (right - left - GAP * 3.0) / 4.0;
        for (index, colour) in [RED, GREEN, YELLOW, CYAN].into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "four keys")]
            let x = left + index as f64 * (quarter + GAP);
            let rect = Rect::new(x, y, x + quarter, y + SMALL_KEY_HEIGHT);
            let lit = open && controls.fastext.get(index).copied().unwrap_or(false);
            canvas.fill(rect, RADIUS, if lit { colour } else { BUTTON });
            border(canvas, rect, 2.0, colour);
            if let Some(fastext) = FastextKey::at(index) {
                buttons.push((Button::Control(PageControl::Fastext(fastext)), rect));
            }
        }
        y += SMALL_KEY_HEIGHT + GAP;
        let index = Rect::new(left, y, right, y + SMALL_KEY_HEIGHT);
        let index_lit = open && controls.fastext.get(FastextKey::Index.index()).copied().unwrap_or(false);
        key(canvas, index, "Index", ROW_TITLE, false, index_lit);
        buttons.push((Button::Control(PageControl::Fastext(FastextKey::Index)), index));
        y += SMALL_KEY_HEIGHT + GAP * 2.0;

        // Sub-pages: the one before, the count, the next.
        let steps = open && controls.subpages > 1;
        let step = SMALL_KEY_HEIGHT;
        let previous = Rect::new(left, y, left + step, y + SMALL_KEY_HEIGHT);
        let next = Rect::new(right - step, y, right, y + SMALL_KEY_HEIGHT);
        let count = Rect::new(previous.x1 + GAP, y, next.x0 - GAP, y + SMALL_KEY_HEIGHT);
        key(canvas, previous, "\u{2039}", ROW_TITLE, false, steps);
        key(canvas, next, "\u{203A}", ROW_TITLE, false, steps);
        let words = subpage_words(&controls, steps);
        key(canvas, count, &words, ROW_STATUS, false, steps);
        buttons.push((Button::Control(PageControl::SubPage(false)), previous));
        buttons.push((Button::Control(PageControl::SubPage(true)), next));
        buttons.push((Button::Latest, count));
        y += SMALL_KEY_HEIGHT + GAP;

        // Reveal, hold and mix: lit while on.
        let third = (right - left - GAP * 2.0) / 3.0;
        for (index, (words, on, control)) in
            [("Reveal", controls.reveal, PageControl::Reveal), ("Hold", controls.hold, PageControl::Hold), ("Mix", controls.mix, PageControl::Mix)]
                .into_iter()
                .enumerate()
        {
            #[allow(clippy::cast_precision_loss, reason = "three keys")]
            let x = left + index as f64 * (third + GAP);
            let rect = Rect::new(x, y, x + third, y + SMALL_KEY_HEIGHT);
            key(canvas, rect, words, ROW_STATUS, open && on, open);
            buttons.push((Button::Control(control), rect));
        }
        self.remote.buttons = buttons;
    }

    /// The remote's tips (requirement 12.12).
    pub(super) fn remote_tips(&self, tips: &mut Vec<(Rect, String)>) {
        if self.remote.handle != Rect::ZERO {
            let words = if self.remote_shown() { "Put the remote control away" } else { "The remote control" };
            tips.push((self.remote.handle, format!("{words} ({})", self.keys_named("Shift+Cmd+R", "Ctrl+Shift+R"))));
        }
        for &(button, rect) in &self.remote.buttons {
            let words = match button {
                Button::Digit(_) => continue,
                Button::PageUp => String::from("The page before (Up)"),
                Button::PageDown => String::from("The next page (Down)"),
                Button::Control(PageControl::Fastext(FastextKey::Index)) => String::from("The page's Index link (i)"),
                Button::Control(PageControl::Fastext(key)) => {
                    let name = key.name().to_lowercase();
                    let letter = name.chars().next().unwrap_or(' ');
                    format!("The page's {name} link ({letter}, or F{})", key.index() + 1)
                }
                Button::Control(PageControl::Reveal) => String::from("Show concealed text, or hide it again (?)"),
                Button::Control(PageControl::Hold) => String::from("Hold the page as it is (Shift+H)"),
                Button::Control(PageControl::Mix) => String::from("Text over a clear background (m)"),
                Button::Control(PageControl::SubPage(false)) => String::from("The sub-page before (Left)"),
                Button::Control(PageControl::SubPage(true)) => String::from("The next sub-page (Right)"),
                Button::Control(PageControl::Latest) | Button::Latest => String::from("Back to the sub-pages as they arrive"),
                Button::Dock if self.remote.place.mode == RemoteMode::Floating => String::from("Dock the remote at the right"),
                Button::Dock => String::from("Float the remote over the page"),
                Button::Close => String::from("Put the remote control away"),
            };
            tips.push((rect, words));
        }
        if self.remote.place.mode == RemoteMode::Floating && self.remote.title != Rect::ZERO {
            tips.push((
                Rect::new(self.remote.title.x0, self.remote.title.y0, self.remote.title.x0 + REMOTE_WIDTH / 2.0, self.remote.title.y1),
                String::from("Drag to move the remote"),
            ));
        }
    }
}

/// The panel's height: every row of the remote.
fn panel_height() -> f64 {
    PADDING * 2.0 + TITLE_HEIGHT + GAP + READOUT_HEIGHT + GAP + 4.0 * (KEY_HEIGHT + GAP) + 5.0 * (SMALL_KEY_HEIGHT + GAP) + GAP * 2.0
}

/// A panel's corner, as kept: whole points within the bound.
fn corner(panel: Rect) -> (u16, u16) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the bound first")]
    let point = |value: f64| value.clamp(0.0, f64::from(MAX_REMOTE_PLACE)) as u16;
    (point(panel.x0), point(panel.y0))
}

/// What the sub-page count says.
fn subpage_words(controls: &PageControls, steps: bool) -> String {
    if !steps {
        return String::from("Sub-pages");
    }
    match controls.chosen {
        Some(place) => format!("{}/{}", u16::from(place) + 1, controls.subpages),
        None => format!("{} \u{21BB}", controls.subpages),
    }
}

/// One key of the remote: lit while `on`, dim while it does nothing.
fn key<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, words: &str, style: crate::canvas::Style, on: bool, live: bool) {
    canvas.fill(rect, RADIUS, if on { SELECTED } else { BUTTON });
    border(canvas, rect, 1.0, if on { SELECTED } else { RULE });
    centred(canvas, rect, words, style, if live { WHITE } else { DIM });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use crate::browser::{Frame, Key, Modifiers};
    use crate::test_canvas::Recorder;

    const WIDTH: f64 = 1440.0;
    const HEIGHT: f64 = 900.0;

    fn middle(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    fn rect_of(browser: &Browser, wanted: Button) -> Rect {
        browser.remote.buttons.iter().find(|(button, _)| *button == wanted).unwrap().1
    }

    /// Decision 68: hidden at first, its shortcut docks it and the trusted
    /// core is told; docked it narrows the body, sliding in; its handle
    /// puts it away again.
    #[test]
    fn the_remote_docks_slides_and_goes_away() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
        let full = browser.body;
        assert!(!browser.remote_shown() && browser.remote.panel == Rect::ZERO);
        browser.key(Key::Text("r"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
        let docked = RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 };
        assert_eq!(browser.take_outputs(), [Output::Setting(SettingChange::Remote(docked))]);
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, SLIDE_MILLISECONDS / 2);
        assert!(browser.remote_animating(SLIDE_MILLISECONDS / 2), "sliding in");
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, SLIDE_MILLISECONDS);
        assert!((full.x1 - browser.body.x1 - (REMOTE_WIDTH + super::super::MARGIN)).abs() < 0.5, "the body narrows by the remote");
        let handle = browser.remote.handle;
        assert!(browser.remote_press(middle(handle), SLIDE_MILLISECONDS));
        assert_eq!(browser.take_outputs(), [Output::Setting(SettingChange::Remote(RemotePlace { mode: RemoteMode::Hidden, x: 0, y: 0 }))]);
    }

    /// Its keys are the inputs that already exist: digits choose a page,
    /// the page keys step, the controls go to the trusted core.
    #[test]
    fn the_remotes_keys_are_the_existing_inputs() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_remote(RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 });
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, SLIDE_MILLISECONDS);
        for digit in ['3', '0'] {
            let key = rect_of(&browser, Button::Digit(digit));
            browser.remote_press(middle(key), SLIDE_MILLISECONDS);
        }
        assert_eq!(browser.digits, "30", "two digits of three entered");
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, SLIDE_MILLISECONDS);
        assert!(canvas.texts.iter().any(|(_, text)| text == "P30-"), "the readout shows the entry");
        for control in [PageControl::Reveal, PageControl::Fastext(FastextKey::Yellow), PageControl::SubPage(true)] {
            let key = rect_of(&browser, Button::Control(control));
            browser.remote_press(middle(key), SLIDE_MILLISECONDS);
            assert_eq!(browser.take_outputs(), [Output::PageControl(control)]);
        }
    }

    /// Floating, it moves by its title row, stays below the strip and in
    /// the window, and the trusted core keeps where it was put.
    #[test]
    fn a_floating_remote_moves_and_stays_below_the_strip() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_remote(RemotePlace { mode: RemoteMode::Floating, x: 600, y: 300 });
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
        let top = browser.body.y0;
        let title = browser.remote.title;
        let grip = Point::new(title.x0 + 20.0, f64::midpoint(title.y0, title.y1));
        assert!(browser.remote_press(grip, 0));
        browser.pointer_moved(grip.x - 200.0, -500.0, 0);
        browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
        assert!((browser.remote.panel.y0 - top).abs() < 0.5, "held below the strip");
        browser.pointer_released();
        let outputs = browser.take_outputs();
        let Some(Output::Setting(SettingChange::Remote(place))) = outputs.last() else { panic!("{outputs:?}") };
        assert_eq!((place.mode, f64::from(place.y)), (RemoteMode::Floating, top.floor()));
        let dock = rect_of(&browser, Button::Dock);
        browser.remote_press(middle(dock), 0);
        assert!(matches!(browser.take_outputs().as_slice(), [Output::Setting(SettingChange::Remote(RemotePlace { mode: RemoteMode::Docked, .. }))]));
    }
}
