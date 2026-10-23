//! Cookie management, a tab of the developer console (`cookies.md` section
//! 4.1, requirement 21.6), which the settings panel opens too: it shows
//! each site with cookies and its channels, their counts and sizes and when
//! last used, never a cookie's name or value, and asks for each search,
//! deletion and block by the rows' numbers, as strip input. It keeps no
//! cookie of its own.

use alloc::format;
use alloc::string::String;

use super::widgets::{button, field};
use super::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::cookies::{CookiesRequest, CookiesView, MAX_QUERY};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::text::{centred, left_centred, right_centred};
use crate::theme::{CAPTION, DIM, MUTED, ROW_STATUS, ROW_TITLE, RULE_SOFT, WHITE};

const CONTROL_HEIGHT: f64 = 28.0;
const GAP: f64 = 8.0;
const SMALL_BUTTON: f64 = 32.0;
const WIDE_BUTTON: f64 = 150.0;
const BLOCK_BUTTON: f64 = 90.0;
/// Rows close enough that several fit in the docked console.
const ROW_HEIGHT: f64 = 26.0;
const SEARCH_WIDTH: f64 = 320.0;
/// A channel's row is set in under its site's.
const INDENT: f64 = 24.0;
const COUNT_WIDTH: f64 = 110.0;
const USED_WIDTH: f64 = 210.0;
/// How many rows one turn of the wheel moves the view.
const WHEEL_ROWS: usize = 3;
const KILOBYTE: u64 = 1024;

/// A row as drawn: its number, its delete button, its block button and
/// whether the channel is blocked (a channel's only), and the row.
#[derive(Clone, Copy, Debug)]
struct Drawn {
    number: u32,
    delete: Rect,
    block: Option<(Rect, bool)>,
    rect: Rect,
}

/// Where the panel's controls were last drawn.
#[derive(Debug, Default)]
struct Controls {
    area: Rect,
    search: Rect,
    delete_all: Rect,
    confirm: Rect,
    cancel: Rect,
    rows: alloc::vec::Vec<Drawn>,
    list: Rect,
}

/// The panel's state in the browser.
pub(super) struct CookiesUi {
    view: Option<CookiesView>,
    open: bool,
    query: Editor,
    focused: bool,
    /// "Delete all" pressed: it asks to be confirmed.
    confirming: bool,
    controls: Controls,
}

impl Default for CookiesUi {
    fn default() -> Self {
        Self { view: None, open: false, query: Editor::new(MAX_QUERY), focused: false, confirming: false, controls: Controls::default() }
    }
}

/// A size in words.
fn size(bytes: u64) -> String {
    if bytes < KILOBYTE { format!("{bytes} bytes") } else { format!("{} KB", bytes.div_ceil(KILOBYTE)) }
}

impl Browser {
    /// The cookies panel's rows as the trusted core sent them.
    pub fn set_cookies(&mut self, view: CookiesView) {
        self.cookies.view = Some(view);
    }

    /// Whether the console shows the cookies' tab.
    #[must_use]
    pub fn cookies_open(&self) -> bool {
        self.cookies.open
    }

    /// The cookies' tab shown, or no longer: the trusted core sends its
    /// rows only while it is.
    pub(super) fn set_cookies_open(&mut self, open: bool) {
        if self.cookies.open == open {
            return;
        }
        self.cookies.open = open;
        self.cookies.confirming = false;
        self.unfocus_cookies();
        if open {
            self.cookies.query.set("");
        }
        self.outputs.push(Output::Cookies(CookiesRequest::Panel(open)));
    }

