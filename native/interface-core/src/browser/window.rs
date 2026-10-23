//! The window's frame: where the banks, the list, the page area and the
//! overview lie below the strip, and the one `draw` that draws every part
//! in its order, the panels and drop-downs over the rest.

use alloc::vec::Vec;

use crate::browser::console_panel::CONSOLE_HEIGHT;
use crate::browser::star_ui::BAR_HEIGHT;
use crate::browser::toolbar::{TOOLBAR_HEIGHT, TOOLBAR_TOP};
use crate::browser::{Browser, Output};
use crate::canvas::{Canvas, PageKey};
use crate::channel::{PAGES_PER_BANK, page_number};
use crate::geometry::Rect;
use crate::theme::WINDOW;
use crate::wall::WallPage;

pub(super) const BODY_TOP: f64 = TOOLBAR_TOP + TOOLBAR_HEIGHT + MARGIN;
pub(super) const MARGIN: f64 = 8.0;
const BANKS_WIDTH: f64 = 212.0;
const LIST_WIDTH: f64 = 260.0;
pub(super) const OVERVIEW_HEIGHT: f64 = 176.0;
pub(super) const HEADING_HEIGHT: f64 = 40.0;
pub(super) const PADDING: f64 = 12.0;

impl Browser {
    /// Draws the whole window; `width` and `height` are logical pixels.
    pub fn draw<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, height: f64, now: u64) {
        self.regions.clear();
        canvas.fill(Rect::new(0.0, 0.0, width, height), 0.0, WINDOW);
        // The bookmarks bar, when shown, is a row of the strip above the body.
        let body_top = BODY_TOP + if self.bar_shown() { BAR_HEIGHT } else { 0.0 };
        // The docked remote narrows the content (decision 68).
        let right = width - MARGIN - self.remote_docked_width(now);
        self.body = Rect::new(MARGIN, body_top, right, height - MARGIN);
        // The console docks where the overview is.
        let overview_height = if self.console.open() {
            CONSOLE_HEIGHT
        } else if self.overview_folded {
            HEADING_HEIGHT
        } else {
            OVERVIEW_HEIGHT
        };
        let overview = Rect::new(MARGIN, height - MARGIN - overview_height, right, height - MARGIN);
        let body_bottom = overview.y0 - MARGIN;
        let banks = Rect::new(MARGIN, body_top, MARGIN + BANKS_WIDTH, body_bottom);
        let list = Rect::new(banks.x1 + MARGIN, body_top, banks.x1 + MARGIN + LIST_WIDTH, body_bottom);
        let zoom = Rect::new(list.x1 + MARGIN, body_top, right, body_bottom);

        // The titles are borrowed from the channel while the wall draws:
        // the fields are borrowed apart.
        let first = self.bank * PAGES_PER_BANK;
        let (channel, updates) = (&self.channel, &self.updates);
        let pages: Vec<WallPage<'_>> = (first..first + PAGES_PER_BANK)
            .filter_map(|index| {
                let info = channel.pages.get(index)?.as_ref()?;
                let key = PageKey { channel: channel.channel, page: page_number(index)? };
                let updated_at = updates.iter().find(|&&(which, _)| which == index).map(|&(_, at)| at);
                Some(WallPage { key, title: &info.title, updated: updated_at })
            })
            .collect();
        let selected = self.selected.and_then(|index| self.page_key(index));
        self.wall.draw(canvas, zoom, &pages, selected, now);
        drop(pages);
        #[cfg(feature = "links")]
        self.draw_links(canvas);
        #[cfg(feature = "interaction")]
        self.draw_terminal(canvas, now);
        // A refused certificate: its screen in place of the pages.
        self.draw_refusal(canvas, zoom);
        let mut visible = self.wall.visible();
        // The overview's previewed page is on screen too, so the trusted
        // core has it rendered, whichever bank it is in.
        if let Some(key) = self.previewed.and_then(|index| self.page_key(index))
            && !visible.contains(&key)
        {
            visible.push(key);
        }
        if visible != self.visible {
            self.visible.clone_from(&visible);
            self.outputs.push(Output::Visible(visible));
        }
        self.draw_banks(canvas, banks);
        self.draw_list(canvas, list, now);
        if self.console.open() {
            self.draw_console(canvas, overview);
        } else {
            self.draw_overview(canvas, overview, now);
        }
        self.draw_toolbar(canvas, width);
        self.draw_bar(canvas, TOOLBAR_TOP + TOOLBAR_HEIGHT, width);
        self.draw_tab_strip(canvas, width);
        self.draw_preview(canvas, width);
        self.draw_remote(canvas, width, height, body_top, now);
        #[cfg(feature = "phone-keyboard")]
        self.draw_phone_panel(canvas);
        self.draw_settings_panel(canvas);
        self.draw_history_panel(canvas);
        self.draw_bookmarks_panel(canvas, now);
        self.draw_star_popover(canvas);
        self.draw_tab_overflow(canvas, width);
        self.draw_popups(canvas);
        self.draw_page_controls(canvas);
        self.draw_tips(canvas, width, height);
        // Pages may have moved under a still pointer: by scrolling, a new
        // size or a zoom.
        self.wall.hover(self.pointer, now);
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser};
    use crate::test_canvas::Recorder;

    /// Requirement 19.8: the console docks in the overview's place.
    #[test]
    fn the_console_takes_the_overviews_place() {
        let (mut browser, _) = browser();
        browser.toggle_console();
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert_eq!(browser.regions.overview, Rect::ZERO, "no overview");
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        assert!(texts.contains(&"Network 0") && texts.contains(&"Pause"), "{texts:?}");
        assert!(browser.console.area.y1 <= HEIGHT && browser.console.area.y0 > BODY_TOP);
    }
}
