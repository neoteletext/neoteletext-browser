//! The painter's tests: a frame that changed nothing is not drawn, a
//! frame drawn in part matches one drawn whole, and a broken page image
//! is refused.

use super::*;
use crate::text::small_capitals;

use decoder::Engine;
use interface_core::browser::{Browser, Frame as WindowFrame, TabInfo};
use interface_core::canvas::Canvas;
use interface_core::channel::{Channel, PageInfo, Status, page_index};

const WIDTH: u16 = 720;
const HEIGHT: u16 = 450;
const CHANNEL: u32 = 3;
/// Long after any transition, in milliseconds.
const SETTLED: u64 = 10_000;
const FRONT: PageKey = PageKey { channel: CHANNEL, page: 0x100 };

/// A sample page as the engine renders it.
struct Sample {
    page: u16,
    title: String,
    palette: Vec<[u8; 3]>,
    screen: u8,
    indices: Vec<u8>,
}

/// A channel of sample pages, decoded by the teletext engine.
fn pages() -> Vec<Sample> {
    let mut engine = Engine::new();
    let mut arrived = engine.feed(&decoder::sample::tti(&[0x100, 0x101, 0x102, 0x103], &["Headlines", "World", "Sport", "Weather"]));
    arrived.extend(engine.finish());
    arrived
        .into_iter()
        .map(|(page, title)| {
            let rendered = engine.render(page).unwrap();
            Sample { page, title, palette: rendered.palette.clone(), screen: rendered.screen, indices: rendered.indices.clone() }
        })
        .collect()
}

fn browser(painter: &mut Painter, pages: &[Sample]) -> Browser {
    let mut channel = Channel::empty();
    channel.channel = CHANNEL;
    for sample in pages {
        channel.pages[page_index(sample.page).unwrap()] = Some(PageInfo { title: sample.title.clone(), status: Status::Loaded });
        painter.set_page(PageKey { channel: CHANNEL, page: sample.page }, &sample.palette, sample.screen, &sample.indices).unwrap();
    }
    let mut browser = Browser::new(WindowFrame { left_inset: 0.0, window_buttons: true });
    browser.set_tabs(
        vec![TabInfo { title: "sample".into(), colour: 0, channel: CHANNEL, muted: false, private: false, address: "https://example.invalid/".into() }],
        0,
    );
    browser.set_channel(channel);
    // These tests draw the wall of pages; a new window shows the page
    // alone (decision 126).
    browser.wall_mut().toggle_single(0);
    browser
}

/// The window as `browser` draws it at `now`, by `painter`.
fn frame(painter: &mut Painter, browser: &mut Browser, now: u64) -> (Vec<DeviceBox>, Vec<u32>) {
    painter.begin(WIDTH, HEIGHT, 1.0);
    browser.draw(painter, f64::from(WIDTH), f64::from(HEIGHT), now);
    let damage = painter.render();
    let mut words = vec![0; usize::from(WIDTH).saturating_mul(usize::from(HEIGHT))];
    painter.copy_to(&mut words, &damage, 0);
    (damage.boxes().to_vec(), words)
}

fn covered(boxes: &[DeviceBox]) -> u32 {
    boxes.iter().map(|area| area.area()).sum()
}

/// The same window drawn whole, every pixel, by another painter.
fn fresh(pages: &[Sample], now: u64, change: impl Fn(&mut Painter, &mut Browser)) -> Vec<u32> {
    let mut painter = Painter::new(Family::Roboto);
    let mut browser = browser(&mut painter, pages);
    // The first frame reports which pages are on screen.
    frame(&mut painter, &mut browser, SETTLED);
    change(&mut painter, &mut browser);
    painter.redraw_whole();
    frame(&mut painter, &mut browser, now).1
}

fn differing(left: &[u32], right: &[u32]) -> usize {
    left.iter().zip(right).filter(|(a, b)| a != b).count()
}

#[test]
fn a_frame_that_changed_nothing_is_not_drawn() {
    let pages = pages();
    let mut painter = Painter::new(Family::Roboto);
    let mut browser = browser(&mut painter, &pages);
    assert_eq!(frame(&mut painter, &mut browser, SETTLED).0, [DeviceBox::whole(WIDTH, HEIGHT)], "the first frame is whole");
    frame(&mut painter, &mut browser, SETTLED);
    assert_eq!(frame(&mut painter, &mut browser, SETTLED).0, []);
    painter.redraw_whole();
    assert_eq!(frame(&mut painter, &mut browser, SETTLED).0, [DeviceBox::whole(WIDTH, HEIGHT)], "asked to, every pixel");
}

