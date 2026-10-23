//! The toolbar below the tab strip: Back, Forward, Reload and Home, the
//! address field with its security prefix, star and badges, the mute-all
//! button, the gear and the menu button.

use crate::browser::Browser;
use crate::browser::address::FIELD_PADDING;
use crate::browser::glyphs::{LOCK_WIDTH, arrow, gear, house, padlock, reload_glyph, speaker_glyph, star};
use crate::browser::window::MARGIN;
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{ADDRESS, BLACK, BUTTON, DIM, FIELD, FIELD_INK, INSECURE, PRIVATE, ROW_STATUS, RULE, SECURE, SELECTED, STAR_ON, WHITE};
use crate::ui::{HAIRLINE, border, left_centred};

use crate::browser::STRIP_HEIGHT;

pub(super) const TOOLBAR_TOP: f64 = STRIP_HEIGHT;
pub(super) const TOOLBAR_HEIGHT: f64 = 54.0;
const TOOL_SIZE: f64 = 44.0;
const TOOL_INSET: f64 = 5.0;
const LOCK_GAP: f64 = 12.0;
const MENU_BAR_WIDTH: f64 = 22.0;
const MENU_BAR_HEIGHT: f64 = 3.0;
const MENU_BAR_GAP: f64 = 7.0;

impl Browser {
    pub(super) fn draw_toolbar<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
        let bar = Rect::new(0.0, TOOLBAR_TOP, width, TOOLBAR_TOP + TOOLBAR_HEIGHT);
        canvas.fill(bar, 0.0, BLACK);
        canvas.fill(Rect::new(0.0, bar.y1 - HAIRLINE, width, bar.y1), 0.0, RULE);
        let top = bar.y0 + TOOL_INSET;
        let tool = |x: f64| Rect::new(x, top, x + TOOL_SIZE, top + TOOL_SIZE);
        self.regions.back = tool(MARGIN);
        self.regions.forward = tool(MARGIN + TOOL_SIZE);
        for (rect, enabled, pointing_left) in [(self.regions.back, self.strip.back, true), (self.regions.forward, self.strip.forward, false)] {
            canvas.fill(rect, 0.0, BUTTON);
            border(canvas, rect, 2.0, if enabled { SELECTED } else { RULE });
            arrow(canvas, rect, pointing_left, if enabled { WHITE } else { DIM });
        }
        // Reload, then Home after it (requirements 11.2 and 19.6).
        self.regions.reload = tool(MARGIN + TOOL_SIZE * 2.0);
        canvas.fill(self.regions.reload, 0.0, BUTTON);
        border(canvas, self.regions.reload, 2.0, RULE);
        reload_glyph(canvas, self.regions.reload, WHITE);
        self.regions.home = tool(MARGIN + TOOL_SIZE * 3.0);
        canvas.fill(self.regions.home, 0.0, BUTTON);
        border(canvas, self.regions.home, 2.0, RULE);
        house(canvas, self.regions.home, WHITE);
        self.regions.menu = tool(width - MARGIN - TOOL_SIZE);
        // Mute or unmute every tab, beside the menu.
        self.regions.mute_all = tool(self.regions.menu.x0 - MARGIN - TOOL_SIZE);
        canvas.fill(self.regions.mute_all, 0.0, BUTTON);
        border(canvas, self.regions.mute_all, 2.0, RULE);
        speaker_glyph(canvas, self.regions.mute_all, self.all_muted(), WHITE);
        // The settings panel's gear, at the right of the strip (requirement 19.3).
        self.regions.gear = tool(self.regions.mute_all.x0 - MARGIN - TOOL_SIZE);
        canvas.fill(self.regions.gear, 0.0, if self.settings_open() { SELECTED } else { BUTTON });
        border(canvas, self.regions.gear, 2.0, RULE);
        gear(canvas, self.regions.gear, WHITE);
        canvas.fill(self.regions.menu, 0.0, BUTTON);
        border(canvas, self.regions.menu, 2.0, RULE);
        let middle = f64::midpoint(self.regions.menu.y0, self.regions.menu.y1);
        let centre_x = f64::midpoint(self.regions.menu.x0, self.regions.menu.x1);
        for offset in [-MENU_BAR_GAP, 0.0, MENU_BAR_GAP] {
            let y = middle + offset - MENU_BAR_HEIGHT / 2.0;
            canvas.fill(Rect::new(centre_x - MENU_BAR_WIDTH / 2.0, y, centre_x + MENU_BAR_WIDTH / 2.0, y + MENU_BAR_HEIGHT), 0.0, WHITE);
        }

