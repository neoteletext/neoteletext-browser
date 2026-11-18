//! "Remember me on this channel" in the trusted strip (neoteletext-common's
//! `docs/viewer-state.md` VS-109, VS-114 and VS-208, open question Q4): the
//! strip's mark that the active tab's channel knows this browser, which
//! opens the list of channels that know you; and the strip's question,
//! asked when the viewer chooses "Remember me here…" from the menu or when
//! the channel offers, that names the channel and says what remembering
//! means, and is answered only by the viewer's press. The channel cannot
//! draw, pre-tick or hide it: it is the browser's own strip.

use alloc::format;

use super::{Browser, Output};
use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::text::{border, centred, left_centred};
use crate::theme::{ADDRESS, BUTTON, LINK_INK, MUTED, ROW_STATUS, ROW_TITLE, RULE, WHITE};
use crate::viewer_state::{Known, KnownRequest};

/// The question's buttons: their inset in the field, width and gap, as the
/// links' question has them.
const BUTTON_INSET: f64 = 3.0;
const BUTTON_WIDTH: f64 = 92.0;
const BUTTON_GAP: f64 = 6.0;
const FIELD_PADDING: f64 = 8.0;
const MARK_GAP: f64 = 8.0;
/// The most days a remembered key is kept (VS-107, open question Q2), as
/// the question says it: the trusted core's limit, which it asserts.
pub const REMEMBERED_DAYS: u32 = 400;

/// The strip's part of viewer state.
#[derive(Debug, Default)]
pub(super) struct RememberUi {
    /// The viewer chose "Remember me here…" from the menu: the strip asks.
    asked: bool,
    remember: Rect,
    not_now: Rect,
    mark: Rect,
}

impl Browser {
    /// Whether the strip asks to remember the viewer: chosen from the menu,
    /// or offered by the channel, on a channel that may be remembered.
    fn remember_asking(&self) -> bool {
        self.strip.can_remember && self.strip.known == Known::Visit && (self.remember.asked || self.strip.keep_offer)
    }

    /// The menu's item for the active tab's channel (VS-109): "Remember me
    /// here…" while it may be remembered, "Stop remembering me here" while
    /// it is; none otherwise.
    pub(super) fn remember_menu_words(&self) -> Option<&'static str> {
        match self.strip.known {
            Known::Remembered => Some("Stop remembering me here"),
            Known::Visit if self.strip.can_remember => Some("Remember me here\u{2026}"),
            Known::Visit | Known::Not => None,
        }
    }

    /// The menu's item chosen: the strip asks, or remembering stops at once,
    /// which needs no question: forgetting is as easy as remembering.
    pub(super) fn remember_chosen(&mut self) {
        if self.strip.known == Known::Remembered {
            self.outputs.push(Output::Known(KnownRequest::RememberActive(false)));
        } else {
            self.remember.asked = true;
        }
    }

    /// A press on the strip's question or its mark. Returns whether it was
    /// on either.
    pub(super) fn remember_press(&mut self) -> bool {
        if self.remember_asking() {
            if self.remember.remember.contains(self.pointer) {
                self.remember.asked = false;
                self.outputs.push(Output::Known(KnownRequest::RememberActive(true)));
                return true;
            }
            if self.remember.not_now.contains(self.pointer) {
                self.remember.asked = false;
                if self.strip.keep_offer {
                    self.outputs.push(Output::Known(KnownRequest::DismissOffer));
                }
                return true;
            }
            return self.regions.field.contains(self.pointer);
        }
        if self.remember.mark.contains(self.pointer) {
            self.manage_known();
            return true;
        }
        false
    }

    /// The tab changed: a question the viewer asked for from the menu was
    /// for the channel it named, and goes.
    pub(super) fn remember_tab_changed(&mut self) {
        self.remember.asked = false;
    }

    /// The strip's question in the address `field`, when it asks. Returns
    /// whether it took the field.
    pub(super) fn draw_remember_prompt<C: Canvas + ?Sized>(&mut self, canvas: &mut C, field: Rect) -> bool {
        if !self.remember_asking() {
            self.remember.remember = Rect::ZERO;
            self.remember.not_now = Rect::ZERO;
            return false;
        }
        let question = format!(
            "Remember you on {} on later visits, until you forget it or for {REMEMBERED_DAYS} days at most?",
            if self.strip.address.is_empty() { "this channel" } else { self.strip.address.as_str() }
        );
        let top = field.y0 + BUTTON_INSET;
        let bottom = field.y1 - BUTTON_INSET;
        let not_now = Rect::new(field.x1 - BUTTON_INSET - BUTTON_WIDTH, top, field.x1 - BUTTON_INSET, bottom);
        let remember = Rect::new(not_now.x0 - BUTTON_GAP - BUTTON_WIDTH, top, not_now.x0 - BUTTON_GAP, bottom);
        let left = field.x0 + FIELD_PADDING;
        canvas.push_clip(Rect::new(left, field.y0, remember.x0 - BUTTON_GAP, field.y1));
        left_centred(canvas, Rect::new(left, field.y0, remember.x0, field.y1), 0.0, &question, ADDRESS, LINK_INK);
        canvas.pop_clip();
        for (rect, label) in [(remember, "Remember"), (not_now, "Not now")] {
            canvas.fill(rect, 0.0, BUTTON);
            centred(canvas, rect, label, ROW_TITLE, WHITE);
            border(canvas, rect, 1.0, RULE);
        }
        self.remember.remember = remember;
        self.remember.not_now = not_now;
        true
    }

    /// The strip's mark that the channel knows this browser (VS-114), at
    /// `left` in the address `field`. Returns where the address starts.
    pub(super) fn draw_known_mark<C: Canvas + ?Sized>(&mut self, canvas: &mut C, field: Rect, left: f64) -> f64 {
        let word = match self.strip.known {
            Known::Not => {
                self.remember.mark = Rect::ZERO;
                return left;
            }
            Known::Visit => "Known",
            Known::Remembered => "Remembered",
        };
        let width = canvas.text_width(word, ROW_STATUS);
        self.remember.mark = Rect::new(left, field.y0, left + width, field.y1);
        left_centred(canvas, self.remember.mark, 0.0, word, ROW_STATUS, MUTED);
        left + width + MARK_GAP
    }

    /// The mark's tip.
    pub(super) fn remember_tips(&self, tips: &mut alloc::vec::Vec<(Rect, alloc::string::String)>) {
        let words = match self.strip.known {
            Known::Not => return,
            Known::Visit => "This channel knows this browser until it closes: see the channels that know you",
            Known::Remembered => "This channel remembers this browser: see the channels that know you",
        };
        tips.push((self.remember.mark, words.into()));
    }
}

