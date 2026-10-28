//! Typing into a page (`neoteletext-common/INPUT.md` section 9): while
//! the page open is a page of a channel that takes input, the viewer's
//! keys are the channel's, as a terminal's are its host's, and the mouse
//! and the contacts of a touch screen on the page too. The browser draws
//! the channel's cursor, and in the strip who is listening; what is typed
//! shows only as the channel draws it. The trusted core decides whether input goes, paces
//! the pointer and encodes it; this only gives it intents (REQ-INT-132,
//! `RETURN-CHANNEL.md` REQ-RCH-036).

use alloc::string::String;
use alloc::vec::Vec;

use neoteletext_input::event::{Action, KeyCode, MouseAction, Point, printable};
use neoteletext_input::{MAX_COMPOSE, MAX_PASTE};

use crate::browser::{Browser, Output};
use crate::canvas::{CELL_HEIGHT, CELL_WIDTH, PAGE_HEIGHT, PAGE_WIDTH, PageKey};
use crate::channel::page_index;
use crate::input::{Intent, TouchPhase, mouse_button};
use crate::wall::cell_in;

mod draw;

/// The cursor blinks at this half period, as common terminals do.
pub const BLINK_MILLISECONDS: u64 = 530;
/// A bar cursor's width as a share of a cell.
const BAR_SHARE: f64 = 0.15;
/// A block cursor's outline as a share of a cell's width.
const BLOCK_LINE_SHARE: f64 = 0.12;
/// The wheel's unit: 1/120th of a line (INPUT.md section 5.4).
const WHEEL_UNITS_PER_LINE: f64 = 120.0;
/// Turns of a wheel that moves in notches, in lines, as a channel gets
/// them: at least a whole line a notch (INPUT.md section 5.4). macOS
/// reports a slow notch as a tenth of a line, which a list that moves a
/// line at a time would need ten notches to show; a faster notch, past a
/// line, keeps its acceleration. A trackpad's pixels are not notches and
/// are not changed.
#[must_use]
pub fn notched(lines: f64) -> f64 {
    if lines != 0.0 && lines.abs() < 1.0 { lines.signum() } else { lines }
}

/// Mouse buttons tracked as held, as section 5.4's `button` numbers them:
/// a move's mask of the buttons held is a byte.
const BUTTONS: u8 = 8;
// Every button the browser sends is tracked.
const _: () = assert!(mouse_button::FORWARD < BUTTONS);

/// The cursor a channel placed, as the trusted core checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorView {
    /// 1 to 23.
    pub row: u8,
    /// 0 to 39.
    pub column: u8,
    /// Block, as the engine's cursor record says.
    pub flags: u8,
}

/// Whether keys go to the open page's channel, as the trusted core said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypingView {
    /// The channel.
    pub channel: u32,
    /// Its open page.
    pub page: u16,
    /// Keys go to it now.
    pub on: bool,
    /// They go encrypted.
    pub secure: bool,
    /// The channel did not answer, or refused: it takes no input.
    pub refused: bool,
    /// The cursor, if the channel placed one on this page.
    pub cursor: Option<CursorView>,
    /// The push connection's last round trip, in microseconds
    /// (REQ-RCH-016).
    pub round_trip_us: Option<u32>,
    /// Another page of the channel its host went to (REQ-INT-113): the
    /// browser shows it.
    pub show: Option<u16>,
}

