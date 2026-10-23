//! The browser window, after the owner's mockups: the tab strip and
//! window buttons along the top as Chrome draws them, the toolbar with the
//! trusted strip's address field, and below it the page banks on the left,
//! the chosen bank's page list in the middle, the zoomable page area on the
//! right, and the overview of all 800 pages along the bottom.
//!
//! The browser holds only what it is shown. What it shows comes from the
//! trusted core; what the user does leaves as [`Output`]s, which the host
//! turns into messages (design section 5.2).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::glyphs::{WINDOW_GLYPH, WINDOW_GLYPH_THICKNESS};
use crate::browser::regions::Regions;
use crate::canvas::{Canvas, PageKey};
use crate::channel::{Channel, PAGES_PER_BANK, page_number};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::theme::{BLACK, GREEN, RED, YELLOW};
use crate::ui::border;
use crate::wall::Wall;

mod address;
mod banks;
mod bookmarks_ui;
mod console_ui;
mod controls_ui;
mod cookies_ui;
#[cfg(test)]
mod fixture;
mod focus;
mod frames;
mod glyphs;
mod history_ui;
mod keys;
#[cfg(feature = "links")]
mod link_ui;
mod output;
mod overview;
mod page_list;
mod pages;
#[cfg(feature = "phone-keyboard")]
mod phone_ui;
mod pointer;
mod popup_ui;
mod refused_ui;
mod regions;
mod remote_ui;
mod settings_ui;
mod shown;
mod star_ui;
mod tabs_ui;
#[cfg(feature = "interaction")]
mod terminal_ui;
mod tips_ui;
mod toolbar;
mod widgets;
mod window;

pub use keys::{Key, Modifiers};
pub use output::{Output, WindowAction};
#[cfg(feature = "phone-keyboard")]
pub use phone_ui::{PhoneStatus, PhoneUiView};
pub use pointer::Cursor;
pub use shown::{CertificateProblem, Frame, StripState, TabInfo};
#[cfg(feature = "interaction")]
pub use terminal_ui::{CursorView, TypingView};

/// The address field's limit, as the trusted core's address rules.
pub const MAX_ADDRESS: usize = 2048;

