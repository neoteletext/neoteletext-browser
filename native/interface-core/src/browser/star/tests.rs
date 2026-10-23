use super::*;
use crate::browser::Modifiers;
use crate::browser::fixture::{blank_browser, middle};
use crate::library::FolderRow;
use crate::test_canvas::Recorder;

/// Requirement 20.16: the star asks; the popover renames, moves, keeps
/// the whole channel, removes, and closes as Done when pressed outside.
#[test]
fn the_popover_asks_for_each_change() {
    let mut browser = blank_browser();
    browser.key(Key::Text("d"), Modifiers { command: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::Bookmark]);
    let folders =
        alloc::vec![FolderRow { number: 11, name: "Bookmarks bar".into(), depth: 0 }, FolderRow { number: 12, name: "Other bookmarks".into(), depth: 0 }];
    browser.set_star(StarView { open: true, name: "News 302 WEATHER".into(), folder: 12, folders, page: true });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let (name, folder) = (browser.star.controls.name, browser.star.controls.folder);
    browser.star_press(middle(name));
    browser.key(Key::Text("a"), Modifiers { command: true, ..Modifiers::default() }, 0);
    browser.key(Key::Text("Weather"), Modifiers::default(), 0);
    browser.star_press(middle(folder));
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let bar = browser.star.controls.folders[0].1;
    browser.star_press(middle(bar));
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let whole = browser.star.controls.whole;
    browser.star_press(middle(whole));
    assert_eq!(
        browser.take_outputs(),
        [
            Output::TextInput(true),
            Output::TextInput(false),
            Output::Star(StarRequest::Edit { name: "Weather".into(), folder: 11 }),
            Output::Star(StarRequest::WholeChannel)
        ]
    );
    // A press elsewhere is Done, with the name and folder.
    browser.star_press(Point::new(10.0, 600.0));
    assert_eq!(browser.take_outputs(), [Output::Star(StarRequest::Edit { name: "Weather".into(), folder: 11 }), Output::Star(StarRequest::Done)]);
    assert!(!browser.star_open());
}
