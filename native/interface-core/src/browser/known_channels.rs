//! "Channels that know you", shown in the settings panel
//! (neoteletext-common's `docs/viewer-state.md` VS-112, DEC-6471): each
//! channel with a key, whether the key is a visit key or remembered, when
//! it was made and last used, and never the key or any part of it
//! (VS-116); and for each, Forget, Remember or Stop remembering, and Block;
//! and Forget all, once confirmed. Every control asks by the rows'
//! numbers, as strip input, so the interface can ask for nothing but
//! changes to rows it was shown. It keeps nothing of its own.

use alloc::format;
use alloc::string::String;

use super::widgets::{button, field};
use super::{Browser, Key, Modifiers, Output};
use crate::canvas::Canvas;
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::text::{centred, left_centred, right_centred};
use crate::theme::{DIM, MUTED, ROW_STATUS, ROW_TITLE, RULE_SOFT, WHITE};
use crate::viewer_state::{KnownRequest, KnownView, MAX_QUERY};

const CONTROL_HEIGHT: f64 = 28.0;
const GAP: f64 = 8.0;
const SMALL_BUTTON: f64 = 32.0;
const WIDE_BUTTON: f64 = 150.0;
const ROW_BUTTON: f64 = 96.0;
/// Rows close enough that several fit in the settings panel.
const ROW_HEIGHT: f64 = 26.0;
const SEARCH_WIDTH: f64 = 220.0;
const WHEN_WIDTH: f64 = 150.0;
/// How many rows one turn of the wheel moves the view.
const WHEEL_ROWS: usize = 3;

/// A row as drawn: its number, its buttons, and the row.
#[derive(Clone, Copy, Debug)]
struct Drawn {
    number: u32,
    forget: Rect,
    /// Remember or Stop remembering, and whether it is remembered; none for
    /// a blocked channel, which has no key.
    remember: Option<(Rect, bool)>,
    /// Block or Unblock, and whether it is blocked.
    block: (Rect, bool),
    rect: Rect,
}

/// Where the list's controls were last drawn.
#[derive(Debug, Default)]
struct Controls {
    area: Rect,
    search: Rect,
    forget_all: Rect,
    confirm: Rect,
    cancel: Rect,
    rows: alloc::vec::Vec<Drawn>,
    list: Rect,
}

/// The list's state in the browser.
pub(super) struct KnownUi {
    view: Option<KnownView>,
    open: bool,
    query: Editor,
    focused: bool,
    /// "Forget all" pressed: it asks to be confirmed.
    confirming: bool,
    controls: Controls,
}

impl Default for KnownUi {
    fn default() -> Self {
        Self { view: None, open: false, query: Editor::new(MAX_QUERY), focused: false, confirming: false, controls: Controls::default() }
    }
}

impl Browser {
    /// The list's rows as the trusted core sent them.
    pub fn set_known(&mut self, view: KnownView) {
        self.known.view = Some(view);
    }

    /// Whether the settings panel shows the list.
    #[must_use]
    pub fn known_open(&self) -> bool {
        self.known.open
    }

    /// The list shown, or no longer: the trusted core sends its rows only
    /// while it is.
    pub(super) fn set_known_open(&mut self, open: bool) {
        if self.known.open == open {
            return;
        }
        self.known.open = open;
        self.known.confirming = false;
        self.unfocus_known();
        if open {
            self.known.query.set("");
        }
        self.outputs.push(Output::Known(KnownRequest::Panel(open)));
    }