// ---- Tab strip ----
const STRIP_HEIGHT: f64 = 44.0;
const TAB_TOP: f64 = 6.0;
const TAB_TOP_INACTIVE: f64 = 9.0;
const TAB_MAX_WIDTH: f64 = 230.0;
const TAB_MIN_WIDTH: f64 = 90.0;
const TAB_GAP: f64 = 4.0;
const TAB_TEXT_INSET: f64 = 14.0;
const TAB_CLOSE: f64 = 22.0;
/// A private tab's mark, before its title.
const PRIVATE_MARK: f64 = 10.0;
/// A tab's sound button, before its close button, as Chrome places it.
const TAB_SPEAKER: f64 = 24.0;
const NEW_TAB: f64 = 38.0;
const STRIP_MARGIN: f64 = 8.0;
const WINDOW_BUTTON_WIDTH: f64 = 44.0;
const WINDOW_BUTTON_HEIGHT: f64 = 32.0;
const WINDOW_BUTTON_GAP: f64 = 6.0;
// ---- Toolbar ----
// ---- Panels ----
// ---- Overview ----
/// Scrolling by one wheel line.
pub const LINE_HEIGHT: f64 = 21.0;
/// The whole browser window: tabs, trusted strip, banks, page list, page
/// area and overview. It draws through a [`Canvas`] and reports what the
/// user did as [`Output`]s; it holds no state the trusted core owns.
pub struct Browser {
    frame: Frame,
    tabs: Vec<TabInfo>,
    active: usize,
    /// The channels playing sound, muted or not.
    sounding: BTreeSet<u32>,
    strip: StripState,
    channel: Channel,
    bank: usize,
    selected: Option<usize>,
    list_scroll: f64,
    wall: Wall,
    editor: Editor,
    editing: bool,
    field_scroll: f64,
    digits: String,
    pulses: Vec<(usize, u64)>,
    /// When each recently updated page had its new version.
    updates: Vec<(usize, u64)>,
    /// The overview folded into the footer: it then draws nothing but its
    /// heading and keeps no animation.
    overview_folded: bool,
    /// The overview's page under the pointer, shown at its native size.
    previewed: Option<usize>,
    pointer: Point,
    visible: Vec<PageKey>,
    /// The version of each page's latest image, as the trusted core sent
    /// it: a click names the image it was made on.
    versions: BTreeMap<PageKey, u32>,
    regions: Regions,
    outputs: Vec<Output>,
    /// The window below the strip, where the settings panel is centred.
    body: Rect,
    /// The settings panel (requirement 19.3).
    settings: settings_ui::SettingsUi,
    /// The developer console (requirement 19.8).
    console: console_ui::ConsoleUi,
    /// The history panel (requirement 20.4).
    history: history_ui::HistoryUi,
    /// The cookies panel (requirement 21.6).
    cookies: cookies_ui::CookiesUi,
    /// What each control is, shown when the pointer rests on it.
    tips: tips_ui::TipsUi,
    /// The open page's controls (decision 67).
    page_controls: controls_ui::ControlsUi,
    /// The remote control (decision 68).
    remote: remote_ui::RemoteUi,
    /// The certificate-error screen's buttons (decision 83).
    refused: refused_ui::RefusedUi,
    /// The tab strip's overflow and dragging (decision 69).
    tab_strip: tabs_ui::TabsUi,
    /// The bookmarks manager (requirement 20.17).
    bookmarks: bookmarks_ui::BookmarksUi,
    /// The star's popover (requirement 20.16).
    star: star_ui::StarUi,
    /// The bookmarks bar (requirement 20.18).
    bar: star_ui::BarUi,
    /// The menu, Back's and Forward's lists, and the suggestions.
    popups: popup_ui::PopupUi,
    /// The last press asked for a new tab: Shift was held.
    press_new_tab: bool,
    /// Hyperlinks (`hyperlinks.md`).
    #[cfg(feature = "links")]
    links: link_ui::LinkUi,
    /// Typing into a page (`interaction.md`).
    #[cfg(feature = "interaction")]
    terminal: terminal_ui::TerminalUi,
    /// The phone keyboard (INPUT.md section 10).
    #[cfg(feature = "phone-keyboard")]
    phone: phone_ui::PhoneUi,
}

impl Browser {
    /// A browser with no tabs and an empty channel, drawn in `frame`.
    #[must_use]
    pub fn new(frame: Frame) -> Self {
        Self {
            frame,
            tabs: Vec::new(),
            active: 0,
            sounding: BTreeSet::new(),
            strip: StripState::default(),
            channel: Channel::empty(),
            bank: 0,
            selected: None,
            list_scroll: 0.0,
            wall: Wall::new(),
            editor: Editor::new(MAX_ADDRESS),
            editing: false,
            field_scroll: 0.0,
            digits: String::new(),
            pulses: Vec::new(),
            updates: Vec::new(),
            overview_folded: false,
            previewed: None,
            pointer: Point::ORIGIN,
            visible: Vec::new(),
            versions: BTreeMap::new(),
            regions: Regions::default(),
            outputs: Vec::new(),
            body: Rect::ZERO,
            settings: settings_ui::SettingsUi::default(),
            console: console_ui::ConsoleUi::default(),
            history: history_ui::HistoryUi::default(),
            cookies: cookies_ui::CookiesUi::default(),
            tips: tips_ui::TipsUi::default(),
            page_controls: controls_ui::ControlsUi::default(),
            remote: remote_ui::RemoteUi::default(),
            refused: refused_ui::RefusedUi::default(),
            tab_strip: tabs_ui::TabsUi::default(),
            bookmarks: bookmarks_ui::BookmarksUi::default(),
            star: star_ui::StarUi::default(),
            bar: star_ui::BarUi::default(),
            popups: popup_ui::PopupUi::default(),
            press_new_tab: false,
            #[cfg(feature = "links")]
            links: link_ui::LinkUi::default(),
            #[cfg(feature = "interaction")]
            terminal: terminal_ui::TerminalUi::default(),
            #[cfg(feature = "phone-keyboard")]
            phone: phone_ui::PhoneUi::default(),
        }
    }