        let field = Rect::new(self.regions.home.x1 + MARGIN * 2.0, top, self.regions.gear.x0 - MARGIN * 2.0, top + TOOL_SIZE);
        self.regions.field = field;
        canvas.fill(field, 0.0, FIELD);
        if self.editing {
            border(canvas, field, 2.0, SELECTED);
        }
        // The strip's question takes the whole field: no address, no star.
        #[cfg(feature = "links")]
        if self.draw_prompt(canvas, field) {
            self.regions.star = Rect::ZERO;
            self.regions.field_text = Rect::ZERO;
            return;
        }
        let middle = f64::midpoint(field.y0, field.y1);
        let mut text_left = field.x0 + FIELD_PADDING;
        // A private tab says so first (requirement 20.8).
        if self.strip.private {
            let word = "Private";
            left_centred(canvas, Rect::new(text_left, field.y0, field.x1, field.y1), 0.0, word, ROW_STATUS, PRIVATE);
            text_left += canvas.text_width(word, ROW_STATUS) + LOCK_GAP;
        }
        if self.strip.built_in {
            let word = "NeoTeletext";
            left_centred(canvas, Rect::new(text_left, field.y0, field.x1, field.y1), 0.0, word, ROW_STATUS, FIELD_INK);
            text_left += canvas.text_width(word, ROW_STATUS) + LOCK_GAP;
        } else if self.strip.local {
            let word = "File";
            left_centred(canvas, Rect::new(text_left, field.y0, field.x1, field.y1), 0.0, word, ROW_STATUS, FIELD_INK);
            text_left += canvas.text_width(word, ROW_STATUS) + LOCK_GAP;
        } else if self.strip.refused.is_some() {
            let warning = "Certificate refused";
            left_centred(canvas, Rect::new(text_left, field.y0, field.x1, field.y1), 0.0, warning, ROW_STATUS, INSECURE);
            text_left += canvas.text_width(warning, ROW_STATUS) + LOCK_GAP;
        } else if self.strip.encrypted {
            padlock(canvas, Point::new(text_left, middle), SECURE);
            text_left += LOCK_WIDTH + LOCK_GAP;
        } else {
            let warning = "Not secure";
            left_centred(canvas, Rect::new(text_left, field.y0, field.x1, field.y1), 0.0, warning, ROW_STATUS, INSECURE);
            text_left += canvas.text_width(warning, ROW_STATUS) + LOCK_GAP;
        }
        self.regions.star = Rect::new(field.x1 - TOOL_SIZE, field.y0, field.x1, field.y1);
        star(canvas, Point::new(f64::midpoint(self.regions.star.x0, self.regions.star.x1), middle), if self.strip.bookmarked { STAR_ON } else { FIELD_INK });
        let text = Rect::new(text_left, field.y0, self.regions.star.x0, field.y1);
        #[cfg(feature = "phone-keyboard")]
        let text = self.draw_phone_badge(canvas, text);
        #[cfg(feature = "interaction")]
        let text = self.draw_typing_badge(canvas, text);
        self.regions.field_text = text;
        // While the pointer or the focus is on a link, the field shows where
        // it leads (requirement 17.6).
        #[cfg(feature = "links")]
        let pointing = !self.editing && self.draw_pointing(canvas, text);
        #[cfg(not(feature = "links"))]
        let pointing = false;
        // The trusted core's notice, for a while (requirements 19.3, 19.10).
        let noticed = !self.editing && !pointing && !self.strip.notice.is_empty();
        if noticed {
            canvas.push_clip(text);
            left_centred(canvas, text, 0.0, &self.strip.notice, ADDRESS, INSECURE);
            canvas.pop_clip();
        }
        if !pointing && !noticed {
            self.draw_address(canvas, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
    use crate::browser::{Key, Modifiers, Output, StripState};
    use crate::test_canvas::Recorder;

    /// Requirements 11.2 and 11.6 (decision 63): Back, Forward, Reload and
    /// Home in that order in one row; Reload by its button and by
    /// Command-R, Control-R away from macOS, which the interface takes on
    /// every platform as the command key.
    #[test]
    fn reload_sits_between_forward_and_home_and_has_its_keys() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let regions = &browser.regions;
        let row = [regions.back, regions.forward, regions.reload, regions.home];
        for pair in row.windows(2) {
            assert!(pair[1].x0 >= pair[0].x1 && (pair[1].y0 - pair[0].y0).abs() < f64::EPSILON, "{pair:?}");
        }
        let reload = browser.regions.reload;
        press_at(&mut browser, &mut canvas, reload);
        assert_eq!(browser.take_outputs(), [Output::Reload]);
        browser.key(Key::Text("r"), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Reload]);
        browser.key(Key::Text("r"), Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [], "r alone is no shortcut");
    }

    /// Requirement 19.6: the Home button after Forward, Command-Shift-H on
    /// macOS and Alt-Home elsewhere.
    #[test]
    fn home_is_asked_for_by_its_button_and_its_keys() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let (forward, home) = (browser.regions.forward, browser.regions.home);
        assert!(home.x0 >= forward.x1 && (home.y0 - forward.y0).abs() < f64::EPSILON, "after Forward, in its row");
        press_at(&mut browser, &mut canvas, home);
        assert_eq!(browser.take_outputs(), [Output::Home]);
        browser.key(Key::Text("H"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Home]);
        browser.key(Key::Home, Modifiers { alt: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Home]);
        // Command-H alone is not Home: it is History, Control-H's place away
        // from macOS (`history-and-bookmarks.md` section 6).
        browser.key(Key::Text("h"), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::History(crate::library::HistoryRequest::Panel(true))]);
    }

    /// Requirements 19.3 and 19.10: the trusted core's notice takes the
    /// address's place while it lasts; the start page is named as the
    /// browser's own, not "Not secure".
    #[test]
    fn the_strip_says_the_notice_and_names_the_start_page() {
        let (mut browser, _) = browser();
        browser.set_strip(StripState { notice: "The proxy refused the connection to example.invalid".into(), ..browser.strip.clone() });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        assert!(texts.contains(&"The proxy refused the connection to example.invalid"), "{texts:?}");
        assert!(!texts.contains(&"example.invalid"), "the notice in the address's place");
        browser.set_strip(StripState { address: "neoteletext:home".into(), identity: "NeoTeletext".into(), built_in: true, ..StripState::default() });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        assert!(texts.contains(&"NeoTeletext") && !texts.contains(&"Not secure"), "{texts:?}");
    }
}
