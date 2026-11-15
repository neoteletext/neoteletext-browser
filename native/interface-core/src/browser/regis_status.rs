//! What the ReGIS renderer's picture of the open page cost, drawn above
//! the picture (DEC-6335 item 4): the count of commits applied and the
//! work units the last repaint of its numbered drawings took. The owner
//! asked for the work units to be shown before any rule limits them.

use alloc::format;
use alloc::string::String;

use crate::browser::Browser;
use crate::canvas::{Canvas, PageKey, ROWS, Style};
use crate::geometry::Rect;
use crate::theme::GREY;

/// The text against a row's height.
const TEXT_SHARE: f64 = 0.6;
/// Room before the text, against a row's height.
const INSET_SHARE: f64 = 0.3;
/// Digits between the separators of a long number.
const GROUP: usize = 3;

/// A number with a comma between each three digits.
fn grouped(number: u64) -> String {
    let digits = format!("{number}");
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && digits.len().saturating_sub(index).is_multiple_of(GROUP) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

impl Browser {
    /// The trusted core sent the picture of a page drawn in ReGIS: what it
    /// cost replaces what the last one did.
    pub fn set_regis(&mut self, key: PageKey, commits: u32, work: u64) {
        self.regis.insert(key, (commits, work));
        // Only the pages of channels still shown are kept.
        let wall = &self.wall;
        self.regis.retain(|held, _| *held == key || wall.shown().any(|(shown, _)| shown.channel == held.channel));
    }

    /// The cost of the open page's picture, on the image's top row, which
    /// lies above the picture.
    pub(super) fn draw_regis_status<C: Canvas + ?Sized>(&self, canvas: &mut C) {
        let Some((key, page)) = self.wall.open() else { return };
        let Some((commits, work)) = self.regis.get(&key) else { return };
        let height = page.height() / f64::from(ROWS);
        let row = Rect::new(page.x0, page.y0, page.x1, page.y0 + height);
        let text = format!("ReGIS  commit {}  {} work units", grouped(u64::from(*commits)), grouped(*work));
        #[allow(clippy::cast_possible_truncation, reason = "a row's height in logical pixels, far inside f32")]
        let style = Style { size: (height * TEXT_SHARE) as f32, monospace: true, bold: false, caps: false };
        let inset = height * INSET_SHARE;
        canvas.push_clip(row);
        canvas.text(Rect::new(row.x0 + inset, row.y0, row.x1, row.y1), &text, style, GREY);
        canvas.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::grouped;

    #[test]
    fn long_numbers_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(100_000_000), "100,000,000");
        assert_eq!(grouped(12_345_678), "12,345,678");
    }
}