    /// A new image of a page arrived, of this version.
    pub fn frame_version(&mut self, key: PageKey, version: u32) {
        self.versions.insert(key, version);
        // The links under the pointer are now those of the new image.
        #[cfg(feature = "links")]
        self.repoint();
    }

    /// The tabs as the trusted core reports them, and which is active.
    pub fn set_tabs(&mut self, tabs: Vec<TabInfo>, active: usize) {
        self.active = active.min(tabs.len().saturating_sub(1));
        self.tabs = tabs;
    }

    /// The channels playing sound, muted or not, as the host's mixers
    /// report. Returns whether that changed what the tabs show.
    pub fn set_sounding(&mut self, sounding: BTreeSet<u32>) -> bool {
        let changed = sounding != self.sounding;
        self.sounding = sounding;
        changed
    }

    /// Whether the tab plays sound, muted or not.
    #[must_use]
    pub fn tab_sounding(&self, tab: &TabInfo) -> bool {
        self.sounding.contains(&tab.channel)
    }

    /// Whether every tab is muted: the mute-all button then unmutes them.
    #[must_use]
    pub fn all_muted(&self) -> bool {
        !self.tabs.is_empty() && self.tabs.iter().all(|tab| tab.muted)
    }

    /// What the trusted strip shows, as the trusted core decides it.
    pub fn set_strip(&mut self, strip: StripState) {
        self.strip = strip;
    }

    /// The page area.
    #[must_use]
    pub fn wall(&self) -> &Wall {
        &self.wall
    }

    /// The page area, to change.
    pub fn wall_mut(&mut self) -> &mut Wall {
        &mut self.wall
    }

    /// Everything the user did since the last call.
    pub fn take_outputs(&mut self) -> Vec<Output> {
        core::mem::take(&mut self.outputs)
    }

    // ---- Input ----

    // ---- Drawing ----

    fn draw_tab_strip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
        canvas.fill(Rect::new(0.0, 0.0, width, STRIP_HEIGHT), 0.0, BLACK);
        let mut right = width - STRIP_MARGIN;
        if self.frame.window_buttons {
            let top = (STRIP_HEIGHT - WINDOW_BUTTON_HEIGHT) / 2.0;
            let button = |right: &mut f64| {
                let rect = Rect::new(*right - WINDOW_BUTTON_WIDTH, top, *right, top + WINDOW_BUTTON_HEIGHT);
                *right = rect.x0 - WINDOW_BUTTON_GAP;
                rect
            };
            self.regions.close = button(&mut right);
            self.regions.maximise = button(&mut right);
            self.regions.minimise = button(&mut right);
            draw_window_buttons(canvas, &self.regions);
        }
        let left = self.frame.left_inset + STRIP_MARGIN;
        self.draw_tabs(canvas, left, right);
        self.regions.strip = Rect::new(0.0, 0.0, width, STRIP_HEIGHT);
    }
}

