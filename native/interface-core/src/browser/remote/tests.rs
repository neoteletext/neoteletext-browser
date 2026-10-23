use super::*;
use crate::browser::fixture::middle;

use crate::browser::{Browser, Frame, Key, Modifiers, Output};
use crate::controls::{FastextKey, PageControl};
use crate::geometry::{Point, Rect};
use crate::settings::{RemoteMode, RemotePlace, SettingChange};
use crate::test_canvas::Recorder;

const WIDTH: f64 = 1440.0;
const HEIGHT: f64 = 900.0;

fn rect_of(browser: &Browser, wanted: Button) -> Rect {
    browser.remote.buttons.iter().find(|(button, _)| *button == wanted).unwrap().1
}

/// Decision 68: hidden at first, its shortcut docks it and the trusted
/// core is told; docked it narrows the body, sliding in; its handle
/// puts it away again.
#[test]
fn the_remote_docks_slides_and_goes_away() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    let full = browser.body;
    assert!(!browser.remote_shown() && browser.remote.panel == Rect::ZERO);
    browser.key(Key::Text("r"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
    let docked = RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 };
    assert_eq!(browser.take_outputs(), [Output::Setting(SettingChange::Remote(docked))]);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, SLIDE_MILLISECONDS / 2);
    assert!(browser.remote_animating(SLIDE_MILLISECONDS / 2), "sliding in");
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, SLIDE_MILLISECONDS);
    assert!((full.x1 - browser.body.x1 - (REMOTE_WIDTH + crate::browser::window::MARGIN)).abs() < 0.5, "the body narrows by the remote");
    let handle = browser.remote.handle;
    assert!(browser.remote_press(middle(handle), SLIDE_MILLISECONDS));
    assert_eq!(browser.take_outputs(), [Output::Setting(SettingChange::Remote(RemotePlace { mode: RemoteMode::Hidden, x: 0, y: 0 }))]);
}

/// Its keys are the inputs that already exist: digits choose a page,
/// the page keys step, the controls go to the trusted core.
#[test]
fn the_remotes_keys_are_the_existing_inputs() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.set_remote(RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, SLIDE_MILLISECONDS);
    for digit in ['3', '0'] {
        let key = rect_of(&browser, Button::Digit(digit));
        browser.remote_press(middle(key), SLIDE_MILLISECONDS);
    }
    assert_eq!(browser.digits, "30", "two digits of three entered");
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, SLIDE_MILLISECONDS);
    assert!(canvas.texts.iter().any(|(_, text)| text == "P30-"), "the readout shows the entry");
    for control in [PageControl::Reveal, PageControl::Fastext(FastextKey::Yellow), PageControl::SubPage(true)] {
        let key = rect_of(&browser, Button::Control(control));
        browser.remote_press(middle(key), SLIDE_MILLISECONDS);
        assert_eq!(browser.take_outputs(), [Output::PageControl(control)]);
    }
}

/// Floating, it moves by its title row, stays below the strip and in
/// the window, and the trusted core keeps where it was put.
#[test]
fn a_floating_remote_moves_and_stays_below_the_strip() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.set_remote(RemotePlace { mode: RemoteMode::Floating, x: 600, y: 300 });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    let top = browser.body.y0;
    let title = browser.remote.title;
    let grip = Point::new(title.x0 + 20.0, f64::midpoint(title.y0, title.y1));
    assert!(browser.remote_press(grip, 0));
    browser.pointer_moved(grip.x - 200.0, -500.0, 0);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    assert!((browser.remote.panel.y0 - top).abs() < 0.5, "held below the strip");
    browser.pointer_released();
    let outputs = browser.take_outputs();
    let Some(Output::Setting(SettingChange::Remote(place))) = outputs.last() else { panic!("{outputs:?}") };
    assert_eq!((place.mode, f64::from(place.y)), (RemoteMode::Floating, top.floor()));
    let dock = rect_of(&browser, Button::Dock);
    browser.remote_press(middle(dock), 0);
    assert!(matches!(browser.take_outputs().as_slice(), [Output::Setting(SettingChange::Remote(RemotePlace { mode: RemoteMode::Docked, .. }))]));
}
