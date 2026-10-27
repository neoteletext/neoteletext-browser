//! Full screen, the teletext page only (`full-screen.md`, decision 85):
//! the open page alone, filling the window at its own proportions in its
//! screen colour, the strip brought down by the pointer at the top edge or
//! by the browser's own keys, a notice of how to leave, and while keys go
//! to a channel the trusted badge in the margin. Anything that needs the
//! window ends it.

use alloc::vec::Vec;

use crate::browser::Browser;
use crate::browser::bookmarks_bar::BAR_HEIGHT;
use crate::browser::window::BODY_TOP;
use crate::canvas::{Canvas, PageKey};
use crate::channel::page_number;
use crate::geometry::{Point, Rect};
use crate::text::{border, left_centred};
use crate::theme::{BUTTON, ROW_TITLE, RULE, WHITE};
use crate::wall::WallPage;

/// The top edge the pointer rests on to bring the strip down, in points:
/// the display's last rows, where the pointer stops.
const TOP_EDGE: f64 = 2.0;
/// How long the pointer rests at the top edge before the strip comes:
/// macOS's full-screen space shows its menu bar there too, and the wait
/// keeps the two from coming together (section 9 of the design).
const EDGE_WAIT_MILLISECONDS: u64 = 500;
/// How long the strip shows for a key the browser keeps for itself
/// (section 4).
const KEY_STRIP_MILLISECONDS: u64 = 3_000;
/// How long "Full screen: Escape to leave" shows on entering (section 2).
const NOTICE_MILLISECONDS: u64 = 3_000;
/// The pointer hides after this long without moving (section 1).
const POINTER_HIDE_MILLISECONDS: u64 = 2_000;
/// Two presses this close in time and place are a double click: the
/// default of Windows (`GetDoubleClickTime`) and of macOS's double-click
/// speed setting's middle; the platforms do not tell the interface theirs.
const DOUBLE_CLICK_MILLISECONDS: u64 = 500;
const DOUBLE_CLICK_DISTANCE: f64 = 4.0;
/// The notice: what it says, and its box.
const NOTICE: &str = "Full screen: Escape to leave";
const NOTICE_PADDING: f64 = 10.0;
const NOTICE_HEIGHT: f64 = 30.0;
const NOTICE_RADIUS: f64 = 4.0;
/// The notice and the badge keep this far from the display's edges and
/// from the page.
const MARGIN_GAP: f64 = 12.0;
/// The typing badge's row in the margin.
#[cfg(feature = "interaction")]
const BADGE_ROW: f64 = 36.0;

/// What the window was when full screen began: a change of any of them
/// needs the window, and ends it (section 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Entered {
    channel: u32,
    tabs: usize,
    active: usize,
    remote: bool,
}

/// Full screen's state in the browser.
#[derive(Debug, Default)]
pub(super) struct FullScreenUi {
    /// Since when full screen is on, and the window as it was.
    on: Option<(u64, Entered)>,
    /// Since when the pointer has rested at the top edge.
    edge: Option<u64>,
    /// The strip is down, brought by the pointer at the edge; it goes when
    /// the pointer leaves it.
    strip: bool,
    /// The strip is down for a key until then.
    strip_until: Option<u64>,
    /// When the pointer last moved.
    moved: u64,
    /// The last press on the open page, for a double click.
    last_press: Option<(Point, u64)>,
    /// What the last frame showed: the notice, the pointer hidden, the
    /// strip; a change of any by time alone needs a frame.
    drawn: (bool, bool, bool),
}

impl Browser {
    /// Whether the window shows the page alone.
    #[must_use]
    pub fn full_screen(&self) -> bool {
        self.full_screen.on.is_some()
    }

    /// The system's full-screen key or the menu's item: enters full screen
    /// with the open page, opening the selected or first page on the wall
    /// first, or leaves it. Returns whether anything changed.
    pub fn toggle_full_screen(&mut self, now: u64) -> bool {
        if self.full_screen() {
            self.leave_full_screen();
            return true;
        }
        if self.wall.zoomed().is_none() {
            let selected = self.selected.and_then(|index| self.page_key(index));
            let Some(key) = selected.or_else(|| self.wall.visible().first().copied()) else { return false };
            self.wall.zoom(key, now);
        }
        // What lies over the window would need it at once.
        self.close_panels();
        self.close_popup();
        self.stop_editing();
        let entered = Entered { channel: self.channel.channel, tabs: self.tabs.len(), active: self.active, remote: self.remote_shown() };
        self.full_screen = FullScreenUi { on: Some((now, entered)), moved: now, ..FullScreenUi::default() };
        true
    }

    /// Leaves full screen, the open page as it was (section 5).
    pub fn leave_full_screen(&mut self) {
        self.full_screen = FullScreenUi::default();
    }

    /// Whether the pointer is hidden: in full screen, two seconds after it
    /// last moved, while the strip is up.
    #[must_use]
    pub fn pointer_hidden(&self, now: u64) -> bool {
        self.full_screen() && !self.full_screen_strip(now) && now.saturating_sub(self.full_screen.moved) >= POINTER_HIDE_MILLISECONDS
    }