/// The browser's typing state.
#[derive(Debug, Default)]
pub(super) struct TerminalUi {
    typing: Option<TypingView>,
    /// The browser's own modifier is held: Command on macOS, Control and
    /// Shift elsewhere. A click is then the browser's, not the host's.
    browser_modifier: bool,
    /// When the cursor last moved, for its blink.
    moved: u64,
    /// The mouse buttons held on the page, bit `n` for button `n`.
    buttons: u8,
    /// The modifiers held, as section 5.5 numbers them, which go with
    /// every press, release, move and wheel turn (REQ-RCH-005).
    mods: u8,
    /// Where the pointer was last given as on the open page: a move goes
    /// for each change of it, and its leaving the page is gone
    /// (REQ-RCH-030, REQ-RCH-034).
    last_dot: Option<Point>,
    /// The contacts given to the channel, each where it was last given:
    /// at most `MAX_CONTACTS`, the most the trusted core numbers.
    touches: Vec<(u64, Point)>,
    /// The contacts that began on a link, and which: the browser's, never
    /// the channel's, followed if they lift on it (REQ-RCH-048).
    link_touches: Vec<(u64, (PageKey, usize))>,
}

/// The longest prefix of `text` of at most `limit` bytes, whole characters.
fn cut(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or("")
}

impl Browser {
    /// The trusted core's word on typing for the open page.
    pub fn set_typing(&mut self, typing: TypingView, now: u64) {
        let state = &mut self.typing;
        let moved = state.typing.and_then(|old| old.cursor) != typing.cursor;
        if !typing.on {
            // The trusted core sends gone for what the channel held.
            state.buttons = 0;
            state.last_dot = None;
            state.touches.clear();
            state.link_touches.clear();
        }
        if moved {
            state.moved = now;
        }
        state.typing = Some(typing);
        // Whether a click goes to the channel decides what lights.
        #[cfg(feature = "links")]
        self.relight();
        // The host went to another of its pages: it is shown, as a
        // terminal shows what its host sends. Only a page of the same
        // channel, listed, while keys go to it.
        if let Some(page) = typing.show.filter(|_| typing.on && typing.channel == self.channel.channel) {
            let listed = page_index(page).filter(|&index| matches!(self.channel.pages.get(index), Some(Some(_))));
            if let Some(index) = listed {
                self.select_page(index, true);
            }
        }
    }

    /// The page keys go to now, if any: the open page, while the trusted
    /// core says its channel takes them.
    #[must_use]
    pub fn typing_target(&self) -> Option<PageKey> {
        let typing = self.typing.typing.filter(|typing| typing.on)?;
        let key = PageKey { channel: typing.channel, page: typing.page };
        // While a panel, popover or list of the strip is open, the keys are
        // the strip's (`history-and-bookmarks.md` section 2.4).
        // While a list at the left has them too.
        (self.opened() == Some(key) && !self.editing && !self.strip_has_keys() && self.list_focus.is_none()).then_some(key)
    }

    /// Whether a panel, popover or list of the strip is open, which then
    /// takes the keys rather than the page's channel.
    pub(super) fn strip_has_keys(&self) -> bool {
        self.settings_open()
            || self.history_open()
            || self.cookies_editing()
            || self.console_searching()
            || self.bookmarks_open()
            || self.star_open()
            || self.popup_open()
    }

    /// Whether the input method's preedit goes to the channel (INPUT.md
    /// sections 6.3 and 9.1): whenever keys go to it.
    #[must_use]
    pub fn composes_for_channel(&self) -> bool {
        self.typing_target().is_some()
    }

    /// The browser's own modifier is held or released.
    pub fn set_browser_modifier(&mut self, held: bool) {
        self.typing.browser_modifier = held;
    }

    /// The modifiers held now, as INPUT.md section 5.5 numbers them: the
    /// mouse's events carry them (section 5.4, REQ-RCH-005).
    pub fn set_mouse_modifiers(&mut self, mods: u8) {
        self.typing.mods = mods;
    }

    /// Where the pointer is on the open page, while it is over it: the cell
    /// and the page image's pixel.
    fn pointer_on_page(&self) -> Option<Point> {
        self.page_point(self.pointer)
    }

