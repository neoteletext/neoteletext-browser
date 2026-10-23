//! Typing into a page (`neoteletext-common/INPUT.md` section 9): while
//! the page open is a page of a channel that takes input, the viewer's
//! keys are the channel's, as a terminal's are its host's, and the mouse
//! too when the channel or the page asks for it. The browser draws the
//! channel's cursor, the viewer's provisional echo, and in the strip who
//! is listening. The trusted core decides whether input goes and encodes
//! it; this only gives it intents (REQ-INT-132).

use alloc::string::String;
use alloc::vec::Vec;

use neoteletext_input::cursor::{ECHO, MASKED};
use neoteletext_input::event::{ALT, Action, CONTROL, HYPER, KeyCode, META, MouseAction, Point, SUPER, printable};
use neoteletext_input::modes::{COMPOSE, MOUSE_ALL, MOUSE_BUTTONS, MOUSE_DRAG, MOUSE_MODES};
use neoteletext_input::{MAX_COMPOSE, MAX_PASTE};

use crate::browser::{Browser, Output};
use crate::canvas::{PAGE_HEIGHT, PAGE_WIDTH, PageKey};
use crate::channel::page_index;
use crate::input::Intent;
use crate::wall::cell_in;

mod draw;

/// An echo is given up after this long with no acknowledgement
/// (INPUT.md section 9.3).
pub const ECHO_LIFETIME_MILLISECONDS: u64 = 2_000;
/// The cursor blinks at this half period, as common terminals do.
pub const BLINK_MILLISECONDS: u64 = 530;
/// A bar cursor's width as a share of a cell.
const BAR_SHARE: f64 = 0.15;
/// A block cursor's outline as a share of a cell's width.
const BLOCK_LINE_SHARE: f64 = 0.12;
/// The echo's text as a share of a cell's height.
const ECHO_TEXT_SHARE: f64 = 0.8;
/// What masked echo shows.
const MASK: char = '*';
/// The modifiers that make a key a command, not text (INPUT.md section
/// 5.3): its echo would be wrong.
const COMMANDS: u8 = CONTROL | ALT | SUPER | HYPER | META;
/// The functional code of Backspace (INPUT.md section 5.6).
const BACKSPACE: u32 = 4;
/// The wheel's unit: 1/120th of a line (INPUT.md section 5.4).
const WHEEL_UNITS_PER_LINE: f64 = 120.0;
/// Mouse buttons tracked as held, as section 5.4's `button` numbers them.
const BUTTONS: u8 = 8;

/// The cursor a channel placed, as the trusted core checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorView {
    /// 1 to 23.
    pub row: u8,
    /// 0 to 39.
    pub column: u8,
    /// How many cells echo may run from the column.
    pub width: u8,
    /// Echo, masked and block, as the engine's cursor record says.
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
    /// The input modes in force on the page: whether the mouse, the wheel
    /// and composition are the channel's.
    pub modes: u32,
    /// The cursor, if the channel placed one on this page.
    pub cursor: Option<CursorView>,
    /// The number of the last intent the channel acted on.
    pub acked: u32,
    /// The last round trip, in milliseconds.
    pub round_trip_ms: Option<u16>,
    /// Another page of the channel its host went to (REQ-INT-113): the
    /// browser shows it.
    pub show: Option<u16>,
}

/// A character shown before the channel confirmed it.
#[derive(Clone, Copy, Debug)]
struct Echo {
    local: u32,
    character: char,
    at: u64,
}

