use super::*;
use crate::browser::fixture::middle;
use alloc::format;

use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
use crate::browser::regions::TabRegion;
use crate::browser::{Frame, Key, Modifiers, Output, WindowAction};
use crate::settings::UpdateView;
use crate::test_canvas::Recorder;
use crate::theme::MUTED;

fn browser_with(count: usize, active: usize) -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    let tabs = (0..count).map(|index| TabInfo {
        title: format!("Tab {index}"),
        colour: 0,
        channel: u32::try_from(index).unwrap(),
        muted: false,
        private: false,
        address: format!("https://tab{index}.invalid/"),
    });
    browser.set_tabs(tabs.collect(), active);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser
}

/// REQ-6640 item 8: the strip's row slides the least distance that shows
/// the active tab whole, within the row's ends. Before, the active tab
/// took the last place and the rest went to the overflow panel.
#[test]
fn the_strip_slides_the_least_distance_to_show_the_active_tab() {
    let step = TAB_WIDTH + TAB_GAP;
    let view = 4.0 * step - TAB_GAP;
    assert!(strip_slide(0.0, 2, 8, view, step).abs() < f64::EPSILON, "in view: no slide");
    assert!((strip_slide(0.0, 6, 8, view, step) - 3.0 * step).abs() < 1e-9, "the active tab is the last in view");
    assert!((strip_slide(3.0 * step, 1, 8, view, step) - step).abs() < 1e-9, "back to the left: the first in view");
    assert!((strip_slide(1.0e6, 7, 8, view, step) - 4.0 * step).abs() < 1e-9, "never past the row's end");
    assert!(strip_slide(0.0, 2, 3, view, step).abs() < f64::EPSILON, "all fit: none");
}

/// REQ-6640 item 8: when the active tab changes to one out of view, the
/// strip slides to it with easing, reaching it, and the count counts those
/// not shown.
#[test]
fn the_active_tab_is_always_shown() {
    let mut browser = browser_with(30, 0);
    let fitting = browser.regions.tabs.len();
    assert!(fitting < 30 && browser.tab_strip.count != Rect::ZERO, "a count of the rest");
    let tabs = browser.tabs.clone();
    browser.set_tabs(tabs, 25);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 1_000);
    assert!(browser.filmstrip.strip.moving(1_050), "the strip slides");
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 5_000);
    let active = browser.regions.tabs.iter().find(|region| region.index == 25).expect("the active tab is shown");
    assert!((active.tab.width() - TAB_WIDTH).abs() < 1e-9, "whole: {:?}", active.tab);
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 5_000);
    assert!(canvas.texts.iter().any(|(_, text)| *text == format!("+{}", 30 - fitting)), "the count of those not shown");
}

/// Decision 102: a tab has one width however many there are; tabs that
/// would not fit overflow rather than shrink. Before, six tabs on this
/// window shrank to 226 points.
#[test]
fn tabs_keep_one_width() {
    for count in [1, 2, 5, 6, 30] {
        let browser = browser_with(count, 0);
        let widths: Vec<f64> = browser.regions.tabs.iter().map(|region| region.tab.x1 - region.tab.x0).collect();
        assert!(!widths.is_empty() && widths.iter().all(|&width| (width - TAB_WIDTH).abs() < f64::EPSILON), "{count} tabs: {widths:?}");
    }
    assert!(browser_with(5, 0).tab_strip.count == Rect::ZERO, "five fit this window");
    assert!(browser_with(6, 0).tab_strip.count != Rect::ZERO, "a sixth overflows");
}

/// A tab dragged along the strip asks to move there; one barely moved
/// is only chosen; Escape cancels a drag.
#[test]
fn a_dragged_tab_moves_where_it_is_let_go() {
    let mut browser = browser_with(4, 0);
    let first = browser.regions.tabs[0].tab;
    let third = browser.regions.tabs[2].tab;
    browser.pointer_moved(middle(first).x - 20.0, middle(first).y, 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_moved(middle(third).x, middle(third).y, 0);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::SelectTab(0), Output::MoveTab { from: 0, to: 2 }]);
    browser.pointer_moved(middle(first).x - 20.0, middle(first).y, 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_moved(middle(first).x - 18.0, middle(first).y, 0);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::SelectTab(0)], "barely moved: chosen, not dragged");
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_moved(middle(third).x, middle(third).y, 0);
    browser.key(Key::Escape, Modifiers::default(), 0);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::SelectTab(0)], "Escape: it stays");
}