    /// The dot of the open page's image under `point`, and the cell it
    /// lies in (INPUT.md section 9.9): the dot first, rounded down and kept
    /// inside the image, and the cell from it, 12 by 20 dots to a cell, so
    /// the dot is always inside its cell, as the trusted core's reader
    /// requires (REQ-INT-127).
    fn page_point(&self, point: crate::geometry::Point) -> Option<Point> {
        let (_, rect) = self.wall.open()?;
        cell_in(rect, point)?;
        let x = (point.x - rect.x0) / rect.width() * f64::from(PAGE_WIDTH);
        let y = (point.y - rect.y0) / rect.height() * f64::from(PAGE_HEIGHT);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "within the page raster, as cell_in checked")]
        let (x, y) = (x as u32, y as u32);
        let x = x.min(u32::from(PAGE_WIDTH).saturating_sub(1));
        let y = y.min(u32::from(PAGE_HEIGHT).saturating_sub(1));
        let (column, row) = (x.checked_div(u32::from(CELL_WIDTH))?, y.checked_div(u32::from(CELL_HEIGHT))?);
        Some(Point { column, row, pixels: Some((x, y)), mods: self.typing.mods })
    }

    /// Whether the mouse is the channel's: keys go to it, the pointer is
    /// over the open page, and not with the browser's modifier, which
    /// always keeps a click the browser's (INPUT.md section 6.3,
    /// REQ-INT-115, REQ-INT-136).
    fn mouse_is_hosts(&self) -> bool {
        self.typing_target().is_some() && !self.typing.browser_modifier && self.pointer_on_page().is_some()
    }

    /// Whether a click on the open page is the host's: the mouse is, and
    /// the pointer is not on a link the page declares, which is the
    /// browser's on every page, so a plain click follows it and no press
    /// or release of any button on it reaches the channel (the owner's
    /// revision of 28 October 2026, decision 122: "links must be acted
    /// upon by the client browser"). A control is not a link.
    pub(in crate::browser) fn click_is_hosts(&self) -> bool {
        #[cfg(feature = "links")]
        if self.pointer_on_link() {
            return false;
        }
        self.mouse_is_hosts()
    }

    fn send(&mut self, target: PageKey, intent: Intent) {
        self.outputs.push(Output::Terminal { key: target, intent });
    }

    /// A key while keys go to a channel: `key` is its unshifted character
    /// or functional code, `mods` the protocol's bits, `text` what it
    /// typed and `base` the United States layout's character of the same
    /// physical key. Nothing is drawn for it: the channel draws what it
    /// makes of the key.
    pub fn terminal_key(&mut self, action: Action, key: KeyCode, mods: u8, text: Option<&str>, base: Option<char>) {
        let Some(target) = self.typing_target() else { return };
        let text = text.map(|text| cut(text, neoteletext_input::MAX_KEY_TEXT)).filter(|text| !text.is_empty()).map(String::from);
        self.send(target, Intent::Key { action, key, mods, text, base });
    }

    /// Text an input method committed, or a phone typed, while keys go to
    /// a channel: one intent, without control characters. Nothing is
    /// drawn for it: the channel draws what it makes of the text.
    pub fn terminal_text(&mut self, text: &str) {
        let Some(target) = self.typing_target() else { return };
        let printable: String = cut(text, MAX_PASTE).chars().filter(|&character| printable(character)).collect();
        if !printable.is_empty() {
            self.send(target, Intent::Text(printable));
        }
    }

    /// The browser's paste shortcut while keys go to a channel: the text as
    /// one PASTE (REQ-INT-136), data the channel never takes as keys.
    pub fn terminal_paste(&mut self, text: &str) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let text = cut(text, MAX_PASTE);
        if text.is_empty() {
            return false;
        }
        self.send(target, Intent::Paste(String::from(text)));
        true
    }

    /// The input method's preedit, `cursor` a code-point offset into it; an
    /// empty preedit ends composition.
    pub fn terminal_compose(&mut self, cursor: u32, text: &str) -> bool {
        let Some(target) = self.typing_target().filter(|_| self.composes_for_channel()) else { return false };
        let text = cut(text, MAX_COMPOSE);
        let length = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
        self.send(target, Intent::Compose { cursor: cursor.min(length), text: String::from(text) });
        true
    }

    /// A button other than the main one pressed over the open page:
    /// middle, right, back or forward, as MOUSE numbers them. It is the
    /// channel's while the mouse is (REQ-RCH-005), once the browser's own
    /// uses of the button, none of them over the page, have not taken it.
    /// Returns whether it went.
    pub fn other_button_pressed(&mut self, button: u8) -> bool {
        button != mouse_button::LEFT && self.terminal_button(MouseAction::Press, button)
    }

    /// A button other than the main one let go: released to the channel
    /// when it saw the press.
    pub fn other_button_released(&mut self, button: u8) -> bool {
        button != mouse_button::LEFT && self.terminal_button(MouseAction::Release, button)
    }

    /// A button pressed or released on the open page while the mouse is
    /// the channel's (REQ-INT-115, REQ-RCH-005).
    pub(in crate::browser) fn terminal_button(&mut self, action: MouseAction, button: u8) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let Some(at) = self.pointer_on_page() else { return false };
        if action == MouseAction::Press && !self.click_is_hosts() {
            return false;
        }
        let bit = 1_u8.checked_shl(u32::from(button)).filter(|_| button < BUTTONS).unwrap_or(0);
        match action {
            MouseAction::Press => self.typing.buttons |= bit,
            MouseAction::Release if self.typing.buttons & bit == 0 => return false,
            MouseAction::Release => self.typing.buttons &= !bit,
            MouseAction::Move | MouseAction::Gone => {}
        }
        // The channel now knows where the pointer is: leaving the page is
        // gone, even with no move since (REQ-RCH-034).
        self.typing.last_dot = Some(at);
        self.send(target, Intent::Mouse { action, button, at });
        true
    }

    /// The pointer moved: over the open page, each change of the dot is a
    /// move for the channel, buttons held or not (INPUT.md section 6.3,
    /// REQ-RCH-030), which the trusted core paces; off it, onto the border,
    /// the browser's controls or out of the window, gone (REQ-RCH-034).
    pub(in crate::browser) fn terminal_moved(&mut self) {
        let Some(target) = self.typing_target() else {
            // Keys stopped going: the trusted core sent gone itself.
            self.typing.last_dot = None;
            return;
        };
        match self.pointer_on_page() {
            Some(at) => {
                if self.typing.last_dot != Some(at) {
                    self.typing.last_dot = Some(at);
                    let buttons = self.typing.buttons;
                    self.send(target, Intent::Mouse { action: MouseAction::Move, button: buttons, at });
                }
            }
            None => {
                if self.typing.last_dot.take().is_some() {
                    // Gone releases what was held, without a click.
                    self.typing.buttons = 0;
                    self.send(target, Intent::Gone);
                }
            }
        }
    }

    /// A contact on a touch screen, at `x`, `y` in logical pixels, as the
    /// system reported it (REQ-RCH-041 to REQ-RCH-048): while keys go to
    /// the open page's channel, a contact that begins on the page is the
    /// channel's, and goes as an intent the trusted core numbers; one that
    /// begins on a link the page declares is the browser's, and follows the
    /// link if it lifts on it, the channel never told of it. A contact that
    /// leaves the page is cancelled for the channel. Returns whether the
    /// contact was taken; the browser has no touch gestures of its own yet.
    pub fn touch(&mut self, phase: TouchPhase, contact: u64, x: f64, y: f64) -> bool {
        let point = crate::geometry::Point::new(x, y);
        let Some(target) = self.typing_target() else { return false };
        if let Some(place) = self.typing.link_touches.iter().position(|(system, _)| *system == contact) {
            if phase == TouchPhase::Ended {
                let (_, link) = self.typing.link_touches.swap_remove(place);
                self.follow_touched_link(point, link);
            } else if phase == TouchPhase::Cancelled {
                self.typing.link_touches.swap_remove(place);
            }
            return true;
        }
        let at = self.page_point(point);
        let known = self.typing.touches.iter().position(|(system, _)| *system == contact);
        match (phase, known, at) {
            (TouchPhase::Began, None, Some(at)) => {
                #[cfg(feature = "links")]
                if let Some(link) = self.link_at(point) {
                    if self.typing.link_touches.len() < neoteletext_input::MAX_CONTACTS {
                        self.typing.link_touches.push((contact, link));
                    }
                    return true;
                }
                if self.typing.touches.len() >= neoteletext_input::MAX_CONTACTS {
                    return true;
                }
                self.typing.touches.push((contact, at));
                self.send(target, Intent::Touch { phase, contact, at });
                true
            }
            (TouchPhase::Moved, Some(place), Some(at)) => {
                if self.typing.touches.get(place).is_some_and(|(_, last)| *last != at) {
                    if let Some(entry) = self.typing.touches.get_mut(place) {
                        entry.1 = at;
                    }
                    self.send(target, Intent::Touch { phase, contact, at });
                }
                true
            }
            (TouchPhase::Ended | TouchPhase::Cancelled | TouchPhase::Moved, Some(place), at) => {
                let (_, last) = self.typing.touches.swap_remove(place);
                // Lifted on the page, a lift; lifted or moved off it, or
                // taken by the system, gone.
                let phase = if phase == TouchPhase::Ended && at.is_some() { TouchPhase::Ended } else { TouchPhase::Cancelled };
                self.send(target, Intent::Touch { phase, contact, at: at.unwrap_or(last) });
                true
            }
            _ => false,
        }
    }

    /// A contact that began on a link lifted at `point`: the link is
    /// followed when it lifted on the same link, as a click on it is
    /// (decision 122).
    #[cfg_attr(not(feature = "links"), expect(unused_variables, clippy::unused_self, reason = "without links no contact begins on a link"))]
    fn follow_touched_link(&mut self, point: crate::geometry::Point, link: (PageKey, usize)) {
        #[cfg(feature = "links")]
        if self.link_at(point) == Some(link)
            && let Some(click) = self.wall.cell_at(point)
        {
            let version = self.versions.get(&click.key).copied().unwrap_or_default();
            self.outputs.push(Output::Cell(crate::wall::CellClick { version, ..click }));
        }
    }

    /// The pointer entered or left the control whose first cell is
    /// `column`, `row` on the page `key` (INPUT.md section 5.4's HOVER);
    /// told only while keys go to that page. Only links know the controls.
    #[cfg(feature = "links")]
    pub(in crate::browser) fn terminal_hover(&mut self, key: PageKey, column: u8, row: u8, leave: bool) {
        if self.typing_target() == Some(key) {
            self.send(key, Intent::Hover { leave, column, row });
        }
    }

    /// The wheel, in lines, over the open page: the channel's while keys go
    /// to it (INPUT.md sections 6.3 and 9.1, decision 107);
    /// `None` when it is the browser's, to scroll: beside the page, or with
    /// the browser's modifier held.
    pub fn terminal_wheel(&mut self, across: f64, down: f64) -> Option<bool> {
        if self.typing.browser_modifier {
            return None;
        }
        let target = self.typing_target()?;
        let at = self.pointer_on_page()?;
        #[allow(clippy::cast_possible_truncation, reason = "a wheel's lines, far inside i32 once scaled")]
        let (dx, dy) = (libm::round(across * WHEEL_UNITS_PER_LINE) as i32, libm::round(down * WHEEL_UNITS_PER_LINE) as i32);
        if (dx, dy) == (0, 0) {
            return Some(false);
        }
        self.send(target, Intent::Wheel { dx, dy, at });
        Some(false)
    }

    /// While a cursor shows, the window draws again at its blink.
    pub(in crate::browser) fn terminal_frame_interval(&self) -> Option<u64> {
        self.typing_target()?;
        let typing = self.typing.typing?;
        typing.cursor.map(|_| BLINK_MILLISECONDS)
    }
}

#[cfg(test)]
mod tests;