#[test]
fn a_page_whose_dots_changed_redraws_only_them_and_matches_a_whole_frame() {
    let pages = pages();
    let mut painter = Painter::new(Family::Roboto);
    let mut browser = browser(&mut painter, &pages);
    frame(&mut painter, &mut browser, SETTLED);
    frame(&mut painter, &mut browser, SETTLED);
    // As a clock's seconds tick in the header: a few dots of one row.
    let change = |painter: &mut Painter, _: &mut Browser| {
        let front = &pages[0];
        let mut ticked = front.indices.clone();
        let row = usize::from(PAGE_WIDTH) * 5;
        for dot in &mut ticked[row + 400..row + 412] {
            *dot = if *dot == 7 { 0 } else { 7 };
        }
        painter.set_page(FRONT, &front.palette, front.screen, &ticked).unwrap();
    };
    change(&mut painter, &mut browser);
    let (damage, words) = frame(&mut painter, &mut browser, SETTLED);
    assert!(!damage.is_empty() && covered(&damage) < 200, "only the dots that changed: {damage:?}");
    assert_eq!(differing(&words, &fresh(&pages, SETTLED, change)), 0);
}

#[test]
fn a_hover_redraws_only_where_it_shows_and_matches_a_whole_frame() {
    let pages = pages();
    let mut painter = Painter::new(Family::Roboto);
    let mut browser = browser(&mut painter, &pages);
    frame(&mut painter, &mut browser, SETTLED);
    let shown: Vec<_> = browser.wall().shown().collect();
    assert!(!shown.is_empty(), "pages on the wall");
    let (_, rect) = shown[shown.len() - 1];
    let (x, y) = (f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    browser.pointer_moved(x, y, SETTLED);
    // The hover fades in; long after, it shows.
    let (damage, words) = frame(&mut painter, &mut browser, SETTLED * 2);
    assert!(!damage.is_empty() && covered(&damage) < u32::from(WIDTH) * u32::from(HEIGHT) / 2, "a part of the window: {damage:?}");
    let expected = fresh(&pages, SETTLED * 2, |_, browser| {
        browser.pointer_moved(x, y, SETTLED);
    });
    assert_eq!(differing(&words, &expected), 0);
}

#[test]
fn a_surface_that_holds_the_last_frame_takes_only_the_damage() {
    let mut painter = Painter::new(Family::Roboto);
    painter.begin(4, 3, 1.0);
    painter.fill(Rect { x0: 0.0, y0: 0.0, x1: 4.0, y1: 3.0 }, 0.0, Colour { red: 1, green: 2, blue: 3 });
    let whole = painter.render();
    let mut words = vec![0; 12];
    painter.copy_to(&mut words, &whole, 1);
    assert!(words.iter().all(|&word| word == 0x0001_0203));
    let mut held = vec![9; 12];
    let mut damage = Damage::default();
    damage.add(DeviceBox { x0: 1, y0: 1, x1: 3, y1: 2 });
    painter.copy_to(&mut held, &damage, 1);
    assert_eq!(held, [9, 9, 9, 9, 9, 0x0001_0203, 0x0001_0203, 9, 9, 9, 9, 9]);
    painter.copy_to(&mut held, &damage, 0);
    assert_eq!(held, words, "a fresh surface takes every pixel");
}

#[test]
fn small_capitals_mark_what_was_lower_case() {
    let (text, small) = small_capitals("Page size ß");
    assert_eq!(text, "PAGE SIZE SS");
    assert_eq!(small, [1..4, 5..9, 10..12]);
}

#[test]
fn a_page_with_an_index_outside_its_palette_is_refused() {
    let mut painter = Painter::new(Family::Roboto);
    let key = PageKey { channel: 1, page: 0x100 };
    let mut indices = vec![0; PAGE_DOTS];
    assert_eq!(painter.set_page(key, &[[0, 0, 0]], 0, &indices[1..]), Err(PageError::Size));
    assert_eq!(painter.set_page(key, &[[0, 0, 0]], 1, &indices), Err(PageError::Index), "the screen colour");
    indices[7] = 1;
    assert_eq!(painter.set_page(key, &[[0, 0, 0]], 0, &indices), Err(PageError::Index));
    indices[7] = 0;
    assert_eq!(painter.set_page(key, &[[9, 8, 7]], 0, &indices), Ok(()));
    assert_eq!(painter.screen(key), Some(Colour { red: 9, green: 8, blue: 7 }));
}
