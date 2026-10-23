//! The wall's two layouts, a grid that grows and shrinks with the page
//! size and, below a size, a list; and the scroll that brings a page into
//! view.

use alloc::format;

use super::updates::{GLOW_WIDTH, glow, since};
use super::{Wall, WallPage};
use crate::canvas::{Canvas, PAGE_HEIGHT, PAGE_WIDTH, PageKey};
use crate::geometry::{Point, Rect};
use crate::size::grid_width;
use crate::theme::{CAPTION, CAPTION_TITLE, MUTED, ROW_NUMBER, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};
use crate::theme::{UPDATED, mix};
use crate::ui::{border, label, outline};

pub(super) const WALL_PADDING: f64 = 16.0;
const COLUMN_GAP: f64 = 12.0;
const ROW_GAP: f64 = 26.0;
pub(super) const CAPTION_GAP: f64 = 6.0;
pub(super) const CAPTION_HEIGHT: f64 = 18.0;
pub(super) const PAGE_ASPECT: f64 = PAGE_HEIGHT as f64 / PAGE_WIDTH as f64;
const LIST_PAGE_WIDTH: f64 = 120.0;
const LIST_ROW_PADDING: f64 = 8.0;
const LIST_GAP: f64 = 16.0;
const LIST_TITLE_RISE: f64 = 22.0;
const LIST_DETAIL_DROP: f64 = 2.0;
pub(super) const SCROLL_BAR_WIDTH: f64 = 4.0;
const SELECTED_BORDER: f64 = 3.0;
/// The gap between a page and its frame.
pub(super) const FRAME: f64 = 1.0;
pub(super) const CAPTION_NUMBER_WIDTH: f64 = 44.0;

impl Wall {
    /// Scrolls so that the row at `top` to `bottom` (in unscrolled
    /// content coordinates) is in view.
    fn bring_into_view(&mut self, top: f64, bottom: f64) {
        let view = self.area.height() - WALL_PADDING * 2.0;
        if top < self.scroll {
            self.scroll = top;
        } else if bottom > self.scroll + view {
            self.scroll = bottom - view;
        }
    }