/// `multiple-windows.md`: a tab dragged off the strip, below it or out
/// of the window, leaves; let go just below the strip, it stays on it.
/// Another window says where on its strip a tab let go would go.
#[test]
fn a_tab_dragged_off_the_strip_leaves_the_window() {
    let mut browser = browser_with(4, 0);
    let first = browser.regions.tabs[0].tab;
    browser.pointer_moved(middle(first).x, middle(first).y, 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_moved(middle(first).x + 30.0, 300.0, 0);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::SelectTab(0), Output::TabOut { index: 0, x: whole(middle(first).x + 30.0), y: 300 }]);
    browser.pointer_moved(middle(first).x, middle(first).y, 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_moved(-200.0, -50.0, 0);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::SelectTab(0), Output::TabOut { index: 0, x: -200, y: -50 }], "out of the window");
    browser.pointer_moved(middle(first).x, middle(first).y, 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_moved(middle(first).x + 30.0, STRIP_HEIGHT + TEAR_DISTANCE / 2.0, 0);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::SelectTab(0)], "within the slack: still the strip's");
    let third = browser.regions.tabs[2].tab;
    assert_eq!(browser.tab_place_at(middle(third)), Some(2));
    assert_eq!(browser.tab_place_at(Point::new(third.x1 + 600.0, middle(third).y)), Some(4), "past the last tab: the end");
    assert_eq!(browser.tab_place_at(Point::new(middle(third).x, 400.0)), None);
    browser.key(Key::Text("n"), Modifiers { command: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::NewWindow { private: false }]);
}

/// The tab keys of decision 69.
#[test]
fn tab_keys_choose_tabs() {
    let mut browser = browser_with(12, 3);
    let command = Modifiers { command: true, ..Modifiers::default() };
    browser.key(Key::Text("2"), command, 0);
    browser.key(Key::Text("9"), command, 0);
    browser.key(Key::Right, Modifiers { alt: true, ..command }, 0);
    browser.key(Key::Left, Modifiers { alt: true, ..command }, 0);
    assert_eq!(browser.take_outputs(), [Output::SelectTab(1), Output::SelectTab(11), Output::SelectTab(4), Output::SelectTab(2)]);
}

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
            TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false, address: "https://example.invalid/".into() },
            TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false, private: false, address: "https://example.invalid/".into() }
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
            TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false, address: "https://example.invalid/".into() },
            TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false, private: false, address: "https://example.invalid/".into() }
        ],
        0,
    );
    press_at(&mut browser, &mut canvas, button);
    browser.set_tabs(
        alloc::vec![
            TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false, address: "https://example.invalid/".into() },
            TabInfo { title: "two".into(), colour: 1, channel: 8, muted: true, private: false, address: "https://example.invalid/".into() }
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

/// Decision 103: resting on a tab shows its title and, under it, its
/// address, drawn as two lines, once the filmstrip the rest opens first
/// has been put away with Escape (REQ-6640 item 9).
#[test]
fn a_tab_s_tip_shows_its_address() {
    let mut browser = browser_with(3, 0);
    let second = browser.regions.tabs[1].tab;
    browser.pointer_moved(middle(second).x, middle(second).y, 0);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    assert!(browser.settle(300) && browser.filmstrip_open(), "the rest opens the filmstrip first");
    assert!(browser.key(Key::Escape, Modifiers::default(), 300));
    browser.pointer_moved(middle(second).x + 1.0, middle(second).y, 310);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 310);
    assert!(browser.settle(1_000));
    assert!(!browser.filmstrip_open(), "put away, it stays shut while the pointer stays on the strip");
    assert_eq!(browser.tip_shown(), Some("Tab 1\nhttps://tab1.invalid/"));
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 1_000);
    for line in ["Tab 1", "https://tab1.invalid/"] {
        assert!(canvas.texts.iter().any(|(_, text)| text == line), "{line} drawn on a line of its own");
    }
}

/// The version the trusted core reports in these tests.
const VERSION: &str = "0.1.0";
/// The macOS window buttons' room at the strip's left (the interface's
/// `window::FRAME` there).
const MAC_INSET: f64 = 72.0;

/// A browser of `count` tabs, the last active, in a window `width` wide,
/// framed as `frame`, told the version running and drawn; with what it
/// drew.
fn versioned(frame: Frame, count: usize, width: f64) -> (Browser, Recorder) {
    let mut browser = Browser::new(frame);
    let tabs = (0..count).map(|index| TabInfo {
        title: format!("Tab {index}"),
        colour: 0,
        channel: u32::try_from(index).unwrap(),
        muted: false,
        private: false,
        address: format!("https://tab{index}.invalid/"),
    });
    browser.set_tabs(tabs.collect(), count.saturating_sub(1));
    browser.set_update(UpdateView { version: VERSION.into(), status: "NeoTeletext is up to date".into(), offer: None, checking: true });
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, width, 900.0, 0);
    (browser, canvas)
}

/// Where the version label was drawn, if it was.
fn label_drawn(canvas: &Recorder) -> Option<Rect> {
    let text = format!("{VERSION} {RELEASE_STAGE}");
    canvas.texts.iter().find(|(_, drawn)| *drawn == text).map(|&(rect, _)| rect)
}

/// Whether two rectangles share any area.
fn overlap(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// Decision 123: the strip shows the version the trusted core reports
/// running, the one the settings panel shows, and BETA after it, in the
/// secondary text's colour; BETA alone before the trusted core reports.
#[test]
fn the_version_label_is_the_version_and_beta() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    assert_eq!(browser.version_label(), "BETA", "before the trusted core reports");
    browser.set_update(UpdateView { version: VERSION.into(), ..UpdateView::default() });
    assert_eq!(browser.version_label(), "0.1.0 BETA");
    let (_, canvas) = versioned(Frame { left_inset: 0.0, window_buttons: false }, 3, 1440.0);
    assert!(label_drawn(&canvas).is_some(), "drawn: {:?}", canvas.texts);
    assert_eq!(canvas.coloured_text("0.1.0 BETA"), Some(MUTED), "quiet, as secondary text");
}

