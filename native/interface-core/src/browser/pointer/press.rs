//! A press of the main button, given to the parts of the window in the
//! order they lie over each other: what lies over the window, the trusted
//! strip, then the body below it.

use crate::browser::list_focus::ListFocus;
use crate::browser::{Browser, Output, WindowAction};
use crate::canvas::Canvas;
use crate::channel::page_index;
use crate::wall::CellClick;

impl Browser {
    /// A press of the main button, with `extend` when Shift is held.
    pub fn pointer_pressed<C: Canvas + ?Sized>(&mut self, canvas: &mut C, extend: bool, now: u64) -> bool {
        self.tips_pressed();
        // A press means the button was let go since the last: a drag whose
        // release never came, as one let go outside every window, is over.
        self.tabs_cancel();
        // A press anywhere takes the keys from a list; one on a list gives
        // them to it again.
        self.list_focus = None;
        // Shift opens what a press opens in a new tab.
        self.press_new_tab = extend;
        // In full screen a press is the page's, unless the strip is down
        // and it is there (decision 85).
        if self.full_screen() && !self.full_screen_press_is_strip(now) {
            return self.press_body(now).unwrap_or(false);
        }
        if let Some(taken) = self.press_overlays(extend, now) {
            return taken;
        }
        if let Some(taken) = self.press_strip(canvas, extend, now) {
            return taken;
        }
        self.press_body(now).unwrap_or(false)
    }

    /// A press on what lies over the window: the drop-downs, the tab
    /// strip, the star's popover, the panels, the strip's question, the
    /// certificate-error screen and the remote. `None` when it was on none.
    fn press_overlays(&mut self, extend: bool, now: u64) -> Option<bool> {
        // The drop-downs and the star's popover are over everything; the
        // panels over everything below the strip.
        if self.popups_press(self.pointer, extend) || self.tabs_press(self.pointer) || self.star_press(self.pointer) {
            return Some(true);
        }
        if self.history_press(self.pointer, extend) || self.bookmarks_press(self.pointer) {
            return Some(true);
        }
        if self.settings_press(self.pointer) {
            return Some(true);
        }
        if self.console_press(self.pointer) {
            return Some(true);
        }
        #[cfg(feature = "phone-keyboard")]
        if self.phone_press(self.pointer) {
            return Some(true);
        }
        // The strip's question, drawn over the address field.
        #[cfg(feature = "links")]
        if self.link_press(now) {
            return Some(true);
        }
        if self.refused_press(self.pointer) {
            return Some(true);
        }
        if self.remote_press(self.pointer, now) {
            return Some(true);
        }
        None
    }

    /// A press on the trusted strip: the address field, Back and Forward,
    /// the gear, the menu, the fixed buttons, the bookmarks bar and the
    /// empty strip, which drags the window. `None` when it was on none.
    fn press_strip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, extend: bool, now: u64) -> Option<bool> {
        let point = self.pointer;
        let regions = &self.regions;
        if regions.field.contains(point) {
            if self.editing {
                let at = self.offset_at(canvas, point.x);
                self.editor.place(at, extend);
            } else {
                self.start_editing();
            }
            return Some(true);
        }
        self.stop_editing();
        let regions = &self.regions;
        // Back and Forward act when released, or open their lists when held.
        for (rect, forward, enabled) in [(regions.back, false, self.strip.back), (regions.forward, true, self.strip.forward)] {
            if rect.contains(point) {
                if enabled {
                    self.hold_back_or_forward(forward, now);
                }
                return Some(true);
            }
        }
        if regions.gear.contains(point) {
            self.toggle_settings();
            return Some(true);
        }
        if regions.menu.contains(point) {
            self.toggle_menu();
            return Some(true);
        }
        let mute_all = Output::MuteAll(!self.all_muted());
        let action = [
            (regions.new_tab, Output::NewTab),
            (regions.reload, Output::Reload),
            (regions.home, Output::Home),
            (regions.star, Output::Bookmark),
            (regions.mute_all, mute_all),
            (regions.minimise, Output::Window(WindowAction::Minimise)),
            (regions.maximise, Output::Window(WindowAction::Maximise)),
            (regions.close, Output::Window(WindowAction::Close)),
        ]
        .into_iter()
        .find(|(rect, _)| rect.contains(point));
        if let Some((_, output)) = action {
            self.outputs.push(output);
            return Some(true);
        }
        if self.bar_press(point, extend) {
            return Some(true);
        }
        let regions = &self.regions;
        if regions.strip.contains(point) {
            self.outputs.push(Output::Window(WindowAction::Drag));
            return Some(false);
        }
        None
    }

    /// A press below the strip: the divider, the banks, the list, the overview, the
    /// page's controls, the wall's controls and a page or its cell.
    /// `None` when it was on none.
    fn press_body(&mut self, now: u64) -> Option<bool> {
        let point = self.pointer;
        if self.divider_press(point) || self.subpages_press(point, now) {
            return Some(true);
        }
        let regions = &self.regions;
        if let Some(bank) = regions.banks.iter().position(|rect| rect.contains(point)) {
            self.choose_bank(bank);
            self.list_focus = Some(ListFocus::Magazines);
            return Some(true);
        }
        if regions.list.contains(point) {
            self.list_focus = Some(ListFocus::Pages);
            if let Some(&(_, index)) = regions.rows.iter().find(|(rect, _)| rect.contains(point)) {
                self.select_page(index, true);
            }
            return Some(true);
        }
        if regions.overview_toggle.contains(point) {
            self.toggle_overview();
            return Some(true);
        }
        if regions.cells.contains(point) {
            if let Some(index) = self.cell_index(point)
                && matches!(self.channel.pages.get(index), Some(Some(_)))
            {
                self.select_page(index, true);
            }
            return Some(true);
        }
        if self.controls_press(point) {
            return Some(true);
        }
        if self.wall.pointer_pressed(point, now) {
            return Some(true);
        }
        if let Some(click) = self.wall.cell_at(point) {
            if let Some(index) = page_index(click.key.page) {
                self.selected = Some(index);
                self.reveal_row(index);
            }
            // A page on the wall zooms to fill the area; on a zoomed page a
            // click is on one of its cells, where links will be followed.
            // On a page whose channel or page asked for the mouse a plain
            // click is the host's, as a terminal with mouse reporting gives
            // it; the browser's modifier keeps it the browser's (INPUT.md
            // section 6.5, REQ-INT-115).
            #[cfg(feature = "interaction")]
            if self.wall.zoomed().is_some() && self.click_is_hosts() {
                return Some(self.terminal_button(neoteletext_input::event::MouseAction::Press, 0));
            }
            // A double click on the open page: the page alone, full
            // screen (decision 85).
            if self.wall.zoomed().is_some() && !self.full_screen() && self.double_click(point, now) {
                return Some(self.toggle_full_screen(now));
            }
            if self.wall.zoomed().is_some() {
                let version = self.versions.get(&click.key).copied().unwrap_or_default();
                self.outputs.push(Output::Cell(CellClick { version, ..click }));
            } else {
                self.wall.zoom(click.key, now);
            }
            return Some(true);
        }
        None
    }
}
