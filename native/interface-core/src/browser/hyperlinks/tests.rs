use super::*;
use crate::browser::{Frame, StripState, TabInfo};
use crate::channel::{Channel, PAGES, PageInfo, Status};
use crate::geometry::Point;
use crate::links::LinkRun;
use crate::test_canvas::Recorder;
use alloc::string::String;
use alloc::vec;

const WIDTH: f64 = 1440.0;
const HEIGHT: f64 = 900.0;
/// After the zoom's 300 milliseconds: the page is open.
const OPEN: u64 = 1_000;
const VERSION: u32 = 5;

fn channel() -> Channel {
    let mut channel = Channel::empty();
    channel.channel = 7;
    for index in (0..PAGES).filter(|index| index % 3 != 2) {
        channel.pages[index] = Some(PageInfo { title: format!("Page {index}"), status: Status::Loaded });
    }
    channel
}

fn run(row: u8, column: u8, length: u8, kind: LinkKind) -> LinkRun {
    LinkRun { row, column, length, double: false, light: false, kind }
}

fn page_links(version: u32) -> PageLinks {
    PageLinks {
        version,
        runs: vec![run(5, 10, 3, LinkKind::Page), run(7, 0, 12, LinkKind::Web)],
        controls: Vec::new(),
        pointer_areas: Vec::new(),
        remapped: vec![0x302],
    }
}