/// Decision 123: the label ends at the strip's right end, before the
/// window's own buttons where the browser draws them, inside the strip.
#[test]
fn the_version_label_is_at_the_strip_s_right_end() {
    let (_, canvas) = versioned(Frame { left_inset: MAC_INSET, window_buttons: false }, 3, 1440.0);
    let label = label_drawn(&canvas).expect("drawn on macOS");
    assert!((label.x1 - (1440.0 - STRIP_MARGIN - VERSION_GAP)).abs() < f64::EPSILON, "at the right end: {label:?}");
    assert!(label.y0 >= 0.0 && label.y1 <= STRIP_HEIGHT, "in the strip: {label:?}");
    let (browser, canvas) = versioned(Frame { left_inset: 0.0, window_buttons: true }, 3, 1440.0);
    let label = label_drawn(&canvas).expect("drawn with the window's buttons");
    let minimise = browser.regions.minimise;
    assert!((label.x1 - (minimise.x0 - WINDOW_BUTTON_GAP - VERSION_GAP)).abs() < f64::EPSILON, "just before the window's buttons: {label:?}");
    let new_tab = browser.regions.new_tab;
    assert!(browser.regions.tabs.iter().all(|region| region.tab.x1 < label.x0) && new_tab.x1 < label.x0, "after every tab and the new-tab button");
}

/// Decision 123: at a wide window and at the narrowest, with the window's
/// buttons on either side, the label covers no tab, count, new-tab
/// button or window button, and the active tab and the count stay on the
/// strip; where there is not room for it beside one tab, the count and
/// the new-tab button, it is left out.
#[test]
fn the_version_label_never_covers_a_tab_or_the_new_tab_button() {
    for frame in [Frame { left_inset: MAC_INSET, window_buttons: false }, Frame { left_inset: 0.0, window_buttons: true }] {
        for width in [1440.0, 1000.0] {
            for count in [1, 5, 30] {
                let (browser, canvas) = versioned(frame, count, width);
                let label = label_drawn(&canvas).unwrap_or_else(|| panic!("{frame:?}, {width} wide, {count} tabs: drawn"));
                let regions = &browser.regions;
                let mut others: Vec<Rect> = regions.tabs.iter().map(|region| region.tab).collect();
                others.extend([regions.new_tab, browser.tab_strip.count, regions.minimise, regions.maximise, regions.close]);
                for other in others.into_iter().filter(|&other| other != Rect::ZERO) {
                    assert!(!overlap(label, other), "{frame:?}, {width} wide, {count} tabs: {label:?} covers {other:?}");
                }
                assert!(regions.tabs.iter().any(|region| region.index == count - 1), "{frame:?}, {width} wide: the active tab is shown");
                assert_eq!(browser.tab_strip.count != Rect::ZERO, regions.tabs.len() < count, "{frame:?}, {width} wide, {count} tabs: the rest counted");
            }
        }
    }
    let narrow = MAC_INSET + STRIP_MARGIN * 2.0 + LEAST_TABS_ROOM;
    let (browser, canvas) = versioned(Frame { left_inset: MAC_INSET, window_buttons: false }, 30, narrow);
    assert!(label_drawn(&canvas).is_none(), "no room beside the tabs: left out");
    assert!(browser.regions.new_tab.x1 <= narrow - STRIP_MARGIN, "the new-tab button still in the window");
}

/// DEC-1549 items 3 and 4: where a drop lands, by the tab's quarters.
#[test]
fn a_drop_lands_by_the_quarters_of_a_tab() {
    use crate::browser::{DropPlace, DropTarget};

    let (browser, _) = browser();
    let tab = browser.regions.tabs.first().unwrap();
    let (index, rect) = (u8::try_from(tab.index).unwrap(), tab.tab);
    let at = |fraction: f64| Point::new(rect.x0 + rect.width() * fraction, f64::midpoint(rect.y0, rect.y1));
    assert_eq!(browser.drop_target(at(0.1)), DropTarget::Tabs(DropPlace::Before(index)));
    assert_eq!(browser.drop_target(at(0.5)), DropTarget::Tabs(DropPlace::Replace(index)));
    assert_eq!(browser.drop_target(at(0.9)), DropTarget::Tabs(DropPlace::After(index)));
    assert_eq!(browser.drop_target(Point::new(1.0, 500.0)), DropTarget::Tabs(DropPlace::NewTabs), "the page");
    let active = u8::try_from(browser.active).unwrap();
    assert_eq!(browser.drop_target(Point::new(1.0, 60.0)), DropTarget::Tabs(DropPlace::Replace(active)), "the toolbar");
    let field = browser.regions.field_text;
    assert_eq!(browser.drop_target(Point::new(f64::midpoint(field.x0, field.x1), f64::midpoint(field.y0, field.y1))), DropTarget::Address);
}