    /// The strip is down over the page.
    fn full_screen_strip(&self, now: u64) -> bool {
        self.full_screen.strip || self.full_screen.strip_until.is_some_and(|until| now < until)
    }

    /// Where the strip lies when it is down.
    fn full_screen_strip_area(&self, width: f64) -> Rect {
        Rect::new(0.0, 0.0, width, BODY_TOP + if self.bar_shown() { BAR_HEIGHT } else { 0.0 })
    }

    /// Something needs the window, which ends full screen (section 5): the
    /// page closed or gone, another tab or channel, a panel or list of the
    /// strip, the console, the remote, the strip's question, a certificate
    /// refused.
    fn full_screen_needs_window(&self) -> bool {
        let Some((_, entered)) = self.full_screen.on else { return false };
        let now = Entered { channel: self.channel.channel, tabs: self.tabs.len(), active: self.active, remote: self.remote_shown() };
        #[cfg(feature = "links")]
        let question = self.prompt_shown();
        #[cfg(not(feature = "links"))]
        let question = false;
        #[cfg(feature = "phone-keyboard")]
        let phone = self.phone_open();
        #[cfg(not(feature = "phone-keyboard"))]
        let phone = false;
        now != entered
            || self.wall.zoomed().is_none()
            || self.popup_open()
            || self.settings_open()
            || self.history_open()
            || self.bookmarks_open()
            || self.star_open()
            || self.cookies_open()
            || self.console.open()
            || phone
            || question
            || self.strip.refused.is_some()
    }

    /// The pointer moved at `now` in full screen: the strip comes down after
    /// a rest at the top edge, and goes when the pointer leaves it.
    pub(super) fn full_screen_moved(&mut self, point: Point, width: f64, now: u64) -> bool {
        if !self.full_screen() {
            return false;
        }
        let was_hidden = self.pointer_hidden(now);
        self.full_screen.moved = now;
        let strip_before = self.full_screen.strip;
        if point.y <= TOP_EDGE && point.y >= 0.0 {
            self.full_screen.edge = self.full_screen.edge.or(Some(now));
        } else {
            self.full_screen.edge = None;
            if self.full_screen.strip && !self.full_screen_strip_area(width).contains(point) {
                self.full_screen.strip = false;
            }
        }
        was_hidden || strip_before != self.full_screen.strip
    }

    /// A key the browser keeps for itself shows the strip as it acts.
    pub(super) fn full_screen_key(&mut self, now: u64) {
        if self.full_screen() {
            self.full_screen.strip_until = Some(now.saturating_add(KEY_STRIP_MILLISECONDS));
        }
    }

    /// A press on the open page at `now`: whether it was the second of a
    /// double click, which enters full screen (section 2). A press on a
    /// link is the link's, and never counts.
    pub(super) fn double_click(&mut self, point: Point, now: u64) -> bool {
        #[cfg(feature = "links")]
        if self.link_cursor().is_some() {
            self.full_screen.last_press = None;
            return false;
        }
        let previous = self.full_screen.last_press.replace((point, now));
        let double = previous.is_some_and(|(at, when)| {
            now.saturating_sub(when) <= DOUBLE_CLICK_MILLISECONDS && libm::hypot(point.x - at.x, point.y - at.y) <= DOUBLE_CLICK_DISTANCE
        });
        if double {
            self.full_screen.last_press = None;
        }
        double
    }

    /// Whether a press in full screen at the pointer is the strip's, which
    /// is then down; any other press is the page's.
    pub(super) fn full_screen_press_is_strip(&self, now: u64) -> bool {
        self.full_screen_strip(now) && self.pointer.y < self.full_screen_strip_area(self.body.width()).y1
    }

    /// The next time full screen changes by itself: the strip at the edge,
    /// the strip for a key or the notice going, the pointer hiding.
    pub(super) fn full_screen_deadline(&self, now: u64) -> Option<u64> {
        let (since, _) = self.full_screen.on?;
        let until = |end: u64| (end > now).then(|| end.saturating_sub(now));
        let edge = self.full_screen.edge.filter(|_| !self.full_screen.strip).map(|at| at.saturating_add(EDGE_WAIT_MILLISECONDS));
        let ends = [
            edge,
            self.full_screen.strip_until,
            Some(since.saturating_add(NOTICE_MILLISECONDS)),
            Some(self.full_screen.moved.saturating_add(POINTER_HIDE_MILLISECONDS)),
        ];
        ends.into_iter().flatten().filter_map(until).min().map(|wait| wait.max(1))
    }

    /// Advances what full screen waits on; returns whether what it shows
    /// changed.
    pub(super) fn full_screen_settle(&mut self, now: u64) -> bool {
        if !self.full_screen() {
            return false;
        }
        if !self.full_screen.strip && self.full_screen.edge.is_some_and(|at| now.saturating_sub(at) >= EDGE_WAIT_MILLISECONDS) {
            self.full_screen.strip = true;
        }
        if self.full_screen.strip_until.is_some_and(|until| now >= until) {
            self.full_screen.strip_until = None;
        }
        self.full_screen_shows(now) != self.full_screen.drawn
    }

