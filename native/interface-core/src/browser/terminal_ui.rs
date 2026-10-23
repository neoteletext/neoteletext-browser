//! Typing into a page (`neoteletext-common/INPUT.md` section 9): while
//! the page open is a page of a channel that takes input, the viewer's
//! keys are the channel's, as a terminal's are its host's, and the mouse
//! too when the channel or the page asks for it. The browser draws the
//! channel's cursor, the viewer's provisional echo, and in the strip who
//! is listening. The trusted core decides whether input goes and encodes
//! it; this only gives it intents (REQ-INT-132).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use neoteletext_input::cursor::{BLOCK, ECHO, MASKED};
use neoteletext_input::event::{ALT, Action, CONTROL, HYPER, KeyCode, META, MouseAction, Point, SUPER, printable};
use neoteletext_input::modes::{COMPOSE, MOUSE_ALL, MOUSE_BUTTONS, MOUSE_DRAG, MOUSE_MODES};
use neoteletext_input::{MAX_COMPOSE, MAX_PASTE};

use super::{Browser, Output};
use crate::canvas::{COLUMNS, Canvas, PAGE_HEIGHT, PAGE_WIDTH, PageKey, ROWS, Style};
use crate::channel::page_index;
use crate::geometry::Rect;
use crate::input::Intent;
use crate::theme::{FIELD_INK, INSECURE, ROW_STATUS, TYPING, TYPING_FIELD};
use crate::ui::left_centred;
use crate::wall::cell_in;

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
/// The strip's badge: its padding and its corner.
const BADGE_PADDING: f64 = 10.0;
const BADGE_RADIUS: f64 = 4.0;
const BADGE_INSET: f64 = 6.0;
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
        let state = &mut self.terminal;
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
        let typing = self.terminal.typing.filter(|typing| typing.on)?;
        let key = PageKey { channel: typing.channel, page: typing.page };
        // While a panel, popover or list of the strip is open, the keys are
        // the strip's (`history-and-bookmarks.md` section 2.4).
        (self.opened() == Some(key) && !self.editing && !self.strip_has_keys()).then_some(key)
    }

    /// Whether a panel, popover or list of the strip is open, which then
    /// takes the keys rather than the page's channel.
    fn strip_has_keys(&self) -> bool {
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
        self.typing_target().and(self.terminal.typing).map_or(0, |typing| typing.modes)
    }

    /// Whether the input method's preedit goes to the channel (INPUT.md
    /// section 9.1): while keys go and the channel asked for COMPOSE.
    #[must_use]
    pub fn composes_for_channel(&self) -> bool {
        self.typing_modes() & COMPOSE != 0
    }

    /// The browser's own modifier is held or released.
    pub fn set_browser_modifier(&mut self, held: bool) {
        self.terminal.browser_modifier = held;
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
        self.typing_modes() & MOUSE_MODES != 0 && !self.terminal.browser_modifier && self.pointer_on_page().is_some()
    }

    /// Whether a click on the open page is the host's.
    pub(super) fn click_is_hosts(&self) -> bool {
        self.mouse_is_hosts()
    }

    fn send(&mut self, target: PageKey, intent: Intent) -> u32 {
        self.terminal.local = self.terminal.local.wrapping_add(1);
        let local = self.terminal.local;
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
            return self.terminal.echoes.pop().is_some();
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
    pub(super) fn terminal_button(&mut self, action: MouseAction, button: u8) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let Some(at) = self.pointer_on_page() else { return false };
        if action == MouseAction::Press && !self.mouse_is_hosts() {
            return false;
        }
        let bit = 1_u8.checked_shl(u32::from(button)).filter(|_| button < BUTTONS).unwrap_or(0);
        match action {
            MouseAction::Press => self.terminal.buttons |= bit,
            MouseAction::Release if self.terminal.buttons & bit == 0 => return false,
            MouseAction::Release => self.terminal.buttons &= !bit,
            MouseAction::Move => {}
        }
        self.send(target, Intent::Mouse { action, button, at });
        true
    }

    /// The pointer moved over the open page: a move for the channel in
    /// `MOUSE_ALL`, or in `MOUSE_DRAG` while a button is held.
    pub(super) fn terminal_moved(&mut self) {
        let modes = self.typing_modes();
        let wanted = modes & MOUSE_ALL != 0 || modes & MOUSE_DRAG != 0 && self.terminal.buttons != 0;
        let (Some(target), Some(at)) = (self.typing_target(), self.pointer_on_page()) else { return };
        if wanted && !self.terminal.browser_modifier {
            let buttons = self.terminal.buttons;
            self.send(target, Intent::Mouse { action: MouseAction::Move, button: buttons, at });
        }
    }

    /// The wheel, in lines, over the open page: the channel's when it asked
    /// for `MOUSE_BUTTONS` (INPUT.md section 9.1); `None` when it is the
    /// browser's, to scroll.
    pub fn terminal_wheel(&mut self, across: f64, down: f64) -> Option<bool> {
        if self.typing_modes() & MOUSE_BUTTONS == 0 || self.terminal.browser_modifier {
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
        let Some(cursor) = self.terminal.typing.and_then(|typing| typing.cursor) else { return false };
        if cursor.flags & ECHO == 0 || !printable(character) {
            return false;
        }
        if self.terminal.echoes.len() >= usize::from(cursor.width) {
            return false;
        }
        let character = if cursor.flags & MASKED != 0 { MASK } else { character };
        self.terminal.echoes.push(Echo { local, character, at: now });
        true
    }

    /// Echoes not confirmed in time are given up.
    pub(super) fn settle_echoes(&mut self, now: u64) -> bool {
        let before = self.terminal.echoes.len();
        self.terminal.echoes.retain(|echo| now.saturating_sub(echo.at) < ECHO_LIFETIME_MILLISECONDS);
        before != self.terminal.echoes.len()
    }

    /// While a cursor shows, the window draws again at its blink; echoes
    /// need a frame when they lapse.
    pub(super) fn terminal_frame_interval(&self) -> Option<u64> {
        self.typing_target()?;
        let typing = self.terminal.typing?;
        typing.cursor.map(|_| BLINK_MILLISECONDS)
    }

    /// Draws the cursor and the echo over the open page, in a face and an
    /// amber no page can draw (REQ-INT-134).
    pub(super) fn draw_terminal<C: Canvas + ?Sized>(&self, canvas: &mut C, now: u64) {
        let Some(target) = self.typing_target() else { return };
        // Where the open page is drawn: zoomed, or on the wall.
        let shown = self.wall.shown().find(|(key, _)| *key == target).map(|(_, rect)| rect);
        let (Some(cursor), Some(page)) = (self.terminal.typing.and_then(|typing| typing.cursor), shown) else { return };
        let (cell_width, cell_height) = (page.width() / f64::from(COLUMNS), page.height() / f64::from(ROWS));
        let cell = |column: usize| {
            #[allow(clippy::cast_precision_loss, reason = "a column of a 40-column page")]
            let x = page.x0 + cell_width * column as f64;
            let y = page.y0 + cell_height * f64::from(cursor.row);
            Rect::new(x, y, x + cell_width, y + cell_height)
        };
        canvas.push_clip(self.wall.area());
        #[allow(clippy::cast_possible_truncation, reason = "a text size of a few dozen pixels")]
        let style = Style { size: (cell_height * ECHO_TEXT_SHARE) as f32, monospace: true, bold: false, caps: false };
        for (index, echo) in self.terminal.echoes.iter().enumerate() {
            let rect = cell(usize::from(cursor.column) + index);
            canvas.fill(rect, 0.0, TYPING_FIELD);
            let mut text = [0_u8; 4];
            left_centred(canvas, rect, 0.0, echo.character.encode_utf8(&mut text), style, TYPING);
        }
        let column = (usize::from(cursor.column) + self.terminal.echoes.len()).min(usize::from(COLUMNS) - 1);
        let visible = (now.saturating_sub(self.terminal.moved) / BLINK_MILLISECONDS).is_multiple_of(2);
        if visible {
            let rect = cell(column);
            if cursor.flags & BLOCK != 0 {
                let line = cell_width * BLOCK_LINE_SHARE;
                crate::ui::border(canvas, rect, line, TYPING);
            } else {
                canvas.fill(Rect::new(rect.x0, rect.y0, rect.x0 + cell_width * BAR_SHARE, rect.y1), 0.0, TYPING);
            }
        }
        canvas.pop_clip();
    }

    /// Draws "Keys go to …" at the right of the address field while keys
    /// go to a channel, with "Not secure" on a plain connection and the
    /// round trip (REQ-INT-131), or that the channel did not answer.
    /// Returns the field's text area left.
    pub(super) fn draw_typing_badge<C: Canvas + ?Sized>(&self, canvas: &mut C, area: Rect) -> Rect {
        let Some(typing) = self.terminal.typing else { return area };
        let refused = typing.refused && self.opened() == Some(PageKey { channel: typing.channel, page: typing.page });
        if self.typing_target().is_none() && !refused {
            return area;
        }
        let mut text = if refused {
            format!("\u{2328} {} did not answer: keys stay here", self.strip.identity)
        } else {
            format!("\u{2328} Keys go to {}", self.strip.identity)
        };
        if let Some(round_trip) = typing.round_trip_ms.filter(|_| !refused) {
            let _ = write!(text, " \u{00B7} {round_trip} ms");
        }
        let warning = (!typing.secure && !refused).then_some(" \u{00B7} Not secure");
        let width = canvas.text_width(&text, ROW_STATUS) + warning.map_or(0.0, |warning| canvas.text_width(warning, ROW_STATUS)) + 2.0 * BADGE_PADDING;
        let badge = Rect::new((area.x1 - width).max(area.x0), area.y0 + BADGE_INSET, area.x1, area.y1 - BADGE_INSET);
        canvas.fill(badge, BADGE_RADIUS, if refused { INSECURE } else { TYPING });
        let inner = Rect::new(badge.x0 + BADGE_PADDING, badge.y0, badge.x1, badge.y1);
        left_centred(canvas, inner, 0.0, &text, ROW_STATUS, FIELD_INK);
        if let Some(warning) = warning {
            let after = inner.x0 + canvas.text_width(&text, ROW_STATUS);
            left_centred(canvas, Rect::new(after, inner.y0, inner.x1, inner.y1), 0.0, warning, ROW_STATUS, INSECURE);
        }
        Rect::new(area.x0, area.y0, (badge.x0 - BADGE_PADDING).max(area.x0), area.y1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Frame;
    use crate::channel::{Channel, PageInfo, Status};
    use crate::test_canvas::Recorder;
    use neoteletext_input::modes::{COMPOSE, MOUSE_PIXELS};

    const KEY: PageKey = PageKey { channel: 4, page: 0x100 };

    fn browser() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        let mut channel = Channel::empty();
        channel.channel = KEY.channel;
        channel.pages[0] = Some(PageInfo { title: "Terminal".into(), status: Status::Loaded });
        browser.set_channel(channel);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.wall.zoom(KEY, 0);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 10_000);
        browser.take_outputs();
        browser
    }

    fn typing(cursor: Option<CursorView>, acked: u32) -> TypingView {
        TypingView { channel: KEY.channel, page: KEY.page, on: true, secure: false, refused: false, modes: 0, cursor, acked, round_trip_ms: None, show: None }
    }

    fn intents(outputs: &[Output]) -> Vec<Intent> {
        outputs.iter().filter_map(|output| if let Output::Terminal { intent, .. } = output { Some(intent.clone()) } else { None }).collect()
    }

    fn press(browser: &mut Browser, character: char, mods: u8) -> bool {
        let mut text = [0_u8; 4];
        let text = character.encode_utf8(&mut text);
        browser.terminal_key(Action::Press, KeyCode::Character(character), mods, Some(text), None, 0)
    }

    fn pointer_at(browser: &mut Browser, column: f64, row: f64) {
        let (_, page) = browser.wall.open().unwrap();
        let x = page.x0 + page.width() * (column / 40.0);
        let y = page.y0 + page.height() * (row / 25.0);
        browser.pointer_moved(x, y, 10_000);
    }

    /// REQ-INT-130, REQ-INT-132: intents only while the core says so.
    #[test]
    fn keys_go_only_while_the_core_says_so() {
        let mut browser = browser();
        assert!(!press(&mut browser, 'a', 0));
        assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "the core has not said");
        browser.set_typing(typing(None, 0), 0);
        assert_eq!(browser.typing_target(), Some(KEY));
        press(&mut browser, 'a', 0);
        browser.terminal_key(Action::Release, KeyCode::Character('a'), 0, None, None, 0);
        let sent = intents(&browser.take_outputs());
        assert_eq!(sent.len(), 2);
        assert!(matches!(&sent[0], Intent::Key { action: Action::Press, key: KeyCode::Character('a'), text: Some(text), .. } if text == "a"));
        assert!(matches!(&sent[1], Intent::Key { action: Action::Release, .. }));
        browser.set_typing(TypingView { on: false, ..typing(None, 0) }, 0);
        press(&mut browser, 'b', 0);
        assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "typing is off");
    }

    /// A panel, popover or list of the strip, while open, takes the keys:
    /// none go to the page's channel. Before, Return in the star's popover
    /// went to a channel that takes keys, and the popover never closed.
    #[test]
    fn the_strips_panels_take_the_keys_from_the_channel() {
        let mut browser = browser();
        browser.set_typing(typing(None, 0), 0);
        assert_eq!(browser.typing_target(), Some(KEY));
        browser.set_star(crate::library::StarView { open: true, ..crate::library::StarView::default() });
        assert_eq!(browser.typing_target(), None, "the star's popover");
        browser.set_star(crate::library::StarView::default());
        browser.toggle_history();
        assert_eq!(browser.typing_target(), None, "the history panel");
        browser.toggle_history();
        browser.toggle_menu();
        assert_eq!(browser.typing_target(), None, "the menu");
        browser.toggle_menu();
        assert_eq!(browser.typing_target(), Some(KEY));
    }

    /// REQ-INT-134: echo of the viewer's own printable keys, within the
    /// width, masked when asked, gone on acknowledgement or after 2 s.
    #[test]
    fn echo_stays_in_its_width_is_masked_and_clears_on_acknowledgement_or_time() {
        let mut browser = browser();
        let cursor = CursorView { row: 5, column: 10, width: 3, flags: ECHO };
        browser.set_typing(typing(Some(cursor), 0), 0);
        for character in "abcd".chars() {
            press(&mut browser, character, 0);
        }
        let echoed: String = browser.terminal.echoes.iter().map(|echo| echo.character).collect();
        assert_eq!(echoed, "abc", "never past the width");
        browser.terminal_key(Action::Press, KeyCode::Functional(BACKSPACE), 0, None, None, 0);
        assert_eq!(browser.terminal.echoes.len(), 2);
        press(&mut browser, 'x', CONTROL);
        assert_eq!(browser.terminal.echoes.len(), 2, "Control-x is a command, not text");
        // The first key's acknowledgement removes its echo.
        browser.set_typing(typing(Some(cursor), 1), 10);
        assert_eq!(browser.terminal.echoes.len(), 1);
        assert!(browser.settle_echoes(ECHO_LIFETIME_MILLISECONDS));
        assert!(browser.terminal.echoes.is_empty());
        browser.set_typing(typing(Some(CursorView { flags: ECHO | MASKED, ..cursor }), 99), 0);
        browser.terminal_text("pw", 0);
        assert!(browser.terminal.echoes.iter().all(|echo| echo.character == MASK));
        // Without echo, nothing shows.
        browser.set_typing(typing(Some(CursorView { flags: 0, ..cursor }), 99), 0);
        press(&mut browser, 'q', 0);
        assert!(browser.terminal.echoes.is_empty());
    }

    /// REQ-INT-115, REQ-INT-136: a plain click is the host's only in a
    /// mouse mode; otherwise, and with the browser's modifier always, it
    /// is the browser's, to follow a link.
    #[test]
    fn a_click_is_the_hosts_only_in_a_mouse_mode() {
        let mut browser = browser();
        let mut canvas = Recorder::new();
        browser.set_typing(typing(None, 0), 0);
        pointer_at(&mut browser, 10.5, 5.5);
        browser.pointer_pressed(&mut canvas, false, 10_000);
        let outputs = browser.take_outputs();
        assert_eq!(intents(&outputs), [] as [Intent; 0], "no mouse mode: the browser's click");
        assert!(outputs.iter().any(|output| matches!(output, Output::Cell(_))), "a link's click");
        browser.set_typing(TypingView { modes: MOUSE_BUTTONS | MOUSE_PIXELS, ..typing(None, 0) }, 0);
        browser.pointer_pressed(&mut canvas, false, 10_000);
        browser.pointer_released();
        let outputs = browser.take_outputs();
        let sent = intents(&outputs);
        assert!(
            matches!(
                sent.as_slice(),
                [
                    Intent::Mouse { action: MouseAction::Press, button: 0, at: Point { column: 10, row: 5, pixels: Some(_), .. } },
                    Intent::Mouse { action: MouseAction::Release, button: 0, .. },
                ]
            ),
            "{sent:?}"
        );
        assert!(!outputs.iter().any(|output| matches!(output, Output::Cell(_))));
        browser.set_browser_modifier(true);
        browser.pointer_pressed(&mut canvas, false, 10_000);
        let outputs = browser.take_outputs();
        assert!(outputs.iter().any(|output| matches!(output, Output::Cell(_))), "the modifier keeps a link's click");
        assert_eq!(intents(&outputs), [] as [Intent; 0]);
    }

    #[test]
    fn the_wheel_and_moves_go_only_when_asked_for() {
        let mut browser = browser();
        browser.set_typing(typing(None, 0), 0);
        pointer_at(&mut browser, 3.0, 3.0);
        assert_eq!(browser.terminal_wheel(0.0, 1.0), None, "the browser scrolls");
        browser.set_typing(TypingView { modes: MOUSE_BUTTONS, ..typing(None, 0) }, 0);
        assert_eq!(browser.terminal_wheel(0.0, 1.0), Some(false));
        assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Wheel { dx: 0, dy: 120, .. }]));
        pointer_at(&mut browser, 4.0, 3.0);
        assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "no moves in MOUSE_BUTTONS");
        browser.set_typing(TypingView { modes: MOUSE_ALL, ..typing(None, 0) }, 0);
        pointer_at(&mut browser, 5.0, 3.0);
        assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Mouse { action: MouseAction::Move, button: 0, .. }]));
    }

    #[test]
    fn paste_is_one_intent_and_composition_only_in_its_mode() {
        let mut browser = browser();
        browser.set_typing(typing(None, 0), 0);
        assert!(browser.terminal_paste("line\nline"));
        assert!(!browser.terminal_compose(1, "に"), "not in COMPOSE");
        browser.set_typing(TypingView { modes: COMPOSE, ..typing(None, 0) }, 0);
        assert!(browser.terminal_compose(9, "に"));
        let sent = intents(&browser.take_outputs());
        assert_eq!(sent, [Intent::Paste("line\nline".into()), Intent::Compose { cursor: 1, text: "に".into() }], "the offset kept within the text");
    }

    #[test]
    fn long_text_goes_as_one_intent_within_the_paste_bound() {
        let mut browser = browser();
        browser.set_typing(typing(None, 0), 0);
        browser.terminal_text(&"é".repeat(2_100), 0);
        let sent = intents(&browser.take_outputs());
        assert!(matches!(sent.as_slice(), [Intent::Text(text)] if text.len() == 4_000), "cut at a character within 4 000 bytes");
    }

    /// REQ-INT-131: the strip says where keys go, "Not secure" on plain
    /// `ws://`, and the round trip; or that the channel did not answer.
    #[test]
    fn the_strip_says_where_keys_go() {
        let area = Rect::new(0.0, 0.0, 900.0, 40.0);
        let badge = |browser: &Browser| {
            let mut canvas = Recorder::new();
            browser.draw_typing_badge(&mut canvas, area);
            canvas.texts.iter().map(|(_, text)| text.as_str()).collect::<String>()
        };
        let mut browser = browser();
        assert_eq!(badge(&browser), "", "nothing while keys do not go");
        browser.set_typing(TypingView { round_trip_ms: Some(3), ..typing(None, 0) }, 0);
        let said = badge(&browser);
        assert!(said.contains("Keys go to") && said.contains("3 ms") && said.contains("Not secure"), "{said}");
        browser.set_typing(TypingView { secure: true, ..typing(None, 0) }, 0);
        assert!(!badge(&browser).contains("Not secure"));
        browser.set_typing(TypingView { on: false, refused: true, ..typing(None, 0) }, 0);
        assert!(badge(&browser).contains("did not answer"), "{}", badge(&browser));
    }

    #[test]
    fn the_cursor_is_drawn_only_while_typing() {
        let mut browser = browser();
        let mut canvas = Recorder::new();
        browser.draw_terminal(&mut canvas, 0);
        let before = canvas.fills.len();
        browser.set_typing(typing(Some(CursorView { row: 3, column: 0, width: 0, flags: 0 }), 0), 0);
        browser.draw_terminal(&mut canvas, 0);
        assert!(canvas.fills.len() > before, "a bar is drawn");
    }
}