fn draw_window_buttons<C: Canvas + ?Sized>(canvas: &mut C, regions: &Regions) {
    for (rect, colour) in [(regions.minimise, GREEN), (regions.maximise, YELLOW), (regions.close, RED)] {
        canvas.fill(rect, 0.0, colour);
    }
    let centre = |rect: Rect| Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let half = WINDOW_GLYPH / 2.0;
    let thick = WINDOW_GLYPH_THICKNESS;
    let minimise = centre(regions.minimise);
    canvas.fill(Rect::new(minimise.x - half, minimise.y + half - thick, minimise.x + half, minimise.y + half), 0.0, BLACK);
    let maximise = centre(regions.maximise);
    border(canvas, Rect::new(maximise.x - half, maximise.y - half, maximise.x + half, maximise.y + half), thick, BLACK);
    // The close cross: two bars, each a thin parallelogram from corner to corner.
    let close = centre(regions.close);
    let normal = thick / 2.0 / core::f64::consts::SQRT_2;
    for direction in [1.0, -1.0] {
        let along = Point::new(half, half * direction);
        let across = Point::new(-normal * direction, normal);
        canvas.polygon(&[close - along + across, close + along + across, close + along - across, close - along - across], BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
    use crate::browser::regions::TabRegion;

    #[test]
    fn a_tab_playing_sound_shows_a_speaker_that_mutes_it() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        assert!(browser.regions.tabs.iter().all(|region| region.speaker.is_none()), "no sound, no speaker");
        assert!(browser.set_sounding(BTreeSet::from([8])));
        assert!(!browser.set_sounding(BTreeSet::from([8])), "the same again changes nothing");
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.regions.tabs[0].speaker.is_none());
        let speaker = browser.regions.tabs[1].speaker.expect("the second tab plays sound");
        press_at(&mut browser, &mut canvas, speaker);
        assert_eq!(browser.take_outputs(), [Output::MuteTab(1)], "a click on the speaker mutes, and does not select");
        // Muted, the speaker stays even once the sound ends.
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false, private: false }
            ],
            0,
        );
        browser.set_sounding(BTreeSet::new());
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.regions.tabs[0].speaker.is_some() && browser.regions.tabs[1].speaker.is_none());
    }

    #[test]
    fn the_mute_all_button_mutes_every_tab_until_all_are_muted() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let button = browser.regions.mute_all;
        press_at(&mut browser, &mut canvas, button);
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false, private: false }
            ],
            0,
        );
        press_at(&mut browser, &mut canvas, button);
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: true, private: false }
            ],
            0,
        );
        press_at(&mut browser, &mut canvas, button);
        assert_eq!(browser.take_outputs(), [Output::MuteAll(true), Output::MuteAll(true), Output::MuteAll(false)]);
    }

    #[test]
    fn tabs_back_and_window_buttons_report_what_was_pressed() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let TabRegion { tab: second, close, .. } = browser.regions.tabs[1];
        press_at(&mut browser, &mut canvas, Rect::new(second.x0, second.y0, second.x0 + 10.0, second.y1));
        press_at(&mut browser, &mut canvas, close);
        let (back, forward, close_window) = (browser.regions.back, browser.regions.forward, browser.regions.close);
        press_at(&mut browser, &mut canvas, back);
        browser.pointer_released();
        // Forward acts when released, so that holding it opens its list.
        press_at(&mut browser, &mut canvas, forward);
        browser.pointer_released();
        press_at(&mut browser, &mut canvas, close_window);
        assert_eq!(
            browser.take_outputs(),
            [Output::SelectTab(1), Output::CloseTab(1), Output::Forward, Output::Window(WindowAction::Close)],
            "back is disabled, so it reports nothing"
        );
    }

    /// Requirement 12.1 (decision 63): the middle button closes the tab it
    /// is on, and does nothing elsewhere.
    #[test]
    fn the_middle_button_closes_the_tab_under_it() {
        let (mut browser, mut canvas) = browser();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        browser.take_outputs();
        let second = browser.regions.tabs[1].tab;
        browser.pointer_moved(f64::midpoint(second.x0, second.x1), f64::midpoint(second.y0, second.y1), 0);
        assert!(browser.middle_pressed());
        assert_eq!(browser.take_outputs(), [Output::CloseTab(1)]);
        browser.pointer_moved(WIDTH / 2.0, HEIGHT / 2.0, 0);
        assert!(!browser.middle_pressed());
        assert_eq!(browser.take_outputs(), [], "not on a tab: nothing");
    }
}
