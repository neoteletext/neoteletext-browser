//! The remote control (`remote-control.md`, decision 68): a teletext set's
//! remote as a panel docked along the window's right edge, sliding in and
//! out, or floating over the page area. Every button is an input that
//! already exists, a digit, a page step, an arrow, OK or a page control, so
//! the remote adds a place to press, never a power. While keys go to a
//! channel, its digits, colours, arrows, OK and page keys go to the channel
//! as the keyboard's same keys would (REQ-1373).

use alloc::vec::Vec;

use crate::browser::{Browser, Key, Modifiers, Output};
use crate::controls::PageControl;
use crate::geometry::{Point, Rect};
use crate::settings::{MAX_REMOTE_PLACE, RemoteMode, RemotePlace, SettingChange};
use crate::tween::STANDARD;

mod draw;

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
    /// The up arrow, or with `false` the down arrow (REQ-1373).
    Arrow(bool),
    /// OK, the keyboard's Enter (REQ-1373).
    Ok,
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
        // With reduced motion the remote is in its place at once (REQ-754).
        self.remote.slide = (!self.wall.reduced_motion()).then_some((from, now));
    }

    /// The user put the remote somewhere: kept by the trusted core.
    fn put_remote(&mut self, place: RemotePlace, now: u64) {
        self.move_remote(place, now);
        self.outputs.push(Output::Setting(SettingChange::Remote(place)));
    }

    /// Its shortcut, the menu and the handle: shown docked, or put away.
    pub(in crate::browser) fn toggle_remote(&mut self, now: u64) {
        let mode = if self.remote.place.mode == RemoteMode::Hidden { RemoteMode::Docked } else { RemoteMode::Hidden };
        self.put_remote(RemotePlace { mode, ..self.remote.place }, now);
    }

    /// The time of the last frame drawn, for a change made from a menu.
    pub(in crate::browser) const fn remote_time(&self) -> u64 {
        self.remote.drawn_at
    }

    /// Whether the remote shows, docked or floating.
    #[must_use]
    pub fn remote_shown(&self) -> bool {
        self.remote.place.mode != RemoteMode::Hidden
    }

    /// While the docked remote slides, frames at the display's pace, up
    /// to the frame that draws it at rest, which ends the slide: a frame
    /// drawn part of the way owes the next, however late it is asked for.
    pub(in crate::browser) fn remote_animating(&self, _now: u64) -> bool {
        self.remote.slide.is_some()
    }

    /// Whether the docked remote is still on its way at `now`.
    fn remote_sliding(&self, now: u64) -> bool {
        self.remote.slide.is_some_and(|(_, start)| now.saturating_sub(start) < SLIDE_MILLISECONDS)
    }

    /// The width the docked remote takes from the window's content at
    /// `now`, with its margin.
    pub(in crate::browser) fn remote_docked_width(&self, now: u64) -> f64 {
        f64::from(self.remote.out(now)) * (REMOTE_WIDTH + crate::browser::window::MARGIN)
    }

    /// A press on the remote or its handle; returns whether it was there.
    /// Whether `point` is on the remote or its handle.
    pub(in crate::browser) fn over_remote(&self, point: Point) -> bool {
        self.remote.handle.contains(point) || self.remote.panel.contains(point)
    }

    pub(in crate::browser) fn remote_press(&mut self, point: Point, now: u64) -> bool {
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
        // While keys go to a channel its keys are the channel's (REQ-1373).
        #[cfg(feature = "interaction")]
        if self.remote_key_to_channel(button) {
            return true;
        }
        match button {
            Button::Arrow(up) => {
                self.key(if up { Key::Up } else { Key::Down }, Modifiers::default(), now);
            }
            Button::Ok => {
                self.key(Key::Enter, Modifiers::default(), now);
            }
            Button::Digit(digit) => {
                self.stop_editing();
                self.type_digit(digit, now);
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
    pub(in crate::browser) fn remote_moved(&mut self, point: Point) -> bool {
        let Some((dx, dy)) = self.remote.drag else { return false };
        self.remote.dragged = Some((point.x - dx, point.y - dy));
        true
    }

    /// The button was released: a drag ends where the remote stands.
    pub(in crate::browser) fn remote_released(&mut self) {
        if self.remote.drag.take().is_none() {
            return;
        }
        let (x, y) = corner(self.remote.panel);
        self.remote.dragged = None;
        self.put_remote(RemotePlace { mode: RemoteMode::Floating, x, y }, self.remote.drawn_at);
    }
}

/// The panel's height: every row of the remote.
fn panel_height() -> f64 {
    PADDING * 2.0 + TITLE_HEIGHT + GAP + READOUT_HEIGHT + GAP + 4.0 * (KEY_HEIGHT + GAP) + 6.0 * (SMALL_KEY_HEIGHT + GAP) + GAP * 2.0
}

/// The keyboard's key a remote button is while keys go to a channel, by
/// its name in the input protocol's table (DOC-1936), and the text it
/// types: a digit its own, the rest none (REQ-1373). The title row's
/// buttons, Index, Reveal, Hold, Mix and the sub-page count are the
/// browser's own and have none.
#[cfg(feature = "interaction")]
fn keyboard_key(button: Button) -> Option<(neoteletext_input::event::KeyCode, Option<char>)> {
    use crate::controls::FastextKey;
    use neoteletext_input::event::KeyCode;
    let name = match button {
        Button::Digit(digit) => return Some((KeyCode::Character(digit), Some(digit))),
        Button::PageUp => "PageUp",
        Button::PageDown => "PageDown",
        Button::Arrow(true) => "ArrowUp",
        Button::Arrow(false) => "ArrowDown",
        Button::Control(PageControl::SubPage(false)) => "ArrowLeft",
        Button::Control(PageControl::SubPage(true)) => "ArrowRight",
        Button::Ok => "Enter",
        Button::Control(PageControl::Fastext(FastextKey::Red)) => "ColorF0Red",
        Button::Control(PageControl::Fastext(FastextKey::Green)) => "ColorF1Green",
        Button::Control(PageControl::Fastext(FastextKey::Yellow)) => "ColorF2Yellow",
        Button::Control(PageControl::Fastext(FastextKey::Cyan)) => "ColorF3Blue",
        _ => return None,
    };
    Some((KeyCode::Functional(neoteletext_input::keys::code(name)?), None))
}

impl Browser {
    /// A remote button pressed while keys go to a channel: a key the
    /// keyboard has, and the page asks for (REQ-7037), goes to the
    /// channel, pressed then released, as `terminal_key` sends the
    /// keyboard's (REQ-1373). Returns whether it went; a key the page does
    /// not ask for is the remote's own to act on.
    #[cfg(feature = "interaction")]
    fn remote_key_to_channel(&mut self, button: Button) -> bool {
        use neoteletext_input::event::Action;
        let Some((key, text)) = keyboard_key(button) else { return false };
        let mut typed = [0_u8; 4];
        let text = text.map(|character| &*character.encode_utf8(&mut typed));
        if self.key_target(Action::Press, key, 0, text, None).is_none() {
            return false;
        }
        self.terminal_key(Action::Press, key, 0, text, None);
        self.terminal_key(Action::Release, key, 0, None, None);
        true
    }
}

/// A panel's corner, as kept: whole points within the bound.
fn corner(panel: Rect) -> (u16, u16) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the bound first")]
    let point = |value: f64| value.clamp(0.0, f64::from(MAX_REMOTE_PLACE)) as u16;
    (point(panel.x0), point(panel.y0))
}

#[cfg(test)]
mod tests;