/// The browser's typing state.
#[derive(Debug, Default)]
pub(super) struct TerminalUi {
    typing: Option<TypingView>,
    /// Numbers the viewer's intents, counting up.
    local: u32,
    echoes: Vec<Echo>,
    /// The browser's own modifier is held: Command on macOS, Control and
    /// Shift elsewhere. A click is then the browser's, not the host's.
    browser_modifier: bool,
    /// When the cursor last moved, for its blink.
    moved: u64,
    /// The mouse buttons held on the page, bit `n` for button `n`.
    buttons: u8,
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
        state.echoes.retain(|echo| echo.local > typing.acked);
        if !typing.on || typing.cursor.is_none_or(|cursor| cursor.flags & ECHO == 0) {
            state.echoes.clear();
        }
        if typing.modes & MOUSE_MODES == 0 {
            state.buttons = 0;
        }
        if moved {
            state.moved = now;
        }
        state.typing = Some(typing);
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
        (self.opened() == Some(key) && !self.editing && !self.strip_has_keys()).then_some(key)
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

    /// The modes in force on the page keys go to, 0 when none.
    fn typing_modes(&self) -> u32 {
        self.typing_target().and(self.typing.typing).map_or(0, |typing| typing.modes)
    }

    /// Whether the input method's preedit goes to the channel (INPUT.md
    /// section 9.1): while keys go and the channel asked for COMPOSE.
    #[must_use]
    pub fn composes_for_channel(&self) -> bool {
        self.typing_modes() & COMPOSE != 0
    }

    /// The browser's own modifier is held or released.
    pub fn set_browser_modifier(&mut self, held: bool) {
        self.typing.browser_modifier = held;
    }

