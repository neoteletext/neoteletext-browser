//! Back's and Forward's lists (`history-and-bookmarks.md` section 2.9,
//! requirement 20.10): opened by holding the button or a secondary click,
//! each entry asked for by its number of steps.

use alloc::vec::Vec;

use crate::browser::popups::{GAP, ROW};
use crate::browser::{Browser, Output};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::library::TabList;
use crate::text::{HAIRLINE, border, left_centred};
use crate::theme::{BLACK, ROW_TITLE, RULE, SELECTED, WHITE};

const LIST_WIDTH: f64 = 460.0;
/// How long Back or Forward is held before its list opens.
const HOLD_MILLISECONDS: u64 = 500;

/// Back's and Forward's lists' state in the browser.
#[derive(Debug, Default)]
pub(super) struct TabListUi {
    tab_list: Option<TabList>,
    /// A list's entries drawn: steps, rect; and "Show all history".
    entries: Vec<(u8, Rect)>,
    show_all: Rect,
    list_area: Rect,
    /// Back or Forward held: which, since when, and whether its list opened.
    held: Option<(bool, u64, bool)>,
}

impl TabListUi {
    /// Whether Back's or Forward's list is open.
    pub(super) fn is_open(&self) -> bool {
        self.tab_list.is_some()
    }

    /// Closes the list; whether one was open.
    pub(super) fn close(&mut self) -> bool {
        self.tab_list.take().is_some()
    }
}

impl Browser {
    /// Back or Forward pressed: it acts when released, unless held long
    /// enough to open its list.
    pub(super) fn hold_back_or_forward(&mut self, forward: bool, now: u64) {
        self.tab_lists.held = Some((forward, now, false));
    }

    /// The main button released: a Back or Forward pressed and not held
    /// goes back or forward.
    pub(super) fn release_back_or_forward(&mut self) -> bool {
        let Some((forward, _, opened)) = self.tab_lists.held.take() else { return false };
        let over = if forward { self.regions.forward } else { self.regions.back };
        if !opened && over.contains(self.pointer) {
            self.outputs.push(if forward { Output::Forward } else { Output::Back });
        }
        true
    }

    /// Asks for Back's or Forward's list: held long enough, or a secondary click.
    pub(super) fn ask_tab_list(&mut self, forward: bool) {
        self.outputs.push(Output::TabList(forward));
    }

    /// Whether Back or Forward has been held long enough at `now`; its list
    /// is asked for once.
    pub(super) fn settle_hold(&mut self, now: u64) -> bool {
        let Some((forward, since, false)) = self.tab_lists.held else { return false };
        if now.saturating_sub(since) < HOLD_MILLISECONDS {
            return false;
        }
        self.tab_lists.held = Some((forward, since, true));
        self.ask_tab_list(forward);
        true
    }

    /// When the held button's list is due, if one is held.
    pub(super) fn hold_deadline(&self, now: u64) -> Option<u64> {
        let (_, since, opened) = self.tab_lists.held?;
        (!opened).then(|| since.saturating_add(HOLD_MILLISECONDS).saturating_sub(now).max(1))
    }

    /// The active tab's list one way, as the trusted core sent it.
    pub fn set_tab_list(&mut self, list: TabList) {
        self.tab_lists.tab_list = Some(list);
    }

    /// A press while a list is open: an entry goes there; "Show all
    /// history" opens the panel; anywhere else closes it.
    pub(super) fn tab_list_press(&mut self, point: Point) -> bool {
        let Some(list) = self.tab_lists.tab_list.take() else { return false };
        if let Some(&(steps, _)) = self.tab_lists.entries.iter().find(|(_, rect)| rect.contains(point)) {
            self.outputs.push(Output::GoTo { forward: list.forward, steps });
            return true;
        }
        if self.tab_lists.show_all.contains(point) {
            self.set_history_open(true);
            return true;
        }
        self.tab_lists.list_area.contains(point)
    }

    pub(super) fn draw_tab_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.tab_lists.entries.clear();
        self.tab_lists.show_all = Rect::ZERO;
        self.tab_lists.list_area = Rect::ZERO;
        let Some(list) = self.tab_lists.tab_list.clone() else { return };
        let anchor = if list.forward { self.regions.forward } else { self.regions.back };
        // The chapter stops, when there are any, after a rule of their own.
        let rows = list.entries.len().saturating_add(list.chapters.len()).saturating_add(1);
        #[allow(clippy::cast_precision_loss, reason = "at most eighteen rows")]
        let height = ROW * rows as f64;
        let area = Rect::new(anchor.x0, anchor.y1 + HAIRLINE, anchor.x0 + LIST_WIDTH, anchor.y1 + HAIRLINE + height);
        canvas.fill(area, 0.0, BLACK);
        border(canvas, area, HAIRLINE, RULE);
        let mut y = area.y0;
        let chapters_from = list.entries.len();
        for (index, entry) in list.entries.iter().chain(&list.chapters).enumerate() {
            if index == chapters_from {
                canvas.fill(Rect::new(area.x0, y, area.x1, y + HAIRLINE), 0.0, RULE);
            }
            let rect = Rect::new(area.x0, y, area.x1, y + ROW);
            if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, SELECTED);
            }
            let title = match entry.page {
                Some(page) => alloc::format!("{page:03X} {}", entry.title),
                None => entry.title.clone(),
            };
            canvas.push_clip(rect);
            left_centred(canvas, rect, GAP * 2.0, &title, ROW_TITLE, WHITE);
            canvas.pop_clip();
            self.tab_lists.entries.push((entry.steps, rect));
            y += ROW;
        }
        let show_all = Rect::new(area.x0, y, area.x1, y + ROW);
        canvas.fill(Rect::new(area.x0, y, area.x1, y + HAIRLINE), 0.0, RULE);
        if show_all.contains(self.pointer) {
            canvas.fill(show_all, 0.0, SELECTED);
        }
        left_centred(canvas, show_all, GAP * 2.0, "Show all history", ROW_TITLE, WHITE);
        self.tab_lists.show_all = show_all;
        self.tab_lists.list_area = area;
    }
}

#[cfg(test)]
mod tests;
