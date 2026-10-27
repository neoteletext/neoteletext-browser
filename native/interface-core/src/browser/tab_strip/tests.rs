use super::*;
use crate::browser::fixture::middle;
use alloc::format;
use alloc::vec;

use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
use crate::browser::regions::TabRegion;
use crate::browser::{Frame, Key, Modifiers, Output, WindowAction};
use crate::test_canvas::Recorder;
use crate::theme::{BLACK, RAISED, SELECTED};

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

/// Decision 69: the tabs that fit, the active one always among them,
/// the rest in order for the panel.
#[test]
fn the_active_tab_is_always_shown() {
    assert_eq!(arrangement(3, 0, 5), (vec![0, 1, 2], vec![]));
    assert_eq!(arrangement(8, 1, 4), (vec![0, 1, 2, 3], vec![4, 5, 6, 7]));
    assert_eq!(arrangement(8, 6, 4), (vec![0, 1, 2, 6], vec![3, 4, 5, 7]), "the active tab takes the last place");
}

/// Opens the overflow panel as a user does: the pointer rests on the tabs.
fn rest_on_the_tabs(browser: &mut Browser, now: u64) {
    browser.pointer_moved(5.0 + STRIP_MARGIN, 20.0, now);
    assert_eq!(browser.frame_interval(now), Some(PANEL_DELAY_MILLISECONDS), "the panel waits for the pointer to rest");
    assert!(browser.settle(now.saturating_add(PANEL_DELAY_MILLISECONDS)), "the panel opens");
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, now.saturating_add(PANEL_DELAY_MILLISECONDS));
}

/// Decisions 69 and 102: too many tabs show a count; resting on the tabs
/// opens the panel the rest wrap into, every one reachable; a tab chosen
/// there stays where it is while the panel is open; leaving closes it.
#[test]
fn overflowing_tabs_wrap_into_a_panel_under_the_strip() {
    let mut browser = browser_with(30, 25);
    let shown = browser.regions.tabs.len();
    assert!(shown < 30 && browser.tab_strip.count != Rect::ZERO, "a count of the rest");
    assert!(browser.regions.tabs.iter().any(|region| region.index == 25), "the active tab is shown");
    rest_on_the_tabs(&mut browser, 0);
    let mut indices: Vec<usize> = browser.regions.tabs.iter().map(|region| region.index).collect();
    indices.sort_unstable();
    assert_eq!(indices, (0..30).collect::<Vec<_>>(), "every tab, on the strip or in the panel");
    let in_panel = *browser.regions.tabs.last().unwrap();
    assert!(in_panel.tab.y0 >= STRIP_HEIGHT - TAB_TOP_INACTIVE, "below the strip");
    browser.pointer_moved(middle(in_panel.tab).x, middle(in_panel.tab).y, 1_000);
    browser.pointer_pressed(&mut Recorder::new(), false, 1_000);
    assert_eq!(browser.take_outputs(), [Output::SelectTab(in_panel.index)], "a tab in the panel is chosen");
    // The trusted core makes it active, as asked.
    let tabs = browser.tabs.clone();
    browser.set_tabs(tabs, in_panel.index);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 1_000);
    let chosen = browser.regions.tabs.iter().find(|region| region.index == in_panel.index).map(|region| region.tab);
    assert_eq!(chosen.map(|tab| (tab.x0, tab.x1)), Some((in_panel.tab.x0, in_panel.tab.x1)), "it stays under the pointer");
    assert_eq!(chosen.map(|tab| tab.y1), Some(in_panel.tab.y1), "in its row of the panel");
    browser.pointer_released();
    browser.pointer_moved(700.0, 600.0, 1_100);
    assert!(!browser.tab_strip.open, "leaving closes the panel");
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 1_100);
    assert!(browser.regions.tabs.iter().any(|region| region.index == in_panel.index && region.tab.y1 == STRIP_HEIGHT), "then the active tab is on the strip");
}

