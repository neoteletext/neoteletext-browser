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

/// The frame that ends the remote's slide is drawn even when the slide's
/// time ran out between two frames: one drawn part of the way owes the
/// next. Before, a frame drawn just short of the end was the last, and the
/// body stayed a little short of narrowed until something else drew it.
#[test]
fn a_slide_drawn_short_of_its_end_asks_for_the_frame_that_ends_it() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    browser.key(Key::Text("r"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
    let short = SLIDE_MILLISECONDS - 1;
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, short);
    let after = SLIDE_MILLISECONDS + 1;
    assert!(browser.remote_animating(after), "the slide's last frame is still to be drawn");
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, after);
    assert!(!browser.remote_animating(after), "at rest");
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

/// REQ-1373: while keys go to a channel, each of the remote's digits,
/// Fastext colours, arrows, OK and page keys goes to the channel as KEY, a
/// press then a release, with the keyboard's same key: a digit its
/// character and text, the rest their functional codes from the input
/// protocol's table of names (DOC-1936), the names the interface gives the
/// keyboard's own keys. The browser's own buttons still act. On the code
/// before, every remote key did the browser's action and none reached the
/// channel.
#[cfg(feature = "interaction")]
#[test]
fn the_remotes_keys_go_to_the_channel_as_the_keyboards() {
    use crate::browser::typing::TypingView;
    use crate::canvas::PageKey;
    use crate::channel::{Channel, PageInfo, Status};
    use crate::input::Intent;
    use neoteletext_input::event::{Action, KeyCode};
    use neoteletext_input::keys::code;
    const KEY: PageKey = PageKey { channel: 4, page: 0x100 };
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    let mut channel = Channel::empty();
    channel.channel = KEY.channel;
    channel.pages[0] = Some(PageInfo { title: "Terminal".into(), status: Status::Loaded });
    browser.set_channel(channel);
    browser.set_remote(RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    browser.wall.zoom(KEY, 0);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 10_000);
    browser.set_typing(TypingView { channel: KEY.channel, page: KEY.page, on: true, secure: false, refused: false, cursor: None, round_trip_us: None }, 10_000);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 10_000);
    browser.take_outputs();
    let functional = |name: &str| KeyCode::Functional(code(name).unwrap());
    let expected = [
        (Button::Digit('7'), KeyCode::Character('7'), Some("7")),
        (Button::Digit('0'), KeyCode::Character('0'), Some("0")),
        (Button::Control(PageControl::Fastext(FastextKey::Red)), functional("ColorF0Red"), None),
        (Button::Control(PageControl::Fastext(FastextKey::Green)), functional("ColorF1Green"), None),
        (Button::Control(PageControl::Fastext(FastextKey::Yellow)), functional("ColorF2Yellow"), None),
        (Button::Control(PageControl::Fastext(FastextKey::Cyan)), functional("ColorF3Blue"), None),
        (Button::Arrow(true), functional("ArrowUp"), None),
        (Button::Arrow(false), functional("ArrowDown"), None),
        (Button::Control(PageControl::SubPage(false)), functional("ArrowLeft"), None),
        (Button::Control(PageControl::SubPage(true)), functional("ArrowRight"), None),
        (Button::Ok, functional("Enter"), None),
        (Button::PageUp, functional("PageUp"), None),
        (Button::PageDown, functional("PageDown"), None),
    ];
    for (button, key, text) in expected {
        assert!(browser.remote_press(middle(rect_of(&browser, button)), 10_000));
        let sent: Vec<Intent> =
            browser.take_outputs().into_iter().filter_map(|output| if let Output::Terminal { intent, .. } = output { Some(intent) } else { None }).collect();
        assert_eq!(
            sent,
            [
                Intent::Key { action: Action::Press, key, mods: 0, text: text.map(alloc::string::String::from), base: None },
                Intent::Key { action: Action::Release, key, mods: 0, text: None, base: None }
            ],
            "{button:?}"
        );
    }
    assert!(browser.digits.is_empty(), "no digit was the browser's");
    // Reveal is the browser's own, keys going or not.
    browser.remote_press(middle(rect_of(&browser, Button::Control(PageControl::Reveal))), 10_000);
    assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::Reveal)]);
}

/// REQ-894 (decision 112 item 4): the remote's sub-page control, the
/// control the page area's had before decision 126, shows how many
/// sub-pages there are and which is shown, to the standard's 8,190; steps
/// with its arrows; and on a broadcast, whose sub-pages rotate, returns to
/// the latest when its count is pressed. A live channel's has no return
/// (DEC-1542).
#[test]
fn the_remotes_sub_page_control_counts_to_the_standards_maximum_and_returns_to_the_latest() {
    use crate::canvas::PageKey;
    use crate::channel::{Channel, PageInfo, Status};
    use crate::controls::{PageControls, Rotation};
    const KEY: PageKey = PageKey { channel: 4, page: 0x100 };
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    let mut channel = Channel::empty();
    channel.channel = KEY.channel;
    channel.pages[0] = Some(PageInfo { title: "Rotating".into(), status: Status::Loaded });
    browser.set_channel(channel);
    browser.set_remote(RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    browser.wall.zoom(KEY, 0);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 10_000);
    let last = u16::try_from(neoteletext_core::limits::MAX_SUBPAGES).unwrap();
    let controls = PageControls { page: Some(0x100), subpages: last, chosen: Some(last - 1), rotation: Rotation::AsBroadcast, ..PageControls::default() };
    browser.set_page_controls(controls);
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 10_000);
    let place = alloc::format!("{last}/{last}");
    assert!(canvas.texts.iter().any(|(_, text)| *text == place), "the last of {last}: {:?}", canvas.texts);
    browser.take_outputs();
    browser.remote_press(middle(rect_of(&browser, Button::Control(PageControl::SubPage(false)))), 10_000);
    assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::SubPage(false))]);
    browser.remote_press(middle(rect_of(&browser, Button::Latest)), 10_000);
    assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::Latest)], "back to the latest");
    browser.set_page_controls(PageControls { rotation: Rotation::Never, ..controls });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 10_000);
    assert!(!browser.remote.buttons.iter().any(|(button, _)| *button == Button::Latest), "a live channel's has no return");
}