    /// Where the pointer is on the open page, while it is over it: the cell
    /// and the page image's pixel.
    fn pointer_on_page(&self) -> Option<Point> {
        let (_, rect) = self.wall.open()?;
        let (column, row) = cell_in(rect, self.pointer)?;
        let x = (self.pointer.x - rect.x0) / rect.width() * f64::from(PAGE_WIDTH);
        let y = (self.pointer.y - rect.y0) / rect.height() * f64::from(PAGE_HEIGHT);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "within the page raster, as cell_in checked")]
        let pixels = Some((x as u32, y as u32));
        Some(Point { column: u32::from(column), row: u32::from(row), pixels, mods: 0 })
    }

    /// Whether the mouse is the channel's: a mouse mode in force, the
    /// pointer over the open page, and not the browser's modifier, which
    /// always keeps a click the browser's (REQ-INT-115, REQ-INT-136).
    fn mouse_is_hosts(&self) -> bool {
        self.typing_modes() & MOUSE_MODES != 0 && !self.typing.browser_modifier && self.pointer_on_page().is_some()
    }

    /// Whether a click on the open page is the host's.
    pub(in crate::browser) fn click_is_hosts(&self) -> bool {
        self.mouse_is_hosts()
    }

    fn send(&mut self, target: PageKey, intent: Intent) -> u32 {
        self.typing.local = self.typing.local.wrapping_add(1);
        let local = self.typing.local;
        self.outputs.push(Output::Terminal { key: target, local, intent });
        local
    }

    /// A key while keys go to a channel: `key` is its unshifted character
    /// or functional code, `mods` the protocol's bits, `text` what it
    /// typed and `base` the United States layout's character of the same
    /// physical key. Returns whether the window needs drawing.
    pub fn terminal_key(&mut self, action: Action, key: KeyCode, mods: u8, text: Option<&str>, base: Option<char>, now: u64) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let text = text.map(|text| cut(text, neoteletext_input::MAX_KEY_TEXT)).filter(|text| !text.is_empty()).map(String::from);
        let echo = text.clone().filter(|text| action != Action::Release && mods & COMMANDS == 0 && text.chars().all(printable));
        let local = self.send(target, Intent::Key { action, key, mods, text, base });
        if action == Action::Press && key == KeyCode::Functional(BACKSPACE) {
            return self.typing.echoes.pop().is_some();
        }
        let mut changed = false;
        for character in echo.iter().flat_map(|text| text.chars()) {
            changed |= self.echo(character, local, now);
        }
        changed
    }

    /// Text an input method committed, or a phone typed, while keys go to
    /// a channel: one intent, without control characters, echoed.
    pub fn terminal_text(&mut self, text: &str, now: u64) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let printable: String = cut(text, MAX_PASTE).chars().filter(|&character| printable(character)).collect();
        if printable.is_empty() {
            return false;
        }
        let echoed: Vec<char> = printable.chars().collect();
        let local = self.send(target, Intent::Text(printable));
        let mut changed = false;
        for character in echoed {
            changed |= self.echo(character, local, now);
        }
        changed
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

    /// The input method's preedit, `cursor` a code-point offset into it, in
    /// the COMPOSE mode; an empty preedit ends composition.
    pub fn terminal_compose(&mut self, cursor: u32, text: &str) -> bool {
        let Some(target) = self.typing_target().filter(|_| self.composes_for_channel()) else { return false };
        let text = cut(text, MAX_COMPOSE);
        let length = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
        self.send(target, Intent::Compose { cursor: cursor.min(length), text: String::from(text) });
        true
    }

    /// The main button pressed or released on the open page while the
    /// mouse is the channel's (REQ-INT-115).
    pub(in crate::browser) fn terminal_button(&mut self, action: MouseAction, button: u8) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let Some(at) = self.pointer_on_page() else { return false };
        if action == MouseAction::Press && !self.mouse_is_hosts() {
            return false;
        }
        let bit = 1_u8.checked_shl(u32::from(button)).filter(|_| button < BUTTONS).unwrap_or(0);
        match action {
            MouseAction::Press => self.typing.buttons |= bit,
            MouseAction::Release if self.typing.buttons & bit == 0 => return false,
            MouseAction::Release => self.typing.buttons &= !bit,
            MouseAction::Move => {}
        }
        self.send(target, Intent::Mouse { action, button, at });
        true
    }

    /// The pointer moved over the open page: a move for the channel in
    /// `MOUSE_ALL`, or in `MOUSE_DRAG` while a button is held.
    pub(in crate::browser) fn terminal_moved(&mut self) {
        let modes = self.typing_modes();
        let wanted = modes & MOUSE_ALL != 0 || modes & MOUSE_DRAG != 0 && self.typing.buttons != 0;
        let (Some(target), Some(at)) = (self.typing_target(), self.pointer_on_page()) else { return };
        if wanted && !self.typing.browser_modifier {
            let buttons = self.typing.buttons;
            self.send(target, Intent::Mouse { action: MouseAction::Move, button: buttons, at });
        }
    }

    /// The wheel, in lines, over the open page: the channel's when it asked
    /// for `MOUSE_BUTTONS` (INPUT.md section 9.1); `None` when it is the
    /// browser's, to scroll.
    pub fn terminal_wheel(&mut self, across: f64, down: f64) -> Option<bool> {
        if self.typing_modes() & MOUSE_BUTTONS == 0 || self.typing.browser_modifier {
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

    fn echo(&mut self, character: char, local: u32, now: u64) -> bool {
        let Some(cursor) = self.typing.typing.and_then(|typing| typing.cursor) else { return false };
        if cursor.flags & ECHO == 0 || !printable(character) {
            return false;
        }
        if self.typing.echoes.len() >= usize::from(cursor.width) {
            return false;
        }
        let character = if cursor.flags & MASKED != 0 { MASK } else { character };
        self.typing.echoes.push(Echo { local, character, at: now });
        true
    }

    /// Echoes not confirmed in time are given up.
    pub(in crate::browser) fn settle_echoes(&mut self, now: u64) -> bool {
        let before = self.typing.echoes.len();
        self.typing.echoes.retain(|echo| now.saturating_sub(echo.at) < ECHO_LIFETIME_MILLISECONDS);
        before != self.typing.echoes.len()
    }

    /// While a cursor shows, the window draws again at its blink; echoes
    /// need a frame when they lapse.
    pub(in crate::browser) fn terminal_frame_interval(&self) -> Option<u64> {
        self.typing_target()?;
        let typing = self.typing.typing?;
        typing.cursor.map(|_| BLINK_MILLISECONDS)
    }
}

#[cfg(test)]
mod tests;
