//! The cookies panel of the trusted strip (`cookies.md` section 4.1,
//! requirement 21.6): opened from the settings panel and the menu; it
//! shows each site with cookies and its channels, their counts and sizes
//! and when last used, never a cookie's name or value, and asks for each
//! search, deletion and block by the rows' numbers, as strip input. It
//! keeps no cookie of its own.

use alloc::format;
use alloc::string::String;

use super::widgets::{button, field, panel, panel_rect};
use super::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::cookies::{CookiesRequest, CookiesView, MAX_QUERY};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::theme::{CAPTION, DIM, HEADING, MUTED, ROW_STATUS, ROW_TITLE, RULE_SOFT, WHITE};
use crate::ui::{centred, left_centred, right_centred};

const PANEL_WIDTH: f64 = 820.0;
const PANEL_HEIGHT: f64 = 680.0;
const PADDING: f64 = 20.0;
const HEADING_HEIGHT: f64 = 34.0;
const CONTROL_HEIGHT: f64 = 32.0;
const GAP: f64 = 8.0;
const SMALL_BUTTON: f64 = 32.0;
const WIDE_BUTTON: f64 = 150.0;
const BLOCK_BUTTON: f64 = 90.0;
const ROW_HEIGHT: f64 = 30.0;
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
    panel: Rect,
    close: Rect,
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

    /// Whether the cookies panel is open.
    #[must_use]
    pub fn cookies_open(&self) -> bool {
        self.cookies.open
    }

    pub(super) fn set_cookies_open(&mut self, open: bool) {
        if self.cookies.open == open {
            return;
        }
        if open {
            self.close_panels();
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

    /// A key while the panel is open; `None` when it is not the panel's.
    pub(super) fn cookies_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.cookies.open {
            return None;
        }
        if !self.cookies.focused {
            if key == Key::Escape {
                if self.cookies.confirming {
                    self.cookies.confirming = false;
                } else {
                    self.set_cookies_open(false);
                }
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

    /// Command-A in the search field.
    pub(super) fn cookies_select_all(&mut self) -> bool {
        if !self.cookies_editing() {
            return false;
        }
        self.cookies.query.select_all();
        true
    }

    /// The wheel over the panel: further rows are asked for.
    pub(super) fn cookies_scroll(&mut self, distance: f64) -> bool {
        if !self.cookies.open || !self.cookies.controls.panel.contains(self.pointer) {
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

    /// A press on the panel; returns whether it was the panel's.
    pub(super) fn cookies_press(&mut self, point: Point) -> bool {
        if !self.cookies.open || !self.cookies.controls.panel.contains(point) {
            return false;
        }
        let controls = &self.cookies.controls;
        if controls.close.contains(point) {
            self.set_cookies_open(false);
            return true;
        }
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

    /// The panel, over the window below the strip, while it is open.
    #[allow(clippy::too_many_lines, reason = "one part of the panel after another, in drawing order")]
    pub(super) fn draw_cookies_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let mut controls = Controls::default();
        if !self.cookies.open {
            self.cookies.controls = controls;
            return;
        }
        let area = panel_rect(self.body, PANEL_WIDTH, PANEL_HEIGHT);
        panel(canvas, area);
        controls.panel = area;
        let (left, right) = (area.x0 + PADDING, area.x1 - PADDING);
        let heading = Rect::new(left, area.y0 + PADDING, right, area.y0 + PADDING + HEADING_HEIGHT);
        left_centred(canvas, heading, 0.0, "Cookies and site data", HEADING, WHITE);
        controls.close = Rect::new(right - SMALL_BUTTON, heading.y0, right, heading.y0 + SMALL_BUTTON);
        button(canvas, controls.close, "\u{00D7}", false);
        let view = self.cookies.view.clone().unwrap_or_default();
        let mut y = heading.y1 + GAP;

        // The search, and "Delete all…".
        controls.delete_all = Rect::new(right - WIDE_BUTTON, y, right, y + CONTROL_HEIGHT);
        button(canvas, controls.delete_all, "Delete all\u{2026}", self.cookies.confirming);
        controls.search = Rect::new(left, y, controls.delete_all.x0 - GAP, y + CONTROL_HEIGHT);
        field(canvas, controls.search, &self.cookies.query, self.cookies.focused);
        if self.cookies.query.text().is_empty() && !self.cookies.focused {
            left_centred(canvas, controls.search, GAP, "Search sites", ROW_STATUS, DIM);
        }
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
        let footer = area.y1 - PADDING - CONTROL_HEIGHT;
        controls.list = Rect::new(left, y, right, footer - GAP);
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

        // The totals, where the rows are among all, and the trusted core's word.
        let footer = Rect::new(left, footer, right, footer + CONTROL_HEIGHT);
        let mut words = format!("{} cookies, {}", view.count, size(view.bytes));
        if view.total > 0 {
            let last = (usize::try_from(view.first).unwrap_or(0) + controls.rows.len()).min(usize::try_from(view.total).unwrap_or(0));
            words = format!("{words} \u{00B7} rows {} to {last} of {}", view.first + 1, view.total);
        }
        right_centred(canvas, footer, 0.0, &words, ROW_STATUS, MUTED);
        if !view.notice.is_empty() {
            canvas.push_clip(Rect::new(left, footer.y0, f64::midpoint(left, right), footer.y1));
            left_centred(canvas, footer, 0.0, &view.notice, ROW_STATUS, WHITE);
            canvas.pop_clip();
        }
        self.cookies.controls = controls;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Modifiers};
    use crate::cookies::CookieRow;
    use crate::test_canvas::Recorder;

    fn row(number: u32, site: bool, label: &str, blocked: bool) -> CookieRow {
        CookieRow { number, site, label: label.into(), count: 2, bytes: 2048, used: "Thursday 22 October".into(), blocked }
    }

    fn view() -> CookiesView {
        CookiesView {
            query: String::new(),
            first: 0,
            total: 3,
            rows: alloc::vec![row(1, true, "https://news.example", false), row(2, false, "/teletext/", false), row(3, false, "/sport/", true)],
            count: 4,
            bytes: 4096,
            notice: String::new(),
        }
    }

    fn open() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.set_cookies_open(true);
        browser.set_cookies(view());
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.take_outputs();
        browser
    }

    fn middle(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    /// Requirement 21.6: the panel lists sites and channels with their
    /// counts, sizes and last use, and asks by number: delete a site, a
    /// channel, block and unblock, delete all once confirmed.
    #[test]
    fn the_panel_lists_and_asks_by_number() {
        let mut canvas = Recorder::new();
        let mut browser = open();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        let texts: alloc::vec::Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        for expected in [
            "Cookies and site data",
            "https://news.example",
            "/teletext/",
            "2 \u{00B7} 2 KB",
            "Thursday 22 October",
            "Block",
            "Unblock",
            "4 cookies, 4 KB \u{00B7} rows 1 to 3 of 3",
        ] {
            assert!(texts.contains(&expected), "{expected} in {texts:?}");
        }
        let site = browser.cookies.controls.rows[0];
        assert!(site.block.is_none(), "a site is deleted, not blocked");
        browser.cookies_press(middle(site.delete));
        let channel = browser.cookies.controls.rows[1];
        browser.cookies_press(middle(channel.block.unwrap().0));
        let blocked = browser.cookies.controls.rows[2];
        browser.cookies_press(middle(blocked.block.unwrap().0));
        assert_eq!(
            browser.take_outputs(),
            [
                Output::Cookies(CookiesRequest::Delete(site.number)),
                Output::Cookies(CookiesRequest::Block { number: channel.number, blocked: true }),
                Output::Cookies(CookiesRequest::Block { number: blocked.number, blocked: false }),
            ]
        );
        let all = browser.cookies.controls.delete_all;
        browser.cookies_press(middle(all));
        assert_eq!(browser.take_outputs(), [], "not until confirmed");
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let confirm = browser.cookies.controls.confirm;
        browser.cookies_press(middle(confirm));
        assert_eq!(browser.take_outputs(), [Output::Cookies(CookiesRequest::DeleteAll)]);
    }

    /// The search follows what is typed; Escape closes the panel.
    #[test]
    fn the_search_asks_for_rows() {
        let mut browser = open();
        let search = browser.cookies.controls.search;
        browser.cookies_press(middle(search));
        browser.key(Key::Text("n"), Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::Cookies(CookiesRequest::Query { text: "n".into(), first: 0 })]);
        browser.key(Key::Escape, Modifiers::default(), 0);
        browser.key(Key::Escape, Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Cookies(CookiesRequest::Panel(false))]);
        assert!(!browser.cookies_open());
    }
}
