//! Tips: a short label that says what a control is and does, with its
//! shortcut, shown when the pointer rests on it, as every desktop
//! application shows one. Each frame lists every control drawn as an
//! icon, a symbol or text that may be cut short, with its tip; a press
//! puts the tip away until the pointer leaves the control. The tips are
//! the interface's own words, never a channel's text, except a tab's and
//! a bookmark's title, drawn as the text it is.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Browser, PAGES_PER_BANK, page_number};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{BUTTON, ROW_TITLE, RULE, WHITE};
use crate::ui::{border, left_centred};

/// How long the pointer rests on a control before its tip shows: about
/// what macOS and the common browsers wait.
const TIP_DELAY_MILLISECONDS: u64 = 500;
/// The tip's distance below the pointer, clear of the arrow's tip.
const BELOW_POINTER: f64 = 22.0;
const PADDING: f64 = 7.0;
const HEIGHT: f64 = 26.0;
const RADIUS: f64 = 4.0;
/// The tip keeps this far inside the window's edges.
const EDGE: f64 = 4.0;

/// A control's tip as last drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tip {
    pub(super) rect: Rect,
    pub(super) text: String,
}

/// The tips' state in the browser.
#[derive(Debug, Default)]
pub(super) struct TipsUi {
    /// Every control's tip in the last frame, in drawing order.
    drawn: Vec<Tip>,
    /// The control under the pointer, and since when.
    hovered: Option<(Tip, u64)>,
    shown: bool,
    /// A press put the tip away until the pointer leaves the control.
    dismissed: bool,
}

impl TipsUi {
    /// The tip of the control under `point`: the last drawn is on top.
    fn at(&self, point: Point) -> Option<&Tip> {
        self.drawn.iter().rev().find(|tip| tip.rect.contains(point))
    }

    /// The pointer is at `point` at `now`: whether a shown tip went away.
    fn moved(&mut self, point: Point, now: u64) -> bool {
        let under = self.at(point).cloned();
        if under.as_ref() == self.hovered.as_ref().map(|(tip, _)| tip) {
            return false;
        }
        let was_shown = self.shown;
        self.hovered = under.map(|tip| (tip, now));
        self.shown = false;
        self.dismissed = false;
        was_shown
    }
}

