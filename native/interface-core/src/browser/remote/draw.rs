//! The remote control drawn: its handle, the panel docked or floating,
//! and every row of keys, the title row, the readout, the number pad, the
//! page keys, Fastext and Index, the sub-pages and the toggles; and its
//! tips.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    Button, GAP, HANDLE_HEIGHT, HANDLE_WIDTH, KEY_HEIGHT, PAD, PAD_COLUMNS, PADDING, RADIUS, READOUT_HEIGHT, REMOTE_WIDTH, SMALL_KEY_HEIGHT, TITLE_BUTTON,
    TITLE_HEIGHT, panel_height,
};
use crate::browser::Browser;
use crate::canvas::Canvas;
use crate::controls::{FastextKey, PageControl, PageControls};
use crate::geometry::Rect;
use crate::settings::RemoteMode;
use crate::text::{border, centred, left_centred, outline};
use crate::theme::{BUTTON, CONTROL, CYAN, DIM, GREEN, HEADING, PANEL, READOUT, RED, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE, YELLOW};

impl Browser {
    /// The remote, docked at the right of the body or floating over it,
    /// and its handle; `top` is the body's top, below the strip and the
    /// bookmarks bar, which it never covers (requirement 11.4).
    pub(in crate::browser) fn draw_remote<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, height: f64, top: f64, now: u64) {
        self.remote.drawn_at = now;
        if !self.remote_sliding(now) {
            self.remote.slide = None;
        }
        self.remote.buttons.clear();
        self.remote.panel = Rect::ZERO;
        self.remote.title = Rect::ZERO;
        self.remote.handle = Rect::ZERO;
        let bottom = height - crate::browser::window::MARGIN;
        let out = self.remote_docked_width(now);
        let panel_height = panel_height().min(bottom - top);
        let panel = match self.remote.place.mode {
            RemoteMode::Floating => {
                let (x, y) = self.remote.dragged.unwrap_or((f64::from(self.remote.place.x), f64::from(self.remote.place.y)));
                // Kept inside the window, below the strip.
                let x = x.clamp(crate::browser::window::MARGIN, (width - crate::browser::window::MARGIN - REMOTE_WIDTH).max(crate::browser::window::MARGIN));
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

    fn draw_remote_keys<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect) {
        let controls = self.page_controls();
        let open = self.wall.zoomed().is_some_and(|key| controls.page == Some(key.page));
        let (left, right) = (panel.x0 + PADDING, panel.x1 - PADDING);
        let mut y = panel.y0 + PADDING;
        let mut buttons: Vec<(Button, Rect)> = Vec::new();
        let line = |y: f64| Rect::new(left, y, right, y);

        y = self.draw_title_row(canvas, &mut buttons, panel, line(y));
        y = self.draw_readout(canvas, &controls, open, line(y));
        y = draw_pad(canvas, &mut buttons, line(y));
        y = draw_page_keys(canvas, &mut buttons, line(y));
        y = draw_fastext_keys(canvas, &mut buttons, &controls, open, line(y));
        y = draw_subpage_row(canvas, &mut buttons, &controls, open, line(y));
        draw_toggles(canvas, &mut buttons, &controls, open, line(y));
        self.remote.buttons = buttons;
    }

    /// The title row: its name, dock or float, and away. Returns where the
    /// next row starts.
    fn draw_title_row<C: Canvas + ?Sized>(&mut self, canvas: &mut C, buttons: &mut Vec<(Button, Rect)>, panel: Rect, line: Rect) -> f64 {
        let (left, right, y) = (line.x0, line.x1, line.y0);
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
        y + TITLE_HEIGHT + GAP
    }

    /// The page number: the digits being entered, or the open page.
    /// Returns where the next row starts.
    fn draw_readout<C: Canvas + ?Sized>(&self, canvas: &mut C, controls: &PageControls, open: bool, line: Rect) -> f64 {
        let (left, right, y) = (line.x0, line.x1, line.y0);
        let readout = Rect::new(left, y, right, y + READOUT_HEIGHT);
        canvas.fill(readout, RADIUS, BUTTON);
        border(canvas, readout, 1.0, RULE);
        let number = if self.digits.is_empty() {
            controls.page.filter(|_| open).map_or_else(|| String::from("P---"), |page| format!("P{page:03X}"))
        } else {
            format!("P{:-<3}", self.digits)
        };
        centred(canvas, readout, &number, READOUT, WHITE);
        y + READOUT_HEIGHT + GAP
    }

    /// The remote's tips (requirement 12.12).
    pub(in crate::browser) fn remote_tips(&self, tips: &mut Vec<(Rect, String)>) {
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
                    format!("The page's {name} link ({letter}, or F{})", key.index().saturating_add(1))
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
/// The number pad. Returns where the next row starts.
fn draw_pad<C: Canvas + ?Sized>(canvas: &mut C, buttons: &mut Vec<(Button, Rect)>, line: Rect) -> f64 {
    let (left, right, y) = (line.x0, line.x1, line.y0);
    let key_width = (right - left - GAP * 2.0) / 3.0;
    for (index, digit) in PAD.iter().enumerate() {
        // 0 sits below the 8, alone on its row.
        let (row, column) = if index == PAD.len().saturating_sub(1) { (3, 1) } else { (index / PAD_COLUMNS, index % PAD_COLUMNS) };
        #[allow(clippy::cast_precision_loss, reason = "a few keys")]
        let (x, top) = (left + column as f64 * (key_width + GAP), y + row as f64 * (KEY_HEIGHT + GAP));
        let rect = Rect::new(x, top, x + key_width, top + KEY_HEIGHT);
        key(canvas, rect, digit, CONTROL, false, true);
        buttons.push((Button::Digit(digit.chars().next().unwrap_or('0')), rect));
    }
    y + 4.0 * (KEY_HEIGHT + GAP)
}

/// Page up and down. Returns where the next row starts.
fn draw_page_keys<C: Canvas + ?Sized>(canvas: &mut C, buttons: &mut Vec<(Button, Rect)>, line: Rect) -> f64 {
    let (left, right, y) = (line.x0, line.x1, line.y0);
    let half = (right - left - GAP) / 2.0;
    let up = Rect::new(left, y, left + half, y + SMALL_KEY_HEIGHT);
    let down = Rect::new(up.x1 + GAP, y, right, y + SMALL_KEY_HEIGHT);
    key(canvas, up, "Page \u{25B2}", ROW_TITLE, false, true);
    key(canvas, down, "Page \u{25BC}", ROW_TITLE, false, true);
    buttons.push((Button::PageUp, up));
    buttons.push((Button::PageDown, down));
    y + SMALL_KEY_HEIGHT + GAP * 2.0
}

/// Fastext: the four coloured keys, and Index below. Returns where the
/// next row starts.
fn draw_fastext_keys<C: Canvas + ?Sized>(canvas: &mut C, buttons: &mut Vec<(Button, Rect)>, controls: &PageControls, open: bool, line: Rect) -> f64 {
    let (left, right, mut y) = (line.x0, line.x1, line.y0);
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
    y + SMALL_KEY_HEIGHT + GAP * 2.0
}

/// Sub-pages: the one before, the count, the next. Returns where the next
/// row starts.
fn draw_subpage_row<C: Canvas + ?Sized>(canvas: &mut C, buttons: &mut Vec<(Button, Rect)>, controls: &PageControls, open: bool, line: Rect) -> f64 {
    let (left, right, y) = (line.x0, line.x1, line.y0);
    let steps = open && controls.subpages > 1;
    let step = SMALL_KEY_HEIGHT;
    let previous = Rect::new(left, y, left + step, y + SMALL_KEY_HEIGHT);
    let next = Rect::new(right - step, y, right, y + SMALL_KEY_HEIGHT);
    let count = Rect::new(previous.x1 + GAP, y, next.x0 - GAP, y + SMALL_KEY_HEIGHT);
    key(canvas, previous, "\u{2039}", ROW_TITLE, false, steps);
    key(canvas, next, "\u{203A}", ROW_TITLE, false, steps);
    let words = subpage_words(controls, steps);
    key(canvas, count, &words, ROW_STATUS, false, steps);
    buttons.push((Button::Control(PageControl::SubPage(false)), previous));
    buttons.push((Button::Control(PageControl::SubPage(true)), next));
    buttons.push((Button::Latest, count));
    y + SMALL_KEY_HEIGHT + GAP
}

/// Reveal, hold and mix: lit while on.
fn draw_toggles<C: Canvas + ?Sized>(canvas: &mut C, buttons: &mut Vec<(Button, Rect)>, controls: &PageControls, open: bool, line: Rect) {
    let (left, right, y) = (line.x0, line.x1, line.y0);
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
}
