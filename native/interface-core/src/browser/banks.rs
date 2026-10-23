//! The page banks panel at the left: one entry per bank of a hundred
//! pages, with its colour, its range, its name and its page count.

use alloc::format;
use alloc::string::String;

use crate::browser::Browser;
use crate::browser::window::{HEADING_HEIGHT, PADDING};
use crate::canvas::Canvas;
use crate::channel::BANKS;
use crate::geometry::{Point, Rect};
use crate::text::{HAIRLINE, border, label, left_centred, outline, right_centred};
use crate::theme::{BANK_COLOURS, BANK_COUNT, BANK_NAME, BANK_RANGE, BUTTON, HEADING, MUTED, PANEL, RULE, RULE_SOFT, WHITE};

const BANK_GAP: f64 = 6.0;
const BANK_MAX_HEIGHT: f64 = 60.0;
/// Each bank's colour, as a thin bar down the left of its entry.
const SWATCH_WIDTH: f64 = 6.0;
const SWATCH_INSET: f64 = 8.0;
pub(super) const BANK_BORDER: f64 = 3.0;
const BANK_RANGE_RISE: f64 = 2.0;

impl Browser {
    pub(super) fn draw_banks<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect) {
        canvas.fill(panel, 0.0, PANEL);
        outline(canvas, panel, RULE);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT);
        left_centred(canvas, heading, PADDING, "Page banks", HEADING, WHITE);
        canvas.fill(Rect::new(panel.x0, heading.y1 - HAIRLINE, panel.x1, heading.y1), 0.0, RULE);
        #[allow(clippy::cast_precision_loss, reason = "eight banks")]
        let height = ((panel.height() - HEADING_HEIGHT - BANK_GAP) / BANKS as f64 - BANK_GAP).min(BANK_MAX_HEIGHT);
        for (bank, &colour) in BANK_COLOURS.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "eight banks")]
            let top = heading.y1 + BANK_GAP + bank as f64 * (height + BANK_GAP);
            let rect = Rect::new(panel.x0 + BANK_GAP, top, panel.x1 - BANK_GAP, top + height);
            let chosen = bank == self.bank;
            canvas.fill(rect, 0.0, if chosen { BUTTON } else { PANEL });
            if chosen {
                border(canvas, rect, BANK_BORDER, colour);
            } else {
                outline(canvas, rect, RULE_SOFT);
            }
            let middle = f64::midpoint(rect.y0, rect.y1);
            let swatch = Rect::new(rect.x0 + PADDING, rect.y0 + SWATCH_INSET, rect.x0 + PADDING + SWATCH_WIDTH, rect.y1 - SWATCH_INSET);
            canvas.fill(swatch, 0.0, colour);
            let text_left = swatch.x1 + PADDING;
            let range = format!("{}00-{}99", bank + 1, bank + 1);
            let name = self.channel.banks.get(bank).map_or("", String::as_str);
            let range_height = canvas.line_height(BANK_RANGE);
            let name_height = canvas.line_height(BANK_NAME);
            let top = middle - f64::midpoint(range_height, name_height) - BANK_RANGE_RISE;
            label(canvas, Point::new(text_left, top), &range, BANK_RANGE, colour);
            canvas.text(Rect::new(text_left, top + range_height, rect.x1 - PADDING, top + range_height + name_height), name, BANK_NAME, colour);
            right_centred(
                canvas,
                Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + range_height + PADDING),
                PADDING,
                &format!("{}", self.channel.bank_count(bank)),
                BANK_COUNT,
                MUTED,
            );
            self.regions.banks.push(rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::browser::Output;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};

    #[test]
    fn the_first_bank_is_shown_and_reported_visible() {
        let (mut browser, _) = browser();
        let outputs = browser.take_outputs();
        let Some(Output::Visible(visible)) = outputs.first() else { panic!("{outputs:?}") };
        assert!(visible.iter().all(|key| key.channel == 7 && (0x100..0x200).contains(&key.page)));
        assert_eq!(visible[0].page, 0x100);
    }

    #[test]
    fn choosing_a_bank_shows_its_pages() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let bank = browser.regions.banks[4];
        press_at(&mut browser, &mut canvas, bank);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let outputs = browser.take_outputs();
        let Some(Output::Visible(visible)) = outputs.first() else { panic!("{outputs:?}") };
        assert!(visible.iter().all(|key| (0x500..0x600).contains(&key.page)), "{visible:?}");
    }
}
