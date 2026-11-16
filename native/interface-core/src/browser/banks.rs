//! The page banks panel at the left: one entry per bank of a hundred
//! pages, with its colour, its range, its name and its page count.

use alloc::format;
use alloc::string::String;

use crate::browser::Browser;
use crate::browser::window::{HEADING_HEIGHT, PADDING};
use crate::canvas::Canvas;
use crate::channel::BANKS;
use crate::geometry::{Point, Rect};
use crate::surface::Surface;
use crate::text::{HAIRLINE, border, fitted_text, label, left_centred, outline, right_centred};
use crate::theme::{BANK_COLOURS, BANK_COUNT, BANK_NAME, BANK_RANGE, BUTTON, HEADING, RAISED};

const BANK_GAP: f64 = 6.0;
const BANK_MAX_HEIGHT: f64 = 60.0;
/// Each bank's colour, as a thin bar down the left of its entry.
const SWATCH_WIDTH: f64 = 6.0;
const SWATCH_INSET: f64 = 8.0;
pub(super) const BANK_BORDER: f64 = 3.0;
const BANK_RANGE_RISE: f64 = 2.0;

/// The least height of a magazine's entry: its range over its name, with
/// the swatch's inset above and below.
fn entry_content_height<C: Canvas + ?Sized>(canvas: &mut C) -> f64 {
    canvas.line_height(BANK_RANGE) + canvas.line_height(BANK_NAME) + 2.0 * SWATCH_INSET
}