    fn unfocus_cookies(&mut self) {
        if self.cookies.focused {
            self.cookies.focused = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    fn cookies_query(&mut self, first: usize) {
        let text = String::from(self.cookies.query.text().trim());
        self.outputs.push(Output::Cookies(CookiesRequest::Query { text, first: u32::try_from(first).unwrap_or(u32::MAX) }));
    }

    /// Whether the search field takes the keys.
    pub(super) fn cookies_editing(&self) -> bool {
        self.cookies.open && self.cookies.focused
    }

    /// A key while the cookies are shown; `None` when it is not theirs.
    pub(super) fn cookies_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.cookies.open {
            return None;
        }
        if !self.cookies.focused {
            if key == Key::Escape && self.cookies.confirming {
                self.cookies.confirming = false;
                return Some(true);
            }
            return None;
        }
        let editor = &mut self.cookies.query;
        let before = String::from(editor.text());
        match key {
            Key::Text(text) => editor.insert(text),
            Key::Backspace => editor.backspace(),
            Key::Delete => editor.delete(),
            Key::Left => editor.left(shift),
            Key::Right => editor.right(shift),
            Key::Home | Key::Up => editor.home(shift),
            Key::End | Key::Down => editor.end(shift),
            Key::Escape | Key::Tab | Key::Enter => self.unfocus_cookies(),
        }
        if self.cookies.query.text() != before {
            self.cookies_query(0);
        }
        Some(true)
    }

    /// The search field, while it has the keys.
    pub(super) fn cookies_field(&mut self) -> Option<&mut Editor> {
        self.cookies_editing().then_some(&mut self.cookies.query)
    }

    /// The wheel over the cookies: further rows are asked for.
    pub(super) fn cookies_scroll(&mut self, distance: f64) -> bool {
        if !self.cookies.open || !self.cookies.controls.area.contains(self.pointer) {
            return false;
        }
        let Some(view) = &self.cookies.view else { return true };
        let first = usize::try_from(view.first).unwrap_or(0);
        let total = usize::try_from(view.total).unwrap_or(0);
        let shown = self.cookies.controls.rows.len().max(1);
        let next = if distance > 0.0 { first.saturating_add(WHEEL_ROWS).min(total.saturating_sub(shown)) } else { first.saturating_sub(WHEEL_ROWS) };
        if next != first {
            self.cookies_query(next);
        }
        true
    }

    /// A press on the cookies; returns whether it was theirs.
    pub(super) fn cookies_press(&mut self, point: Point) -> bool {
        if !self.cookies.open || !self.cookies.controls.area.contains(point) {
            return false;
        }
        let controls = &self.cookies.controls;
        if controls.search.contains(point) {
            if !self.cookies.focused {
                self.cookies.focused = true;
                self.outputs.push(Output::TextInput(true));
            }
            return true;
        }
        self.unfocus_cookies();
        let controls = &self.cookies.controls;
        if controls.delete_all.contains(point) {
            self.cookies.confirming = !self.cookies.confirming;
        } else if controls.cancel.contains(point) {
            self.cookies.confirming = false;
        } else if controls.confirm.contains(point) {
            self.cookies.confirming = false;
            self.outputs.push(Output::Cookies(CookiesRequest::DeleteAll));
        } else if let Some(&Drawn { number, delete, block, .. }) = controls.rows.iter().find(|row| row.rect.contains(point)) {
            if delete.contains(point) {
                self.outputs.push(Output::Cookies(CookiesRequest::Delete(number)));
            } else if let Some((_, blocked)) = block.filter(|(rect, _)| rect.contains(point)) {
                self.outputs.push(Output::Cookies(CookiesRequest::Block { number, blocked: !blocked }));
            }
        }
        true
    }

    /// The cookies' controls: each row's delete and block buttons.
    pub(super) fn cookies_tips(&self, tips: &mut alloc::vec::Vec<(Rect, String)>) {
        let rows = self.cookies.view.as_ref().map_or(&[][..], |view| view.rows.as_slice());
        for drawn in &self.cookies.controls.rows {
            let Some(row) = rows.iter().find(|row| row.number == drawn.number) else { continue };
            let whose = if row.site { "this site and all its channels" } else { "this channel" };
            tips.push((drawn.delete, format!("Delete the cookies of {whose}")));
            if let Some((rect, blocked)) = drawn.block {
                let words = if blocked { "Let this channel keep cookies again" } else { "Keep no cookies for this channel" };
                tips.push((rect, words.into()));
            }
        }
        tips.push((self.cookies.controls.delete_all, "Delete every cookie, after you confirm".into()));
    }

    /// The cookies, in the console's `area` below its tabs.
    pub(super) fn draw_cookies<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        let mut controls = Controls { area, ..Controls::default() };
        let (left, right) = (area.x0, area.x1);
        let view = self.cookies.view.clone().unwrap_or_default();
        let mut y = area.y0;

        // The search, the totals and the trusted core's word, and "Delete all…".
        controls.search = Rect::new(left, y, (left + SEARCH_WIDTH).min(right), y + CONTROL_HEIGHT);
        field(canvas, controls.search, &self.cookies.query, self.cookies.focused);
        if self.cookies.query.text().is_empty() && !self.cookies.focused {
            left_centred(canvas, controls.search, GAP, "Search sites", ROW_STATUS, DIM);
        }
        controls.delete_all = Rect::new(right - WIDE_BUTTON, y, right, y + CONTROL_HEIGHT);
        button(canvas, controls.delete_all, "Delete all\u{2026}", self.cookies.confirming);
        let words_area = Rect::new(controls.search.x1 + GAP, y, controls.delete_all.x0 - GAP, y + CONTROL_HEIGHT);
        let mut words = format!("{} cookies, {}", view.count, size(view.bytes));
        if view.total > 0 {
            let shown = usize::try_from(view.first).unwrap_or(0) + view.rows.len();
            let last = shown.min(usize::try_from(view.total).unwrap_or(0));
            words = format!("{words} \u{00B7} rows {} to {last} of {}", view.first + 1, view.total);
        }
        canvas.push_clip(words_area);
        right_centred(canvas, words_area, 0.0, &words, ROW_STATUS, MUTED);
        if !view.notice.is_empty() {
            left_centred(canvas, words_area, 0.0, &view.notice, ROW_STATUS, WHITE);
        }
        canvas.pop_clip();
        y += CONTROL_HEIGHT + GAP;
        if self.cookies.confirming {
            let line = Rect::new(left, y, right, y + CONTROL_HEIGHT);
            left_centred(canvas, line, 0.0, "Delete every cookie? Channels will sign you out.", ROW_STATUS, WHITE);
            controls.cancel = Rect::new(right - WIDE_BUTTON, y, right, y + CONTROL_HEIGHT);
            button(canvas, controls.cancel, "Cancel", false);
            controls.confirm = Rect::new(controls.cancel.x0 - GAP - WIDE_BUTTON, y, controls.cancel.x0 - GAP, y + CONTROL_HEIGHT);
            button(canvas, controls.confirm, "Delete all", true);
            y += CONTROL_HEIGHT + GAP;
        }

        // The rows: each site, then its channels set in.
        controls.list = Rect::new(left, y, right, area.y1);
        canvas.push_clip(controls.list);
        if view.rows.is_empty() {
            let empty = if view.query.is_empty() { "No cookies are kept" } else { "No site matches" };
            centred(canvas, Rect::new(left, y, right, y + ROW_HEIGHT * 2.0), empty, ROW_TITLE, MUTED);
        }
        for row in &view.rows {
            if y + ROW_HEIGHT > controls.list.y1 {
                break;
            }
            let rect = Rect::new(left, y, right, y + ROW_HEIGHT);
            let hovered = rect.contains(self.pointer);
            if hovered {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            let delete = Rect::new(right - SMALL_BUTTON, y, right, y + ROW_HEIGHT);
            centred(canvas, delete, "\u{00D7}", ROW_TITLE, if hovered { WHITE } else { MUTED });
            let block = (!row.site).then(|| {
                let rect = Rect::new(delete.x0 - GAP - BLOCK_BUTTON, y + 2.0, delete.x0 - GAP, y + ROW_HEIGHT - 2.0);
                button(canvas, rect, if row.blocked { "Unblock" } else { "Block" }, row.blocked);
                (rect, row.blocked)
            });
            let end = block.map_or(delete.x0, |(rect, _)| rect.x0) - GAP;
            let used = Rect::new(end - USED_WIDTH, y, end, y + ROW_HEIGHT);
            right_centred(canvas, used, 0.0, &row.used, ROW_STATUS, MUTED);
            let count = Rect::new(used.x0 - COUNT_WIDTH, y, used.x0 - GAP, y + ROW_HEIGHT);
            let counted = if row.blocked && row.count == 0 { String::from("blocked") } else { format!("{} \u{00B7} {}", row.count, size(row.bytes)) };
            right_centred(canvas, count, 0.0, &counted, ROW_STATUS, MUTED);
            let indent = if row.site { 0.0 } else { INDENT };
            let label = Rect::new(left + indent, y, count.x0 - GAP, y + ROW_HEIGHT);
            canvas.push_clip(label);
            left_centred(canvas, label, 0.0, &row.label, if row.site { CAPTION } else { ROW_TITLE }, WHITE);
            canvas.pop_clip();
            controls.rows.push(Drawn { number: row.number, delete, block, rect });
            y += ROW_HEIGHT;
        }
        canvas.pop_clip();

        self.cookies.controls = controls;
    }
}

#[cfg(test)]
mod tests;