/// A browser with its first page zoomed and open, its image at `VERSION`.
fn opened() -> (Browser, Recorder, PageKey, Rect) {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
    browser.set_tabs(vec![TabInfo { title: "one".into(), colour: 0, channel: 7, muted: false, private: false, address: "https://example.invalid/".into() }], 0);
    browser.set_strip(StripState { address: "https://news.invalid/".into(), identity: "news.invalid".into(), encrypted: true, ..StripState::default() });
    browser.set_channel(channel());
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    let (key, _) = browser.wall().shown().next().unwrap();
    browser.wall_mut().zoom(key, 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
    browser.frame_version(key, VERSION);
    let (open, rect) = browser.wall().open().expect("the page is open");
    assert_eq!(open, key);
    browser.take_outputs();
    (browser, canvas, key, rect)
}

fn centre(rect: Rect, column: u8, row: u8) -> Point {
    let (width, height) = (rect.width() / 40.0, rect.height() / 25.0);
    Point::new(rect.x0 + (f64::from(column) + 0.5) * width, rect.y0 + (f64::from(row) + 0.5) * height)
}

fn move_to(browser: &mut Browser, point: Point) {
    browser.pointer_moved(point.x, point.y, OPEN);
}

fn points(browser: &mut Browser) -> Vec<Option<LinkCell>> {
    browser.take_outputs().into_iter().filter_map(|output| if let Output::Point(cell) = output { Some(cell) } else { None }).collect()
}

#[test]
fn the_pointer_on_a_link_is_reported_once_per_link_with_its_shape() {
    let (mut browser, _, key, rect) = opened();
    browser.set_links(key, page_links(VERSION));
    browser.take_outputs();
    move_to(&mut browser, centre(rect, 11, 5));
    assert_eq!(points(&mut browser), [Some(LinkCell { key, version: VERSION, column: 11, row: 5 })]);
    assert_eq!(browser.cursor(), Cursor::Hand);
    move_to(&mut browser, centre(rect, 12, 5));
    assert_eq!(points(&mut browser), [], "the same link");
    move_to(&mut browser, centre(rect, 3, 7));
    assert_eq!(points(&mut browser).len(), 1);
    assert_eq!(browser.cursor(), Cursor::External, "a link that leaves");
    move_to(&mut browser, centre(rect, 30, 12));
    assert_eq!(points(&mut browser), [None]);
    assert_eq!(browser.cursor(), Cursor::Arrow);
}

#[test]
fn a_new_image_under_the_pointer_is_reported_again() {
    // The trusted core keeps a few images' links; the strip must follow
    // the image on screen, which a flashing page renews many times a
    // second.
    let (mut browser, _, key, rect) = opened();
    browser.set_links(key, page_links(VERSION));
    move_to(&mut browser, centre(rect, 11, 5));
    browser.take_outputs();
    browser.frame_version(key, VERSION + 1);
    assert_eq!(points(&mut browser), [None], "the links of the new image have not come yet");
    browser.set_links(key, page_links(VERSION + 1));
    assert_eq!(points(&mut browser), [Some(LinkCell { key, version: VERSION + 1, column: 11, row: 5 })]);
}

#[test]
fn links_are_drawn_only_over_their_own_image() {
    let (mut browser, mut canvas, key, _) = opened();
    let blue = |canvas: &Recorder| canvas.fills.iter().filter(|(_, colour)| *colour == LINK).count();
    browser.set_links(key, page_links(VERSION - 1));
    canvas.fills.clear();
    browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
    assert_eq!(blue(&canvas), 0, "links of an older image are not drawn");
    browser.set_links(key, page_links(VERSION));
    canvas.fills.clear();
    browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
    assert_eq!(blue(&canvas), 2);
}

#[test]
fn a_click_names_the_image_it_was_made_on() {
    let (mut browser, mut canvas, key, rect) = opened();
    browser.set_links(key, page_links(VERSION));
    move_to(&mut browser, centre(rect, 11, 5));
    browser.pointer_pressed(&mut canvas, false, OPEN);
    let clicks: Vec<CellClick> =
        browser.take_outputs().into_iter().filter_map(|output| if let Output::Cell(click) = output { Some(click) } else { None }).collect();
    assert_eq!(clicks, [CellClick { key, version: VERSION, column: 11, row: 5 }]);
}

#[test]
fn tab_moves_between_links_and_enter_follows_the_focused_one() {
    let (mut browser, _, key, _) = opened();
    browser.set_links(key, page_links(VERSION));
    browser.take_outputs();
    let plain = Modifiers::default();
    let first = Some(LinkCell { key, version: VERSION, column: 10, row: 5 });
    let second = Some(LinkCell { key, version: VERSION, column: 0, row: 7 });
    assert!(browser.key(Key::Tab, plain, OPEN));
    assert_eq!(points(&mut browser), [first]);
    browser.key(Key::Tab, plain, OPEN);
    assert_eq!(points(&mut browser), [second]);
    browser.key(Key::Tab, Modifiers { shift: true, ..plain }, OPEN);
    assert_eq!(points(&mut browser), [first]);
    browser.key(Key::Enter, Modifiers { repeat: true, ..plain }, OPEN);
    assert!(browser.take_outputs().is_empty(), "a key held from before follows nothing");
    browser.key(Key::Enter, plain, OPEN);
    assert_eq!(browser.take_outputs(), [Output::Cell(CellClick { key, version: VERSION, column: 10, row: 5 })]);
    assert!(browser.key(Key::Escape, plain, OPEN));
    assert_eq!(points(&mut browser), [None]);
    assert!(browser.wall().zoomed().is_some(), "Escape left the links, not the page");
}

fn prompt(id: u32) -> Prompt {
    Prompt { id, kind: PromptKind::Web, destination: "https://www.example.invalid/".into(), secure: true }
}

#[test]
fn the_question_waits_half_a_second_and_never_takes_the_focus() {
    let (mut browser, mut canvas, _, _) = opened();
    browser.set_prompt(Some(prompt(3)), OPEN);
    browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
    assert!(canvas.texts.iter().any(|(_, text)| text.starts_with("Open https://www.example.invalid/")));
    let go = browser.links.go;
    browser.pointer_moved(f64::midpoint(go.x0, go.x1), f64::midpoint(go.y0, go.y1), OPEN);
    browser.pointer_pressed(&mut canvas, false, OPEN + PROMPT_DELAY_MILLISECONDS - 1);
    assert!(browser.take_outputs().iter().all(|output| !matches!(output, Output::Confirm(_))), "too soon");
    browser.pointer_pressed(&mut canvas, false, OPEN + PROMPT_DELAY_MILLISECONDS);
    assert!(browser.take_outputs().contains(&Output::Confirm(3)));
    // Enter answers only once the focus was moved to a button.
    let later = OPEN + PROMPT_DELAY_MILLISECONDS;
    browser.key(Key::Enter, Modifiers::default(), later);
    assert!(browser.take_outputs().is_empty(), "the question does not take the focus by itself");
    browser.key(Key::Tab, Modifiers::default(), later);
    browser.key(Key::Enter, Modifiers { repeat: true, ..Modifiers::default() }, later);
    assert!(browser.take_outputs().is_empty(), "a key held from before");
    browser.key(Key::Enter, Modifiers::default(), later);
    assert_eq!(browser.take_outputs(), [Output::Confirm(3)]);
    browser.key(Key::Escape, Modifiers::default(), later);
    assert_eq!(browser.take_outputs(), [Output::Cancel(3)]);
}

#[test]
fn the_strip_shows_where_a_link_leads_in_place_of_the_address() {
    let (mut browser, mut canvas, _, _) = opened();
    browser.set_pointing(Some(Pointing { kind: LinkKind::Page, text: "Page 302".into() }));
    canvas.texts.clear();
    browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
    let texts: Vec<&String> = canvas.texts.iter().map(|(_, text)| text).collect();
    assert!(texts.iter().any(|text| text.as_str() == "\u{2192} Page 302"));
    assert!(!texts.iter().any(|text| text.contains("news.invalid")), "the address gives way");
}

#[test]
fn a_number_the_page_or_channel_links_goes_to_the_trusted_core() {
    let (mut browser, _, key, _) = opened();
    browser.set_links(key, page_links(VERSION));
    browser.set_aliases(7, vec![0x500]);
    browser.take_outputs();
    browser.key(Key::Text("302"), Modifiers::default(), OPEN);
    assert_eq!(browser.take_outputs(), [Output::Typed { number: 0x302, opened: Some((key, VERSION)) }]);
    browser.key(Key::Text("500"), Modifiers::default(), OPEN);
    assert_eq!(browser.take_outputs(), [Output::Typed { number: 0x500, opened: Some((key, VERSION)) }]);
    browser.key(Key::Text("401"), Modifiers::default(), OPEN);
    assert!(!browser.take_outputs().iter().any(|output| matches!(output, Output::Typed { .. })), "an ordinary number chooses its page");
}

#[test]
fn a_page_the_trusted_core_opens_is_zoomed_now_or_when_it_arrives() {
    let (mut browser, _, _, _) = opened();
    let listed = PageKey { channel: 7, page: 0x301 };
    browser.show(listed, OPEN);
    assert_eq!(browser.wall().zoomed(), Some(listed));
    // Page 303 is not listed yet (every third page is missing).
    let missing = PageKey { channel: 7, page: 0x303 };
    browser.show(missing, OPEN);
    assert_eq!(browser.wall().zoomed(), Some(listed));
    browser.add_page(7, 0x303, "Arrived".into(), OPEN);
    assert_eq!(browser.wall().zoomed(), Some(missing));
}

fn lits(browser: &mut Browser) -> Vec<Option<Lit>> {
    browser.take_outputs().into_iter().filter_map(|output| if let Output::Lit(lit) = output { Some(lit) } else { None }).collect()
}

/// INPUT.md section 9.7: while keys do not go to the channel, a link under the pointer takes
/// the hover look and a press its pressed look; a control takes neither,
/// nor the hand, since a click on it would not reach the channel.
#[test]
fn a_link_lights_under_the_pointer_and_a_control_only_while_keys_go() {
    let (mut browser, mut canvas, key, rect) = opened();
    let control = Area { row: 20, column: 1, width: 9, height: 1 };
    browser.set_links(key, PageLinks { controls: vec![control], pointer_areas: Vec::new(), ..page_links(VERSION) });
    browser.take_outputs();
    move_to(&mut browser, centre(rect, 11, 5));
    let link = Area { row: 5, column: 10, width: 3, height: 1 };
    assert_eq!(lits(&mut browser), [Some(Lit { key, area: link, pressed: false })]);
    browser.pointer_pressed(&mut canvas, false, OPEN);
    assert!(lits(&mut browser).contains(&Some(Lit { key, area: link, pressed: true })));
    browser.pointer_released();
    assert_eq!(lits(&mut browser), [Some(Lit { key, area: link, pressed: false })]);
    move_to(&mut browser, centre(rect, 4, 20));
    assert_eq!(lits(&mut browser), [None], "a control while keys do not go");
    assert_eq!(browser.cursor(), Cursor::Arrow);
}

/// INPUT.md section 9.7: while keys go to the channel, a control shows the hand and
/// lights, hovered and then pressed while the button is held on it, and
/// a new image with the same links keeps the look rather than putting it
/// out while its links follow.
#[cfg(feature = "interaction")]
#[test]
fn a_control_lights_and_shows_the_hand_while_keys_go() {
    use crate::browser::typing::TypingView;
    let (mut browser, mut canvas, key, rect) = opened();
    let control = Area { row: 20, column: 1, width: 9, height: 2 };
    browser.set_links(key, PageLinks { controls: vec![control], pointer_areas: Vec::new(), ..page_links(VERSION) });
    let typing =
        TypingView { channel: key.channel, page: key.page, on: true, secure: false, refused: false, cursor: None, acked: 0, round_trip_ms: None, show: None };
    browser.set_typing(typing, OPEN);
    browser.take_outputs();
    move_to(&mut browser, centre(rect, 9, 21));
    assert_eq!(lits(&mut browser), [Some(Lit { key, area: control, pressed: false })], "the second row of the rectangle");
    assert_eq!(browser.cursor(), Cursor::Hand);
    browser.pointer_pressed(&mut canvas, false, OPEN);
    assert!(lits(&mut browser).contains(&Some(Lit { key, area: control, pressed: true })));
    // The look's own image arrives, its links after it.
    browser.frame_version(key, VERSION + 1);
    move_to(&mut browser, centre(rect, 2, 20));
    assert_eq!(lits(&mut browser), [], "still pressed, on the same control");
    browser.pointer_released();
    assert_eq!(lits(&mut browser), [Some(Lit { key, area: control, pressed: false })]);
    move_to(&mut browser, centre(rect, 11, 5));
    assert_eq!(lits(&mut browser), [None], "a link's click is the channel's while keys go");
    assert_eq!(browser.cursor(), Cursor::Arrow);
}

/// INPUT.md section 5.4 (decision E.129): while keys go to the channel it is
/// told as the pointer enters and leaves each control, the leave before the
/// enter from one to the next, with the control's first cell.
#[cfg(feature = "interaction")]
#[test]
fn the_channel_is_told_as_the_pointer_enters_and_leaves_controls() {
    use crate::browser::typing::TypingView;
    use crate::input::Intent;
    let (mut browser, _, key, rect) = opened();
    let first = Area { row: 20, column: 1, width: 9, height: 1 };
    let second = Area { row: 20, column: 10, width: 9, height: 1 };
    browser.set_links(key, PageLinks { controls: vec![first, second], ..page_links(VERSION) });
    let typing =
        TypingView { channel: key.channel, page: key.page, on: true, secure: false, refused: false, cursor: None, acked: 0, round_trip_ms: None, show: None };
    browser.set_typing(typing, OPEN);
    browser.take_outputs();
    let hovers = |browser: &mut Browser| -> Vec<Intent> {
        browser
            .take_outputs()
            .into_iter()
            .filter_map(|output| match output {
                Output::Terminal { intent: intent @ Intent::Hover { .. }, .. } => Some(intent),
                _ => None,
            })
            .collect()
    };
    move_to(&mut browser, centre(rect, 3, 20));
    assert_eq!(hovers(&mut browser), [Intent::Hover { leave: false, column: 1, row: 20 }]);
    move_to(&mut browser, centre(rect, 5, 20));
    assert_eq!(hovers(&mut browser), [], "within the same control");
    move_to(&mut browser, centre(rect, 12, 20));
    assert_eq!(hovers(&mut browser), [Intent::Hover { leave: true, column: 1, row: 20 }, Intent::Hover { leave: false, column: 10, row: 20 }]);
    move_to(&mut browser, centre(rect, 30, 12));
    assert_eq!(hovers(&mut browser), [Intent::Hover { leave: true, column: 10, row: 20 }]);
}