    /// What full screen shows at `now`: the notice, the pointer hidden,
    /// the strip.
    fn full_screen_shows(&self, now: u64) -> (bool, bool, bool) {
        let (since, _) = self.full_screen.on.unwrap_or_default();
        (now.saturating_sub(since) < NOTICE_MILLISECONDS, self.pointer_hidden(now), self.full_screen_strip(now))
    }

    /// Draws full screen, or leaves it when something needs the window;
    /// returns whether it drew.
    pub(super) fn draw_full_screen<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, height: f64, now: u64) -> bool {
        if !self.full_screen() {
            return false;
        }
        if self.full_screen_needs_window() {
            self.leave_full_screen();
            return false;
        }
        let area = Rect::new(0.0, 0.0, width, height);
        self.body = area;
        let shows = self.full_screen_shows(now);
        self.full_screen.drawn = shows;
        let (notice, _, strip) = shows;
        // What the window's layout drew is not on the screen, so none of it
        // takes the pointer.
        self.tab_strip_forget(!strip);
        self.tips_forget();
        self.controls_forget();
        let (channel, updates) = (&self.channel, &self.updates);
        let pages: Vec<WallPage<'_>> = crate::channel::bank_indices(self.bank)
            .filter_map(|index| {
                let info = channel.pages.get(index)?.as_ref()?;
                let key = PageKey { channel: channel.channel, page: page_number(index)? };
                let updated_at = updates.iter().find(|&&(which, _)| which == index).map(|&(_, at)| at);
                Some(WallPage { key, title: &info.title, updated: updated_at })
            })
            .collect();
        let page = self.wall.draw_full_screen(canvas, area, &pages, now);
        drop(pages);
        #[cfg(feature = "links")]
        self.draw_links(canvas);
        #[cfg(feature = "interaction")]
        self.draw_terminal(canvas, now);
        self.report_visible();
        let Some(page) = page else {
            // The page left the wall: the window comes back next frame.
            self.leave_full_screen();
            return true;
        };
        if notice {
            draw_notice(canvas, area, page);
        }
        #[cfg(feature = "interaction")]
        self.draw_full_screen_badge(canvas, area, page);
        if strip {
            self.draw_toolbar(canvas, width);
            self.draw_bar(canvas, crate::browser::toolbar::TOOLBAR_TOP + crate::browser::toolbar::TOOLBAR_HEIGHT, width);
            self.draw_tab_strip(canvas, width);
            self.draw_tab_overflow(canvas, width, height);
        }
        true
    }

    /// While keys go to a channel, the strip's typing badge in the margin,
    /// since the strip that says it is up (section 3).
    #[cfg(feature = "interaction")]
    fn draw_full_screen_badge<C: Canvas + ?Sized>(&self, canvas: &mut C, area: Rect, page: Rect) {
        if self.typing_target().is_none() {
            return;
        }
        let right_margin = Rect::new(page.x1 + MARGIN_GAP, area.y0 + MARGIN_GAP, area.x1 - MARGIN_GAP, area.y0 + MARGIN_GAP + BADGE_ROW);
        let below = Rect::new(page.x0, page.y1 + MARGIN_GAP / 2.0, page.x1, (page.y1 + MARGIN_GAP / 2.0 + BADGE_ROW).min(area.y1));
        let row = if right_margin.width() >= below.width() / 2.0 { right_margin } else { below };
        let _ = self.draw_typing_badge(canvas, row);
    }
}

/// "Full screen: Escape to leave" in the margin beside the page, never
/// over it: the side margin when it is wide enough, else above.
fn draw_notice<C: Canvas + ?Sized>(canvas: &mut C, area: Rect, page: Rect) {
    let width = canvas.text_width(NOTICE, ROW_TITLE) + NOTICE_PADDING * 2.0;
    let Some(place) = margin_place(area, page, width, NOTICE_HEIGHT) else { return };
    canvas.fill(place, NOTICE_RADIUS, BUTTON);
    border(canvas, place, 1.0, RULE);
    left_centred(canvas, place, NOTICE_PADDING, NOTICE, ROW_TITLE, WHITE);
}

/// A box of `width` by `height` in the margin around `page` within
/// `area`: the left margin's top when it is wide enough, else the top
/// margin's middle; none when neither is.
fn margin_place(area: Rect, page: Rect, width: f64, height: f64) -> Option<Rect> {
    if page.x0 - area.x0 >= width + MARGIN_GAP * 2.0 {
        let left = area.x0 + MARGIN_GAP;
        return Some(Rect::new(left, area.y0 + MARGIN_GAP, left + width, area.y0 + MARGIN_GAP + height));
    }
    if page.y0 - area.y0 >= height + MARGIN_GAP * 2.0 {
        let left = f64::midpoint(area.x0, area.x1) - width / 2.0;
        return Some(Rect::new(left, area.y0 + MARGIN_GAP, left + width, area.y0 + MARGIN_GAP + height));
    }
    None
}

#[cfg(test)]
mod tests;
