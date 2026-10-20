//! Typing into a page (`interaction.md` sections 5 to 8): while the page
//! open is a page of a channel that takes input, the viewer's keys are the
//! channel's, as a terminal's are its host's. The browser draws the
//! channel's cursor, the viewer's provisional echo, and in the strip who
//! is listening. The trusted core decides whether keys go; this only asks.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use neoteletext_input::cursor::{BLOCK, ECHO, MASKED};
use neoteletext_input::key::{self, Event, MAX_RECORD, MAX_TEXT, Press, Record};
use neoteletext_input::names::{is_named, named_key};

use super::{Browser, Output};
use crate::canvas::{COLUMNS, Canvas, PageKey, ROWS, Style};
use crate::channel::page_index;
use crate::geometry::Rect;
use crate::theme::{FIELD_INK, INSECURE, ROW_STATUS, TYPING, TYPING_FIELD};
use crate::ui::left_centred;
use crate::wall::cell_in;

/// An echo is given up after this long with no acknowledgement
/// (`interaction.md` section 8).
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
    /// The cursor, if the channel placed one on this page.
    pub cursor: Option<CursorView>,
    /// The number of the last key the channel acted on.
    pub acked: u32,
    /// The last round trip, in milliseconds.
    pub round_trip_ms: Option<u16>,
    /// Another page of the channel its host went to (`interaction.md`
    /// section 5.1): the browser shows it.
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
    /// Numbers the viewer's keys, counting up.
    local: u32,
    echoes: Vec<Echo>,
    /// The browser's own modifier is held: Command on macOS, Control and
    /// Shift elsewhere. A click is then the browser's, not the host's.
    browser_modifier: bool,
    /// When the cursor last moved, for its blink.
    moved: u64,
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
        (self.opened() == Some(key) && !self.editing).then_some(key)
    }

    /// The browser's own modifier is held or released.
    pub fn set_browser_modifier(&mut self, held: bool) {
        self.terminal.browser_modifier = held;
    }

    /// Whether a click on the open page is the host's.
    pub(super) fn click_is_hosts(&self) -> bool {
        self.typing_target().is_some() && !self.terminal.browser_modifier
    }

    fn send(&mut self, target: PageKey, event: Event<'_>, clock_ms: u32) -> Option<u32> {
        let mut out = [0_u8; MAX_RECORD];
        let length = Record { sequence: 0, clock_ms, event }.write(&mut out)?;
        self.terminal.local = self.terminal.local.wrapping_add(1);
        let local = self.terminal.local;
        self.outputs.push(Output::Terminal { key: target, local, record: out.get(..length).unwrap_or(&[]).to_vec() });
        Some(local)
    }

    /// A key went down or up while keys go to a channel. `key` is the
    /// engine's key code (`neoteletext_input::names`), `physical` its
    /// position, `modifiers` the engine's bits. Returns whether the window
    /// needs drawing.
    pub fn terminal_key(&mut self, down: bool, key: u16, modifiers: u16, physical: u16, clock_ms: u32, now: u64) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let press = Press { key, modifiers, physical };
        let Some(local) = self.send(target, if down { Event::Down(press) } else { Event::Up(press) }, clock_ms) else { return false };
        if !down || modifiers & key::REPEAT != 0 && is_named(key) {
            return false;
        }
        // Echo only plain typing: Control or Alt make a key a command.
        let plain = modifiers & (key::CONTROL | key::ALT) == 0;
        if key == named_key("Backspace").unwrap_or(0) {
            return self.terminal.echoes.pop().is_some();
        }
        match char::from_u32(u32::from(key)).filter(|_| plain && !is_named(key)) {
            Some(character) => self.echo(character, local, now),
            None => false,
        }
    }

    /// Text an input method committed, or a phone typed, while keys go to
    /// a channel: sent in records of at most 32 bytes, each echoed.
    pub fn terminal_text(&mut self, text: &str, clock_ms: u32, now: u64) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let mut changed = false;
        let mut start = 0;
        while start < text.len() {
            let mut end = (start + MAX_TEXT).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let piece = &text[start..end];
            start = end;
            let printable: String = piece.chars().filter(|character| !character.is_control()).collect();
            if printable.is_empty() {
                continue;
            }
            let Some(local) = self.send(target, Event::Text(&printable), clock_ms) else { continue };
            for character in printable.chars() {
                changed |= self.echo(character, local, now);
            }
        }
        changed
    }

    /// A click on the open page while keys go to its channel: the host's,
    /// as a terminal with mouse reporting gives it.
    pub(super) fn terminal_click(&mut self, clock_ms: u32) -> bool {
        let Some(target) = self.typing_target() else { return false };
        let Some((_, rect)) = self.wall.open() else { return false };
        let Some((column, row)) = cell_in(rect, self.pointer) else { return false };
        self.send(target, Event::Click { column, row }, clock_ms).is_some()
    }

    fn echo(&mut self, character: char, local: u32, now: u64) -> bool {
        let Some(cursor) = self.terminal.typing.and_then(|typing| typing.cursor) else { return false };
        if cursor.flags & ECHO == 0 || character.is_control() {
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
    /// amber no page can draw (requirement 18.7).
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
    /// round trip (requirement 18.3). Returns the field's text area left.
    pub(super) fn draw_typing_badge<C: Canvas + ?Sized>(&self, canvas: &mut C, area: Rect) -> Rect {
        if self.typing_target().is_none() {
            return area;
        }
        let Some(typing) = self.terminal.typing else { return area };
        let mut text = format!("\u{2328} Keys go to {}", self.strip.identity);
        if let Some(round_trip) = typing.round_trip_ms {
            let _ = write!(text, " \u{00B7} {round_trip} ms");
        }
        let warning = (!typing.secure).then_some(" \u{00B7} Not secure");
        let width = canvas.text_width(&text, ROW_STATUS) + warning.map_or(0.0, |warning| canvas.text_width(warning, ROW_STATUS)) + 2.0 * BADGE_PADDING;
        let badge = Rect::new((area.x1 - width).max(area.x0), area.y0 + BADGE_INSET, area.x1, area.y1 - BADGE_INSET);
        canvas.fill(badge, BADGE_RADIUS, TYPING);
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
        TypingView { channel: KEY.channel, page: KEY.page, on: true, secure: false, cursor, acked, round_trip_ms: None, show: None }
    }

    fn terminal(outputs: &[Output]) -> Vec<Record<'_>> {
        outputs.iter().filter_map(|output| if let Output::Terminal { record, .. } = output { key::read(record).ok() } else { None }).collect()
    }

    #[test]
    fn keys_go_only_while_the_core_says_so() {
        let mut browser = browser();
        assert!(!browser.terminal_key(true, u16::from(b'a'), 0, 0, 1, 0));
        assert_eq!(terminal(&browser.take_outputs()).len(), 0, "the core has not said");
        browser.set_typing(typing(None, 0), 0);
        assert_eq!(browser.typing_target(), Some(KEY));
        browser.terminal_key(true, u16::from(b'a'), 0, 0, 1, 0);
        browser.terminal_key(false, u16::from(b'a'), 0, 0, 2, 0);
        let outputs = browser.take_outputs();
        let records = terminal(&outputs);
        assert_eq!(records.len(), 2);
        assert!(matches!(records[0].event, Event::Down(press) if press.key == u16::from(b'a')));
        assert!(matches!(records[1].event, Event::Up(_)));
        browser.set_typing(TypingView { on: false, ..typing(None, 0) }, 0);
        browser.terminal_key(true, u16::from(b'b'), 0, 0, 3, 0);
        assert_eq!(terminal(&browser.take_outputs()).len(), 0, "typing is off");
    }

    #[test]
    fn echo_stays_in_its_width_is_masked_and_clears_on_acknowledgement_or_time() {
        let mut browser = browser();
        let cursor = CursorView { row: 5, column: 10, width: 3, flags: ECHO };
        browser.set_typing(typing(Some(cursor), 0), 0);
        for character in b"abcd" {
            browser.terminal_key(true, u16::from(*character), 0, 0, 0, 0);
        }
        let echoed: String = browser.terminal.echoes.iter().map(|echo| echo.character).collect();
        assert_eq!(echoed, "abc", "never past the width");
        browser.terminal_key(true, named_key("Backspace").unwrap(), 0, 0, 0, 0);
        assert_eq!(browser.terminal.echoes.len(), 2);
        browser.terminal_key(true, u16::from(b'x'), key::CONTROL, 0, 0, 0);
        assert_eq!(browser.terminal.echoes.len(), 2, "Control-x is a command, not text");
        // The first key's acknowledgement removes its echo.
        browser.set_typing(typing(Some(cursor), 1), 10);
        assert_eq!(browser.terminal.echoes.len(), 1);
        assert!(browser.settle_echoes(ECHO_LIFETIME_MILLISECONDS));
        assert!(browser.terminal.echoes.is_empty());
        browser.set_typing(typing(Some(CursorView { flags: ECHO | MASKED, ..cursor }), 99), 0);
        browser.terminal_text("pw", 0, 0);
        assert!(browser.terminal.echoes.iter().all(|echo| echo.character == MASK));
        // Without echo, nothing shows.
        browser.set_typing(typing(Some(CursorView { flags: 0, ..cursor }), 99), 0);
        browser.terminal_key(true, u16::from(b'q'), 0, 0, 0, 0);
        assert!(browser.terminal.echoes.is_empty());
    }

    #[test]
    fn long_text_goes_in_pieces_of_32_bytes() {
        let mut browser = browser();
        browser.set_typing(typing(None, 0), 0);
        browser.terminal_text(&"é".repeat(40), 0, 0);
        let outputs = browser.take_outputs();
        let pieces: Vec<usize> = terminal(&outputs).iter().map(|record| if let Event::Text(text) = record.event { text.len() } else { 0 }).collect();
        assert_eq!(pieces, [32, 32, 16]);
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