impl Browser {
    pub(super) fn draw_banks<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, surface: Surface) {
        canvas.fill(panel, 0.0, surface.back);
        outline(canvas, panel, surface.rule);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT);
        left_centred(canvas, heading, PADDING, "Channel magazines", HEADING, surface.ink);
        canvas.fill(Rect::new(panel.x0, heading.y1 - HAIRLINE, panel.x1, heading.y1), 0.0, surface.rule);
        // Room is kept under eight entries for the bytes downloaded, so
        // that they show whatever the window's height (REQ-6352).
        let downloaded_room = BANK_GAP + canvas.line_height(BANK_NAME) + canvas.line_height(BANK_RANGE);
        // An entry is never smaller than what it holds, its range over its
        // name with the swatch's inset above and below: in a short window
        // the last entries are left out, and none is drawn squeezed.
        let content_height = entry_content_height(canvas);
        #[allow(clippy::cast_precision_loss, reason = "eight banks")]
        let height = ((panel.height() - HEADING_HEIGHT - BANK_GAP - downloaded_room) / BANKS as f64 - BANK_GAP).min(BANK_MAX_HEIGHT).max(content_height);
        // Only a magazine with pages has an entry, and the entries stand
        // one under another with no gap for the others (REQ-6350).
        let mut top = heading.y1 + BANK_GAP;
        for (bank, &colour) in BANK_COLOURS.iter().enumerate() {
            if self.channel.bank_count(bank) == 0 {
                // The regions stay one for each magazine, in order: an
                // empty one is under no pointer.
                self.regions.banks.push(Rect::ZERO);
                continue;
            }
            let rect = Rect::new(panel.x0 + BANK_GAP, top, panel.x1 - BANK_GAP, top + height);
            if rect.y1 > panel.y1 - BANK_GAP {
                // No room for it whole: it has no entry, and is under no
                // pointer.
                self.regions.banks.push(Rect::ZERO);
                continue;
            }
            let chosen = bank == self.bank;
            let hovered = self.hovered_bank == Some(bank);
            canvas.fill(
                rect,
                0.0,
                if chosen {
                    BUTTON
                } else if hovered {
                    RAISED
                } else {
                    surface.back
                },
            );
            if chosen {
                border(canvas, rect, BANK_BORDER, colour);
            } else {
                outline(canvas, rect, surface.rule_soft);
            }
            if hovered && !chosen {
                surface.edge(canvas, rect, RAISED);
            }
            // A magazine with a fill of its own reads as it does on black;
            // the others stand on the surface.
            let on = if chosen || hovered { Surface::BROWSER } else { surface };
            let colour = on.mark(colour);
            let middle = f64::midpoint(rect.y0, rect.y1);
            let swatch = Rect::new(rect.x0 + PADDING, rect.y0 + SWATCH_INSET, rect.x0 + PADDING + SWATCH_WIDTH, rect.y1 - SWATCH_INSET);
            canvas.fill(swatch, 0.0, colour);
            let text_left = swatch.x1 + PADDING;
            let magazine = bank.saturating_add(1);
            let range = format!("{magazine}00-{magazine}99");
            let name = self.channel.banks.get(bank).map_or("", String::as_str);
            let range_height = canvas.line_height(BANK_RANGE);
            let name_height = canvas.line_height(BANK_NAME);
            let text_top = middle - f64::midpoint(range_height, name_height) - BANK_RANGE_RISE;
            label(canvas, Point::new(text_left, text_top), &range, BANK_RANGE, colour);
            let name_line = Rect::new(text_left, text_top + range_height, rect.x1 - PADDING, text_top + range_height + name_height);
            if fitted_text(canvas, name_line, name, BANK_NAME, colour) {
                self.regions.cut_banks.push((bank, String::from(name)));
            }
            right_centred(
                canvas,
                Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + range_height + PADDING),
                PADDING,
                &format!("{}", self.channel.bank_count(bank)),
                BANK_COUNT,
                on.muted,
            );
            self.regions.banks.push(rect);
            top = rect.y1 + BANK_GAP;
        }
        self.draw_downloaded(canvas, Rect::new(panel.x0 + BANK_GAP + PADDING, top + BANK_GAP, panel.x1 - BANK_GAP, panel.y1), surface);
    }

    /// Under the magazines, in `room`: the bytes downloaded for the page
    /// shown, or for the sub-page shown when the page has sub-pages, one
    /// figure and never both, as the trusted core counts them (REQ-6352).
    fn draw_downloaded<C: Canvas + ?Sized>(&self, canvas: &mut C, room: Rect, surface: Surface) {
        let controls = self.page_controls();
        if controls.page.is_none() {
            return;
        }
        let caption = if controls.subpages > 1 { "Sub-page downloaded" } else { "Page downloaded" };
        let (caption_height, figure_height) = (canvas.line_height(BANK_NAME), canvas.line_height(BANK_RANGE));
        if room.y0 + caption_height + figure_height > room.y1 {
            return;
        }
        label(canvas, Point::new(room.x0, room.y0), caption, BANK_NAME, surface.muted);
        let figure = format!("{} bytes", crate::browser::regis_status::grouped(controls.downloaded));
        fitted_text(canvas, Rect::new(room.x0, room.y0 + caption_height, room.x1, room.y0 + caption_height + figure_height), &figure, BANK_RANGE, surface.ink);
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

    /// REQ-6350: a magazine without pages has no entry, the others close
    /// up, each stays its own magazine's, and the keys pass over it.
    #[test]
    fn a_magazine_without_pages_is_not_shown() {
        use crate::browser::{Key, Modifiers};
        use crate::channel::bank_indices;
        use crate::geometry::Rect;
        let (mut browser, mut canvas) = browser();
        let all = browser.regions.banks.clone();
        for bank in [1, 3] {
            for index in bank_indices(bank) {
                browser.channel.pages[index] = None;
            }
        }
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let shown = browser.regions.banks.clone();
        assert_eq!(shown.len(), all.len(), "one region for each magazine still");
        assert_eq!((shown[1], shown[3]), (Rect::ZERO, Rect::ZERO), "the magazines without pages have no entry");
        assert_eq!([shown[0], shown[2], shown[4]], [all[0], all[1], all[2]], "the others close up from the top");
        press_at(&mut browser, &mut canvas, shown[2]);
        assert_eq!(browser.bank, 2, "the second entry drawn is the third magazine's");
        browser.key(Key::Down, Modifiers::default(), 0);
        assert_eq!(browser.bank, 4, "Down passes over the magazine without pages");
        browser.key(Key::Up, Modifiers::default(), 0);
        browser.key(Key::Up, Modifiers::default(), 0);
        assert_eq!(browser.bank, 0, "and Up too");
    }

    /// REQ-6352: under the magazines, the bytes of the page shown, or of
    /// the sub-page shown on a page with sub-pages; nothing with none open.
    #[test]
    fn the_bytes_downloaded_are_shown_under_the_magazines() {
        use crate::controls::PageControls;
        let (mut browser, mut canvas) = browser();
        let said = |canvas: &crate::test_canvas::Recorder, text: &str| canvas.texts.iter().any(|(_, drawn)| drawn == text);
        let drawn = |browser: &mut crate::browser::Browser, canvas: &mut crate::test_canvas::Recorder, controls: PageControls| {
            browser.set_page_controls(controls);
            canvas.texts.clear();
            browser.draw(canvas, WIDTH, HEIGHT, 0);
        };
        drawn(&mut browser, &mut canvas, PageControls::default());
        assert!(!said(&canvas, "Page downloaded") && !said(&canvas, "Sub-page downloaded"), "no page is open");
        drawn(&mut browser, &mut canvas, PageControls { channel: 7, page: Some(0x100), subpages: 1, downloaded: 1_234_567, ..PageControls::default() });
        assert!(said(&canvas, "Page downloaded") && said(&canvas, "1,234,567 bytes"), "{:?}", canvas.texts);
        assert!(!said(&canvas, "Sub-page downloaded"), "one figure, the page's");
        drawn(
            &mut browser,
            &mut canvas,
            PageControls { channel: 7, page: Some(0x100), subpages: 10, chosen: Some(2), downloaded: 890, ..PageControls::default() },
        );
        assert!(said(&canvas, "Sub-page downloaded") && said(&canvas, "890 bytes"), "{:?}", canvas.texts);
        assert!(!said(&canvas, "Page downloaded"), "one figure, the sub-page's");
    }

    /// The owner, 16 November 2026: "it should not be possible for the box
    /// to be smaller than its contents". In a window too short for every
    /// magazine, each entry drawn is still as tall as what it holds, and
    /// the ones with no room are left out.
    #[test]
    fn a_magazines_entry_is_never_smaller_than_what_it_holds() {
        use crate::geometry::Rect;
        /// A window too short for eight entries of their content's height.
        const SHORT: f64 = 420.0;
        let (mut browser, mut canvas) = browser();
        let least = super::entry_content_height(&mut canvas);
        browser.draw(&mut canvas, WIDTH, SHORT, 0);
        let drawn: alloc::vec::Vec<Rect> = browser.regions.banks.iter().copied().filter(|rect| *rect != Rect::ZERO).collect();
        assert!(!drawn.is_empty(), "the first magazines still have entries");
        assert!(drawn.len() < browser.regions.banks.len(), "and the last have none: there is no room for them whole");
        for rect in &drawn {
            assert!(rect.height() >= least, "an entry of {} holds content of {least}", rect.height());
            assert!(rect.y1 <= SHORT, "and lies within the window");
        }
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