/// Decision 102: the open panel is raised as a dialog is: the strip and
/// the panel one surface, a blue edge at the panel's foot, the window
/// below dimmed. Shut, the strip is black and nothing is dimmed.
#[test]
fn the_open_panel_stands_out_from_the_window() {
    let mut browser = browser_with(30, 0);
    let mut shut = Recorder::new();
    browser.draw(&mut shut, 1440.0, 900.0, 0);
    assert!(shut.dims.is_empty(), "nothing dimmed while shut");
    assert!(shut.fills.iter().any(|&(rect, colour)| colour == BLACK && rect.y0 == 0.0 && rect.y1 == STRIP_HEIGHT), "the strip black");
    rest_on_the_tabs(&mut browser, 0);
    let mut open = Recorder::new();
    browser.draw(&mut open, 1440.0, 900.0, 1_000);
    let panel = browser.tab_strip.panel;
    assert!(open.fills.iter().any(|&(rect, colour)| colour == RAISED && rect.y0 == 0.0 && rect.y1 == STRIP_HEIGHT), "the strip raised");
    assert!(open.fills.iter().any(|&(rect, colour)| colour == RAISED && rect == panel), "the panel the same surface");
    assert!(
        open.fills.iter().any(|&(rect, colour)| colour == SELECTED && rect == Rect::new(0.0, panel.y1 - PANEL_EDGE, 1440.0, panel.y1)),
        "a blue edge at its foot"
    );
    assert!(open.dims.contains(&(Rect::new(0.0, panel.y1, 1440.0, 900.0), PANEL_DIM)), "the window below dimmed: {:?}", open.dims);
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

/// Decision 102: a pointer crossing the strip to the toolbar does not
/// open the panel, so the address field under it can be clicked; one
/// resting on the tabs does. Before, the panel opened at once, lay over
/// the toolbar, and took the field's click as a tab's.
#[test]
fn a_pointer_crossing_the_strip_does_not_open_the_panel() {
    let mut browser = browser_with(30, 0);
    let field = browser.regions.field;
    browser.pointer_moved(field.x0 + 20.0, 20.0, 0);
    browser.pointer_moved(field.x0 + 20.0, f64::midpoint(field.y0, field.y1), 100);
    browser.settle(1_000);
    assert!(!browser.tab_strip.open, "crossed, not rested");
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 1_000);
    browser.pointer_pressed(&mut Recorder::new(), false, 1_000);
    assert!(browser.editing, "the field takes the click");
    assert_eq!(browser.take_outputs(), [Output::TextInput(true)], "the field's text input, no tab chosen");
}

/// The count opens the panel at once, for a pad or a touch screen, and
/// shuts it; shut so, it stays shut until the pointer leaves the strip.
/// Before, a second press did nothing.
#[test]
fn the_count_opens_and_shuts_the_panel() {
    let mut browser = browser_with(30, 0);
    let count = browser.tab_strip.count;
    browser.pointer_moved(middle(count).x, middle(count).y, 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_released();
    assert!(browser.tab_strip.open, "open at once");
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 10);
    browser.pointer_released();
    assert!(!browser.tab_strip.open, "shut");
    browser.settle(10_000);
    assert!(!browser.tab_strip.open, "and stays shut while the pointer rests there");
    browser.pointer_moved(700.0, 600.0, 10_000);
    rest_on_the_tabs(&mut browser, 11_000);
    assert!(browser.tab_strip.open, "back on the tabs, it opens again");
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
/// address, drawn as two lines.
#[test]
fn a_tab_s_tip_shows_its_address() {
    let mut browser = browser_with(3, 0);
    let second = browser.regions.tabs[1].tab;
    browser.pointer_moved(middle(second).x, middle(second).y, 0);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    assert!(browser.settle(1_000));
    assert_eq!(browser.tip_shown(), Some("Tab 1\nhttps://tab1.invalid/"));
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 1_000);
    for line in ["Tab 1", "https://tab1.invalid/"] {
        assert!(canvas.texts.iter().any(|(_, text)| text == line), "{line} drawn on a line of its own");
    }
}