    fn unfocus_known(&mut self) {
        if self.known.focused {
            self.known.focused = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    fn known_query(&mut self, first: usize) {
        let text = String::from(self.known.query.text().trim());
        self.outputs.push(Output::Known(KnownRequest::Query { text, first: u32::try_from(first).unwrap_or(u32::MAX) }));
    }

    /// Whether the search field takes the keys.
    pub(super) fn known_editing(&self) -> bool {
        self.known.open && self.known.focused
    }

    /// A key while the list is shown; `None` when it is not the list's.
    pub(super) fn known_key(&mut self, key: Key<'_>, modifiers: Modifiers) -> Option<bool> {
        if !self.known.open {
            return None;
        }
        if !self.known.focused {
            if key == Key::Escape {
                if self.known.confirming {
                    self.known.confirming = false;
                } else {
                    self.set_known_open(false);
                }
                return Some(true);
            }
            return None;
        }
        let editor = &mut self.known.query;
        let before = String::from(editor.text());
        match key {
            Key::Text(_) | Key::Backspace | Key::Delete | Key::Left | Key::Right | Key::Home | Key::End | Key::Up | Key::Down | Key::PageUp | Key::PageDown => {
                editor.key(key, modifiers);
            }
            Key::Escape | Key::Tab | Key::Enter => self.unfocus_known(),
        }
        if self.known.query.text() != before {
            self.known_query(0);
        }
        Some(true)
    }

    /// The search field, while it has the keys.
    pub(super) fn known_field(&mut self) -> Option<&mut Editor> {
        self.known_editing().then_some(&mut self.known.query)
    }

    /// The wheel over the list: further rows are asked for.
    pub(super) fn known_scroll(&mut self, distance: f64) -> bool {
        if !self.known.open || !self.known.controls.area.contains(self.pointer) {
            return false;
        }
        let Some(view) = &self.known.view else { return true };
        let first = usize::try_from(view.first).unwrap_or(0);
        let total = usize::try_from(view.total).unwrap_or(0);
        let shown = self.known.controls.rows.len().max(1);
        let next = if distance > 0.0 { first.saturating_add(WHEEL_ROWS).min(total.saturating_sub(shown)) } else { first.saturating_sub(WHEEL_ROWS) };
        if next != first {
            self.known_query(next);
        }
        true
    }

    /// A press on the list; returns whether it was the list's.
    pub(super) fn known_press(&mut self, point: Point) -> bool {
        if !self.known.open || !self.known.controls.area.contains(point) {
            return false;
        }
        if self.known.controls.search.contains(point) {
            if !self.known.focused {
                self.known.focused = true;
                self.outputs.push(Output::TextInput(true));
            }
            return true;
        }
        self.unfocus_known();
        let controls = &self.known.controls;
        if controls.forget_all.contains(point) {
            self.known.confirming = !self.known.confirming;
        } else if controls.cancel.contains(point) {
            self.known.confirming = false;
        } else if controls.confirm.contains(point) {
            self.known.confirming = false;
            self.outputs.push(Output::Known(KnownRequest::ForgetAll));
        } else if let Some(&Drawn { number, forget, remember, block, .. }) = controls.rows.iter().find(|row| row.rect.contains(point)) {
            if forget.contains(point) {
                self.outputs.push(Output::Known(KnownRequest::Forget(number)));
            } else if let Some((_, remembered)) = remember.filter(|(rect, _)| rect.contains(point)) {
                self.outputs.push(Output::Known(KnownRequest::Remember { number, remembered: !remembered }));
            } else if block.0.contains(point) {
                self.outputs.push(Output::Known(KnownRequest::Block { number, blocked: !block.1 }));
            }
        }
        true
    }

    /// The list's controls: each row's buttons.
    pub(super) fn known_tips(&self, tips: &mut alloc::vec::Vec<(Rect, String)>) {
        if !self.known.open {
            return;
        }
        for drawn in &self.known.controls.rows {
            tips.push((drawn.forget, "Forget this channel: it will not know this browser again unless it asks and you act".into()));
            if let Some((rect, remembered)) = drawn.remember {
                let words =
                    if remembered { "Stop remembering: the channel knows this browser until it closes" } else { "Remember me on this channel on later visits" };
                tips.push((rect, words.into()));
            }
            let (rect, blocked) = drawn.block;
            let words = if blocked { "Let this channel ask to know you again" } else { "Forget this channel and decline whenever it asks to know you" };
            tips.push((rect, words.into()));
        }
        tips.push((self.known.controls.forget_all, "Forget every channel, after you confirm".into()));
    }

    /// The list, in `area` of the settings panel below its heading.
    pub(super) fn draw_known<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        let mut controls = Controls { area, ..Controls::default() };
        let (left, right) = (area.x0, area.x1);
        let view = self.known.view.clone().unwrap_or_default();
        let mut y = area.y0;

        // The search, the count and the trusted core's word, and "Forget all…".
        controls.search = Rect::new(left, y, (left + SEARCH_WIDTH).min(right), y + CONTROL_HEIGHT);
        field(canvas, controls.search, &self.known.query, self.known.focused);
        if self.known.query.text().is_empty() && !self.known.focused {
            left_centred(canvas, controls.search, GAP, "Search channels", ROW_STATUS, DIM);
        }
        controls.forget_all = Rect::new(right - WIDE_BUTTON, y, right, y + CONTROL_HEIGHT);
        button(canvas, controls.forget_all, "Forget all\u{2026}", self.known.confirming);
        let words_area = Rect::new(controls.search.x1 + GAP, y, controls.forget_all.x0 - GAP, y + CONTROL_HEIGHT);
        canvas.push_clip(words_area);
        if view.notice.is_empty() {
            if view.total > 0 {
                let shown = usize::try_from(view.first).unwrap_or(0).saturating_add(view.rows.len());
                let last = shown.min(usize::try_from(view.total).unwrap_or(0));
                let words = format!("{} to {last} of {}", view.first.saturating_add(1), view.total);
                right_centred(canvas, words_area, 0.0, &words, ROW_STATUS, MUTED);
            }
        } else {
            left_centred(canvas, words_area, 0.0, &view.notice, ROW_STATUS, WHITE);
        }
        canvas.pop_clip();
        y += CONTROL_HEIGHT + GAP;
        if self.known.confirming {
            let line = Rect::new(left, y, right, y + CONTROL_HEIGHT);
            left_centred(canvas, line, 0.0, "Forget every channel?", ROW_STATUS, WHITE);
            controls.cancel = Rect::new(right - ROW_BUTTON, y, right, y + CONTROL_HEIGHT);
            button(canvas, controls.cancel, "Cancel", false);
            controls.confirm = Rect::new(controls.cancel.x0 - GAP - ROW_BUTTON, y, controls.cancel.x0 - GAP, y + CONTROL_HEIGHT);
            button(canvas, controls.confirm, "Forget all", true);
            y += CONTROL_HEIGHT + GAP;
        }

        // The rows, with a scroll bar when more answer than are in view.
        controls.list = Rect::new(left, y, right, area.y1);
        crate::browser::page_list::draw_scroll_bar(
            canvas,
            controls.list,
            f64::from(view.total) * ROW_HEIGHT * 2.0,
            f64::from(view.first) * ROW_HEIGHT * 2.0,
            crate::surface::Surface::BROWSER,
        );
        let right = right - crate::browser::page_list::SCROLL_BAR_ROOM;
        canvas.push_clip(controls.list);
        if view.rows.is_empty() {
            let empty = if view.query.is_empty() { "No channel knows this browser" } else { "No channel matches" };
            centred(canvas, Rect::new(left, y, right, y + ROW_HEIGHT * 2.0), empty, ROW_TITLE, MUTED);
        }
        for row in &view.rows {
            // Each channel takes two lines: its identity, then what it
            // knows and its buttons.
            if y + ROW_HEIGHT * 2.0 > controls.list.y1 {
                break;
            }
            let rect = Rect::new(left, y, right, y + ROW_HEIGHT * 2.0);
            let hovered = rect.contains(self.pointer);
            if hovered {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            let first = Rect::new(left, y, right, y + ROW_HEIGHT);
            let forget = Rect::new(right - SMALL_BUTTON, y, right, y + ROW_HEIGHT);
            centred(canvas, forget, "\u{00D7}", ROW_TITLE, if hovered { WHITE } else { MUTED });
            let label = Rect::new(left, y, forget.x0 - GAP, y + ROW_HEIGHT);
            canvas.push_clip(label);
            left_centred(canvas, label, 0.0, &row.label, ROW_TITLE, WHITE);
            canvas.pop_clip();
            let second_y = first.y1;
            let block = Rect::new(right - ROW_BUTTON, second_y + 1.0, right, second_y + ROW_HEIGHT - 1.0);
            button(canvas, block, if row.blocked { "Unblock" } else { "Block" }, row.blocked);
            let remember = row.has_key.then(|| {
                let rect = Rect::new(block.x0 - GAP - ROW_BUTTON * 1.4, second_y + 1.0, block.x0 - GAP, second_y + ROW_HEIGHT - 1.0);
                button(canvas, rect, if row.remembered { "Stop remembering" } else { "Remember" }, row.remembered);
                (rect, row.remembered)
            });
            let end = remember.map_or(block.x0, |(rect, _)| rect.x0) - GAP;
            let kind = if row.blocked {
                String::from("Blocked")
            } else if row.remembered {
                String::from("Remembered")
            } else {
                String::from("Known for this visit")
            };
            let kind_area = Rect::new(left, second_y, end - WHEN_WIDTH * 2.0, second_y + ROW_HEIGHT);
            canvas.push_clip(kind_area);
            left_centred(canvas, kind_area, 0.0, &kind, ROW_STATUS, MUTED);
            canvas.pop_clip();
            if row.has_key {
                let made = Rect::new(end - WHEN_WIDTH * 2.0, second_y, end - WHEN_WIDTH, second_y + ROW_HEIGHT);
                right_centred(canvas, made, GAP, &format!("made {}", row.created), ROW_STATUS, MUTED);
                let used = Rect::new(end - WHEN_WIDTH, second_y, end, second_y + ROW_HEIGHT);
                right_centred(canvas, used, 0.0, &format!("used {}", row.used), ROW_STATUS, MUTED);
            }
            controls.rows.push(Drawn { number: row.number, forget, remember, block: (block, row.blocked), rect });
            y += ROW_HEIGHT * 2.0;
        }
        canvas.pop_clip();

        self.known.controls = controls;
    }
}

#[cfg(test)]
mod tests;
