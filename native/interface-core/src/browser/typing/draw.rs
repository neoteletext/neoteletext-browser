//! Typing drawn: the channel's cursor and the viewer's echo over the open
//! page, and the strip's badge saying where keys go (REQ-INT-131,
//! REQ-INT-134).

use alloc::format;
use core::fmt::Write;

use neoteletext_input::cursor::BLOCK;

use super::{BAR_SHARE, BLINK_MILLISECONDS, BLOCK_LINE_SHARE, ECHO_TEXT_SHARE};
use crate::browser::Browser;
use crate::canvas::{COLUMNS, Canvas, PageKey, ROWS, Style};
use crate::geometry::Rect;
use crate::text::left_centred;
use crate::theme::{FIELD_INK, INSECURE, ROW_STATUS, TYPING, TYPING_FIELD};

/// The strip's badge: its padding and its corner.
const BADGE_PADDING: f64 = 10.0;
const BADGE_RADIUS: f64 = 4.0;
const BADGE_INSET: f64 = 6.0;

impl Browser {
    /// Draws the cursor and the echo over the open page, in a face and an
    /// amber no page can draw (REQ-INT-134).
    pub(in crate::browser) fn draw_terminal<C: Canvas + ?Sized>(&self, canvas: &mut C, now: u64) {
        let Some(target) = self.typing_target() else { return };
        // Where the open page is drawn: zoomed, or on the wall.
        let shown = self.wall.shown().find(|(key, _)| *key == target).map(|(_, rect)| rect);
        let (Some(cursor), Some(page)) = (self.typing.typing.and_then(|typing| typing.cursor), shown) else { return };
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
        for (index, echo) in self.typing.echoes.iter().enumerate() {
            let rect = cell(usize::from(cursor.column).saturating_add(index));
            canvas.fill(rect, 0.0, TYPING_FIELD);
            let mut text = [0_u8; 4];
            left_centred(canvas, rect, 0.0, echo.character.encode_utf8(&mut text), style, TYPING);
        }
        let column = usize::from(cursor.column).saturating_add(self.typing.echoes.len()).min(usize::from(COLUMNS).saturating_sub(1));
        let visible = (now.saturating_sub(self.typing.moved) / BLINK_MILLISECONDS).is_multiple_of(2);
        if visible {
            let rect = cell(column);
            if cursor.flags & BLOCK != 0 {
                let line = cell_width * BLOCK_LINE_SHARE;
                crate::text::border(canvas, rect, line, TYPING);
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
    pub(in crate::browser) fn draw_typing_badge<C: Canvas + ?Sized>(&self, canvas: &mut C, area: Rect) -> Rect {
        let Some(typing) = self.typing.typing else { return area };
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
