use super::*;
use crate::browser::fixture::middle;
use crate::browser::{Frame, StripState};
use crate::library::TabEntry;
use crate::test_canvas::Recorder;

fn browser() -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.set_strip(StripState { address: "https://news.example/".into(), back: true, forward: true, ..StripState::default() });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser
}

fn press(browser: &mut Browser, rect: Rect, now: u64) {
    let point = middle(rect);
    browser.pointer_moved(point.x, point.y, now);
    browser.pointer_pressed(&mut Recorder::new(), false, now);
}

/// Requirement 20.10: a press and release goes back; held, the list is
/// asked for and nothing else; an entry goes that many steps.
#[test]
fn back_held_opens_its_list() {
    let mut browser = browser();
    let back = browser.regions.back;
    press(&mut browser, back, 0);
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::Back]);
    press(&mut browser, back, 1_000);
    assert_eq!(browser.hold_deadline(1_100), Some(400));
    assert!(browser.settle(1_000 + HOLD_MILLISECONDS));
    browser.pointer_released();
    assert_eq!(browser.take_outputs(), [Output::TabList(false)]);
    browser.set_tab_list(TabList {
        forward: false,
        entries: alloc::vec![
            TabEntry { steps: 1, page: None, title: "a".into(), address: "https://a.example/".into() },
            TabEntry { steps: 2, page: Some(0x302), title: "b".into(), address: "https://b.example/#302".into() }
        ],
        chapters: alloc::vec![TabEntry { steps: 20, page: None, title: "c".into(), address: "https://c.example/".into() }],
    });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let second = browser.tab_lists.entries[1].1;
    press(&mut browser, second, 2_000);
    assert_eq!(browser.take_outputs(), [Output::GoTo { forward: false, steps: 2 }]);
    browser.set_tab_list(TabList {
        forward: false,
        entries: alloc::vec![],
        chapters: alloc::vec![TabEntry { steps: 20, page: None, title: "c".into(), address: "https://c.example/".into() }],
    });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let chapter = browser.tab_lists.entries[0].1;
    press(&mut browser, chapter, 2_100);
    assert_eq!(browser.take_outputs(), [Output::GoTo { forward: false, steps: 20 }], "a chapter stop goes there");
    assert!(!browser.secondary_pressed(), "nothing open, and not on Back");
    assert_eq!(browser.take_outputs(), []);
    browser.pointer_moved(middle(back).x, middle(back).y, 3_000);
    assert!(browser.secondary_pressed());
    assert_eq!(browser.take_outputs(), [Output::TabList(false)]);
}