    /// Draws the grid and returns its height.
    pub(super) fn draw_grid<C: Canvas + ?Sized>(&mut self, canvas: &mut C, pages: &[WallPage<'_>], selected: Option<PageKey>, now: u64) -> f64 {
        let area = self.area;
        let available = area.width() - WALL_PADDING * 2.0;
        let card_width = f64::from(grid_width(self.shown_width.value(now)));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small, positive column count")]
        let columns = (libm::floor((available + COLUMN_GAP) / (card_width + COLUMN_GAP)) as usize).max(1);
        let rows = pages.len().div_ceil(columns);
        let page_height = card_width * PAGE_ASPECT;
        let card_height = page_height + CAPTION_GAP + CAPTION_HEIGHT;
        let pitch = card_height + ROW_GAP;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let content = rows as f64 * card_height + rows.saturating_sub(1) as f64 * ROW_GAP;
        if let Some(key) = self.reveal.take()
            && let Some(index) = pages.iter().position(|page| page.key == key)
        {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let top = (index / columns) as f64 * pitch;
            self.bring_into_view(top, top + card_height);
        }
        let top = area.y0 + WALL_PADDING - self.scroll;
        let centre_x = f64::midpoint(area.x0, area.x1);
        for (index, page) in pages.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let in_row = columns.min(pages.len() - row * columns);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_width = in_row as f64 * card_width + (in_row - 1) as f64 * COLUMN_GAP;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let left = centre_x - row_width / 2.0 + column as f64 * (card_width + COLUMN_GAP);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let card_top = top + row as f64 * pitch;
            // Only pages in the panel are drawn and reported visible
            // (requirement 9.5).
            if card_top > area.y1 || card_top + card_height < area.y0 {
                continue;
            }
            let rect = Rect::new(left, card_top, left + card_width, card_top + page_height);
            self.draw_page(canvas, page.key, rect);
            // A frame, so a page's black edges stand apart from the panel;
            // amber and thick just after the page was updated, fading back.
            let glowing = glow(page.updated, now);
            if glowing > 0.0 {
                let width = FRAME + (GLOW_WIDTH - FRAME) * glowing;
                border(canvas, Rect::new(rect.x0 - width, rect.y0 - width, rect.x1 + width, rect.y1 + width), width, mix(RULE, UPDATED, glowing));
            } else {
                outline(canvas, Rect::new(rect.x0 - FRAME, rect.y0 - FRAME, rect.x1 + FRAME, rect.y1 + FRAME), RULE);
            }
            if selected == Some(page.key) {
                let ring = Rect::new(rect.x0 - SELECTED_BORDER, rect.y0 - SELECTED_BORDER, rect.x1 + SELECTED_BORDER, rect.y1 + SELECTED_BORDER);
                for width in 0..3_u8 {
                    let inset = f64::from(width);
                    outline(canvas, Rect::new(ring.x0 + inset, ring.y0 + inset, ring.x1 - inset, ring.y1 - inset), WHITE);
                }
            }
            let caption = Point::new(left, card_top + page_height + CAPTION_GAP);
            label(canvas, caption, &format!("{:03X}", page.key.page), CAPTION, WHITE);
            let title_at = Point::new(caption.x + CAPTION_NUMBER_WIDTH, caption.y + 1.0);
            let mut title_end = caption.x + card_width;
            if let Some(age) = since(page.updated, now) {
                let width = canvas.text_width(&age, CAPTION);
                title_end -= width + CAPTION_GAP;
                label(canvas, Point::new(caption.x + card_width - width, caption.y), &age, CAPTION, UPDATED);
            }
            canvas.text(Rect::new(title_at.x, title_at.y, title_end, title_at.y + CAPTION_HEIGHT), page.title, CAPTION_TITLE, MUTED);
        }
        content
    }

    /// Draws the list and returns its height.
    pub(super) fn draw_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, pages: &[WallPage<'_>], selected: Option<PageKey>) -> f64 {
        let area = self.area;
        let left = area.x0 + WALL_PADDING;
        let right = area.x1 - WALL_PADDING;
        let page_height = LIST_PAGE_WIDTH * PAGE_ASPECT;
        let row_height = page_height + LIST_ROW_PADDING * 2.0;
        if let Some(key) = self.reveal.take()
            && let Some(index) = pages.iter().position(|page| page.key == key)
        {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let top = index as f64 * row_height;
            self.bring_into_view(top, top + row_height);
        }
        let top = area.y0 + WALL_PADDING - self.scroll;
        for (index, page) in pages.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_top = top + index as f64 * row_height;
            if row_top > area.y1 || row_top + row_height < area.y0 {
                continue;
            }
            let row = Rect::new(left, row_top, right, row_top + row_height);
            if selected == Some(page.key) {
                canvas.fill(row, 0.0, SELECTED);
            }
            canvas.fill(Rect::new(left, row.y1 - 1.0, right, row.y1), 0.0, RULE_SOFT);
            let rect = Rect::new(
                left + LIST_ROW_PADDING,
                row_top + LIST_ROW_PADDING,
                left + LIST_ROW_PADDING + LIST_PAGE_WIDTH,
                row_top + LIST_ROW_PADDING + page_height,
            );
            self.draw_page(canvas, page.key, rect);
            let text_left = rect.x1 + LIST_GAP;
            let middle = rect.y0 + page_height / 2.0;
            label(canvas, Point::new(text_left, middle - LIST_TITLE_RISE), &format!("{:03X}", page.key.page), ROW_NUMBER, WHITE);
            canvas.text(Rect::new(text_left, middle + LIST_DETAIL_DROP, right, row.y1), page.title, ROW_TITLE, MUTED);
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let height = row_height * pages.len() as f64;
        height
    }
}