#[cfg(test)]
mod tests {
    use crate::browser::fixture::{blank_browser, middle};
    use crate::browser::{Output, StripState};
    use crate::test_canvas::Recorder;
    use crate::viewer_state::{Known, KnownRequest};

    fn known(known: Known, can_remember: bool, keep_offer: bool) -> crate::browser::Browser {
        let mut browser = blank_browser();
        browser.set_strip(StripState { address: "https://news.example/teletext/".into(), known, can_remember, keep_offer, ..StripState::default() });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.take_outputs();
        browser
    }

    fn press(browser: &mut crate::browser::Browser, rect: crate::geometry::Rect) -> bool {
        let point = middle(rect);
        browser.pointer_moved(point.x, point.y, 0);
        browser.remember_press()
    }

    /// VS-109: the menu's choice asks in the strip, naming the channel;
    /// only the viewer's press on Remember asks the trusted core; Not now
    /// asks nothing. A channel that is remembered is forgotten at once.
    #[test]
    fn remembering_is_asked_and_answered_only_by_the_viewer() {
        let mut browser = known(Known::Visit, true, false);
        assert_eq!(browser.remember_menu_words(), Some("Remember me here\u{2026}"));
        browser.remember_chosen();
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        assert!(canvas.texts.iter().any(|(_, text)| text.contains("https://news.example/teletext/") && text.contains("400 days")), "the channel named");
        let not_now = browser.remember.not_now;
        assert!(press(&mut browser, not_now));
        assert_eq!(browser.take_outputs(), []);
        browser.remember_chosen();
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let remember = browser.remember.remember;
        assert!(press(&mut browser, remember));
        assert_eq!(browser.take_outputs(), [Output::Known(KnownRequest::RememberActive(true))]);
        let mut remembered = known(Known::Remembered, false, false);
        assert_eq!(remembered.remember_menu_words(), Some("Stop remembering me here"));
        remembered.remember_chosen();
        assert_eq!(remembered.take_outputs(), [Output::Known(KnownRequest::RememberActive(false))]);
        assert_eq!(known(Known::Visit, false, false).remember_menu_words(), None, "not offered where it cannot be kept");
        assert_eq!(known(Known::Not, true, false).remember_menu_words(), None, "nothing to remember");
    }

    /// VS-208 and Q4: the channel's offer is the strip's question, which
    /// the viewer may dismiss; it never asks where it cannot be kept.
    #[test]
    fn the_channels_offer_is_a_question_the_viewer_may_dismiss() {
        let mut browser = known(Known::Visit, true, true);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let not_now = browser.remember.not_now;
        assert!(press(&mut browser, not_now));
        assert_eq!(browser.take_outputs(), [Output::Known(KnownRequest::DismissOffer)]);
        let mut private = known(Known::Visit, false, true);
        assert!(!private.draw_remember_prompt(&mut Recorder::new(), crate::geometry::Rect::new(0.0, 0.0, 800.0, 30.0)), "no question");
    }

    /// VS-114: the strip's mark says the channel knows this browser, and
    /// opens the list of channels that know you.
    #[test]
    fn the_mark_says_known_and_opens_the_list() {
        let mut browser = known(Known::Remembered, false, false);
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        assert!(canvas.texts.iter().any(|(_, text)| text == "Remembered"));
        let mark = browser.remember.mark;
        assert!(press(&mut browser, mark));
        assert_eq!(browser.take_outputs(), [Output::Known(KnownRequest::Panel(true))]);
        assert!(browser.known_open() && browser.settings_open());
    }
}