impl Browser {
    /// The shortcut on this platform: macOS's, or elsewhere's.
    pub(super) fn keys_named(&self, mac: &'static str, elsewhere: &'static str) -> &'static str {
        if self.frame.window_buttons { elsewhere } else { mac }
    }

    /// The pointer moved: whether a shown tip went away.
    pub(super) fn tips_moved(&mut self, now: u64) -> bool {
        self.tips.moved(self.pointer, now)
    }

    /// A press puts the tip away.
    pub(super) fn tips_pressed(&mut self) -> bool {
        let was_shown = self.tips.shown;
        self.tips.shown = false;
        self.tips.dismissed = self.tips.hovered.is_some();
        was_shown
    }

    /// How long until a tip shows, while the pointer rests on a control.
    pub(super) fn tips_deadline(&self, now: u64) -> Option<u64> {
        let (_, since) = self.tips.hovered.as_ref().filter(|_| !self.tips.shown && !self.tips.dismissed)?;
        Some(since.saturating_add(TIP_DELAY_MILLISECONDS).saturating_sub(now).max(1))
    }

    /// The tip shows once the pointer has rested; whether it did now.
    pub(super) fn tips_settle(&mut self, now: u64) -> bool {
        if self.tips_deadline(now).is_some_and(|_| self.tips.hovered.as_ref().is_some_and(|(_, since)| now.saturating_sub(*since) >= TIP_DELAY_MILLISECONDS)) {
            self.tips.shown = true;
            return true;
        }
        false
    }

    /// The tip shown now, if any.
    #[cfg(test)]
    pub(super) fn tip_shown(&self) -> Option<&str> {
        self.tips.hovered.as_ref().filter(|_| self.tips.shown).map(|(tip, _)| tip.text.as_str())
    }

    /// Every control's tip as this frame drew them, then the tip under the
    /// pointer, over everything.
    pub(super) fn draw_tips<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, height: f64) {
        let mut tips = Vec::new();
        // A list or menu open over the window takes the pointer; a panel
        // covers what is below the strip.
        if !self.popup_open() {
            self.strip_tips(&mut tips);
            self.bar_tips(&mut tips);
            self.remote_tips(&mut tips);
            self.tabs_tips(&mut tips);
            if self.settings_open() {
                self.settings_tips(&mut tips);
            } else if self.history_open() {
                self.history_tips(&mut tips);
            } else if self.bookmarks_open() {
                self.bookmarks_tips(&mut tips);
            } else if !self.star_open() {
                self.body_tips(&mut tips);
            }
        }
        self.tips.drawn = tips.into_iter().map(|(rect, text)| Tip { rect, text }).collect();
        // What a control's tip says may have changed under a still pointer.
        if let Some((hovered, _)) = &self.tips.hovered
            && self.tips.at(self.pointer) != Some(hovered)
        {
            self.tips.hovered = None;
            self.tips.shown = false;
        }
        let Some((tip, _)) = self.tips.hovered.as_ref().filter(|_| self.tips.shown) else { return };
        let text_width = canvas.text_width(&tip.text, ROW_TITLE);
        let box_width = (text_width + PADDING * 2.0).min(width - EDGE * 2.0);
        let x = (self.pointer.x - box_width / 2.0).clamp(EDGE, (width - EDGE - box_width).max(EDGE));
        // Below the pointer, or above it at the window's foot.
        let below = self.pointer.y + BELOW_POINTER;
        let y = if below + HEIGHT > height - EDGE { tip.rect.y0 - EDGE - HEIGHT } else { below };
        let area = Rect::new(x, y, x + box_width, y + HEIGHT);
        canvas.fill(area, RADIUS, BUTTON);
        border(canvas, area, 1.0, RULE);
        canvas.push_clip(area);
        left_centred(canvas, area, PADDING, &tip.text, ROW_TITLE, WHITE);
        canvas.pop_clip();
    }

    /// The tab strip's and the toolbar's controls.
    fn strip_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let regions = &self.regions;
        for region in &regions.tabs {
            if let Some(tab) = self.tabs.get(region.index) {
                let private = if tab.private { " (private tab)" } else { "" };
                tips.push((region.tab, format!("{}{private}", tab.title)));
                if let Some(speaker) = region.speaker {
                    tips.push((speaker, String::from(if tab.muted { "Unmute this tab" } else { "Mute this tab" })));
                }
            }
            let keys = self.keys_named("Cmd+W", "Ctrl+W");
            tips.push((region.close, format!("Close this tab ({keys}), or click the tab with the middle button")));
        }
        let fixed = [
            (regions.new_tab, "New tab", self.keys_named("Cmd+T", "Ctrl+T")),
            (regions.minimise, "Minimise", ""),
            (regions.maximise, "Maximise", ""),
            (regions.close, "Close the window", ""),
            (regions.reload, "Reload: open this channel afresh", self.keys_named("Cmd+R", "Ctrl+R")),
            (regions.home, "Home: open the home page", self.keys_named("Shift+Cmd+H", "Alt+Home")),
            (regions.field, "The address: type a channel's address or search", self.keys_named("Cmd+L", "Ctrl+L")),
            (regions.gear, "Settings", self.keys_named("Cmd+,", "Ctrl+,")),
            (regions.menu, "Menu: tabs, history, bookmarks, developer console", ""),
        ];
        for (rect, words, keys) in fixed {
            tips.push((rect, if keys.is_empty() { String::from(words) } else { format!("{words} ({keys})") }));
        }
        let back = format!("Back ({}); hold or right-click for the list", self.keys_named("Cmd+[", "Ctrl+["));
        let forward = format!("Forward ({}); hold or right-click for the list", self.keys_named("Cmd+]", "Ctrl+]"));
        tips.push((regions.back, back));
        tips.push((regions.forward, forward));
        let star = if self.strip.bookmarked { "Edit this channel's bookmark" } else { "Bookmark this channel" };
        tips.push((regions.star, format!("{star} ({})", self.keys_named("Cmd+D", "Ctrl+D"))));
        let mute = if self.all_muted() { "Unmute every tab" } else { "Mute every tab" };
        tips.push((regions.mute_all, String::from(mute)));
    }

    /// The window below the strip: the banks, the overview and the console.
    fn body_tips(&self, tips: &mut Vec<(Rect, String)>) {
        for (bank, rect) in self.regions.banks.iter().enumerate() {
            let first = page_number(bank * PAGES_PER_BANK);
            let last = page_number((bank + 1) * PAGES_PER_BANK - 1);
            if let (Some(first), Some(last)) = (first, last) {
                tips.push((*rect, format!("Show pages {first} to {last}")));
            }
        }
        let toggle = if self.overview_folded { "Open the overview of every page" } else { "Fold the overview away" };
        tips.push((self.regions.overview_toggle, String::from(toggle)));
        let (smaller, larger) = self.wall.size_buttons();
        tips.push((smaller, String::from("Smaller pages: more on the wall")));
        tips.push((larger, String::from("Larger pages: fewer on the wall")));
        self.console_tips(tips);
        self.controls_tips(tips);
        #[cfg(feature = "phone-keyboard")]
        self.phone_tips(tips);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Output};
    use crate::test_canvas::Recorder;

    fn draw(browser: &mut Browser, now: u64) -> Recorder {
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, now);
        canvas
    }

    fn rest_on(browser: &mut Browser, rect: Rect, now: u64) {
        browser.pointer_moved(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1), now);
    }

    /// A tip shows once the pointer has rested on a control, with the
    /// platform's shortcut; moving off or pressing puts it away.
    #[test]
    fn a_tip_shows_after_a_rest_and_goes_on_a_press() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        draw(&mut browser, 0);
        let reload = browser.regions.reload;
        rest_on(&mut browser, reload, 1_000);
        assert_eq!(browser.tips_deadline(1_000), Some(TIP_DELAY_MILLISECONDS));
        assert!(!browser.tips_settle(1_200) && browser.tip_shown().is_none(), "not yet");
        assert!(browser.tips_settle(1_500));
        assert_eq!(browser.tip_shown(), Some("Reload: open this channel afresh (Cmd+R)"));
        let canvas = draw(&mut browser, 1_500);
        assert!(canvas.texts.iter().any(|(_, text)| text == "Reload: open this channel afresh (Cmd+R)"), "drawn");
        browser.pointer_pressed(&mut Recorder::new(), false, 1_600);
        assert_eq!(browser.take_outputs(), [Output::Reload]);
        assert!(browser.tip_shown().is_none() && browser.tips_deadline(2_000).is_none(), "away until the pointer leaves");
        let home = browser.regions.home;
        rest_on(&mut browser, home, 2_000);
        assert!(browser.tips_settle(2_500));
        assert_eq!(browser.tip_shown(), Some("Home: open the home page (Shift+Cmd+H)"));
        assert!(browser.pointer_moved(-1.0, -1.0, 2_600), "leaving the window takes it away");
        assert!(browser.tip_shown().is_none());
    }

    /// Elsewhere than macOS, the shortcuts are Control's.
    #[test]
    fn tips_name_each_platforms_shortcut() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
        draw(&mut browser, 0);
        let texts: Vec<String> = browser.tips.drawn.iter().map(|tip| tip.text.clone()).collect();
        for expected in ["Reload: open this channel afresh (Ctrl+R)", "Home: open the home page (Alt+Home)", "Settings (Ctrl+,)", "Minimise"] {
            assert!(texts.iter().any(|text| text == expected), "{expected} in {texts:?}");
        }
    }
}
