use super::*;
use alloc::vec::Vec;

use super::hover::{HOVER_GROWTH, HOVER_MILLISECONDS};
use super::zoom::{SLIDE_MILLISECONDS, WHEEL_PER_PAGE, ZOOM_MILLISECONDS};
use crate::canvas::{CELL_HEIGHT, CELL_WIDTH, PAGE_HEIGHT, PAGE_WIDTH, PageKey, image_within};
use crate::geometry::{Point, Rect};
use crate::size::stepped_width;
use crate::test_canvas::Recorder;

const PANEL: Rect = Rect::new(0.0, 0.0, 1000.0, 700.0);

fn pages(count: u16) -> Vec<WallPage<'static>> {
    (0..count).map(|offset| WallPage { key: PageKey { channel: 1, page: 0x100_u16.saturating_add(offset) }, title: "Title", updated: None }).collect()
}

#[test]
fn only_pages_on_screen_are_visible() {
    let mut wall = Wall::new();
    let mut canvas = Recorder::new();
    wall.draw(&mut canvas, PANEL, &pages(24), None, 0);
    let visible = wall.visible();
    assert!(!visible.is_empty() && visible.len() < 24, "{} of 24 visible", visible.len());
    assert_eq!(visible[0], PageKey { channel: 1, page: 0x100 });
}

#[test]
fn a_revealed_page_is_drawn() {
    let mut wall = Wall::new();
    let mut canvas = Recorder::new();
    let pages = pages(24);
    wall.draw(&mut canvas, PANEL, &pages, None, 0);
    let last = pages[23].key;
    assert!(!wall.visible().contains(&last));
    wall.reveal(last);
    wall.draw(&mut canvas, PANEL, &pages, None, 0);
    assert!(wall.visible().contains(&last));
}

#[test]
fn a_click_finds_its_cell() {
    let mut wall = Wall::new();
    let mut canvas = Recorder::new();
    wall.draw(&mut canvas, PANEL, &pages(24), None, 0);
    let (key, rect) = canvas.pages.iter().copied().find(|(key, _)| key.page == 0x101).unwrap();
    // The middle of column 10, row 3.
    let point = Point::new(
        rect.x0 + (10.5 * f64::from(CELL_WIDTH)) / f64::from(PAGE_WIDTH) * rect.width(),
        rect.y0 + (3.5 * f64::from(CELL_HEIGHT)) / f64::from(PAGE_HEIGHT) * rect.height(),
    );
    assert_eq!(wall.cell_at(point), Some(CellClick { key, version: 0, column: 10, row: 3 }));
    assert_eq!(wall.cell_at(Point::new(rect.x0 - 2.0, rect.y0)), None, "the gap between pages");
}

/// Where `key` was drawn last in the frame just drawn: the topmost.
fn last_rect(canvas: &Recorder, key: PageKey) -> Rect {
    canvas.pages.iter().rev().find(|(drawn, _)| *drawn == key).map(|&(_, rect)| rect).unwrap()
}

fn draw_at(wall: &mut Wall, pages: &[WallPage<'_>], now: u64) -> Recorder {
    let mut canvas = Recorder::new();
    wall.draw(&mut canvas, PANEL, pages, None, now);
    canvas
}

#[test]
fn a_zoomed_page_grows_from_its_place_to_fill_the_area() {
    let mut wall = Wall::new();
    let pages = pages(24);
    // The second page: on screen, in the first row.
    let key = pages[1].key;
    let slot = last_rect(&draw_at(&mut wall, &pages, 0), key);
    wall.zoom(key, 0);
    // Halfway through, the page is between its slot and its zoomed place.
    let middle = last_rect(&draw_at(&mut wall, &pages, ZOOM_MILLISECONDS / 2), key);
    let full = image_within(wall.zoomed_rect());
    assert!(middle.width() > slot.width() && middle.width() < full.width(), "{slot:?} {middle:?} {full:?}");
    assert!(wall.animating(ZOOM_MILLISECONDS / 2));
    // At the end it fills the area alone.
    let end = draw_at(&mut wall, &pages, ZOOM_MILLISECONDS);
    assert_eq!(wall.visible(), [key], "only the zoomed page is shown");
    assert_eq!(last_rect(&end, key), full);
    assert!(!wall.animating(ZOOM_MILLISECONDS));
}

#[test]
fn the_close_button_zooms_back_to_the_wall() {
    let mut wall = Wall::new();
    let pages = pages(24);
    // The second page: on screen, in the first row.
    let key = pages[1].key;
    let slot = last_rect(&draw_at(&mut wall, &pages, 0), key);
    wall.zoom(key, 0);
    draw_at(&mut wall, &pages, ZOOM_MILLISECONDS);
    let close = wall.close;
    let now = 2 * ZOOM_MILLISECONDS;
    assert!(wall.pointer_pressed(Point::new(f64::midpoint(close.x0, close.x1), f64::midpoint(close.y0, close.y1)), now));
    assert_eq!(wall.zoomed(), None);
    // On its way back, the wall shows again behind it.
    draw_at(&mut wall, &pages, now + ZOOM_MILLISECONDS / 2);
    assert!(wall.visible().len() > 1);
    let back = draw_at(&mut wall, &pages, now + ZOOM_MILLISECONDS);
    assert_eq!(last_rect(&back, key), slot, "home in its slot");
    assert!(!wall.animating(now + ZOOM_MILLISECONDS));
}

#[test]
fn escape_part_way_reverses_without_a_jump() {
    let mut wall = Wall::new();
    let pages = pages(24);
    // The second page: on screen, in the first row.
    let key = pages[1].key;
    draw_at(&mut wall, &pages, 0);
    wall.zoom(key, 0);
    let part = ZOOM_MILLISECONDS / 3;
    let before = last_rect(&draw_at(&mut wall, &pages, part), key);
    assert!(wall.unzoom(part));
    let after = last_rect(&draw_at(&mut wall, &pages, part), key);
    assert!((before.width() - after.width()).abs() < 1e-6, "{before:?} then {after:?}");
    let later = last_rect(&draw_at(&mut wall, &pages, part + ZOOM_MILLISECONDS / 3), key);
    assert!(later.width() < after.width(), "heading back");
}

#[test]
fn a_hovered_page_pops_out_and_settles_back() {
    let mut wall = Wall::new();
    let pages = pages(24);
    let key = pages[1].key;
    let slot = last_rect(&draw_at(&mut wall, &pages, 0), key);
    let centre = Point::new(f64::midpoint(slot.x0, slot.x1), f64::midpoint(slot.y0, slot.y1));
    assert!(wall.hover(centre, 0));
    assert!(wall.over_page());
    let popped = last_rect(&draw_at(&mut wall, &pages, HOVER_MILLISECONDS), key);
    assert!(popped.width() > slot.width() * (1.0 + HOVER_GROWTH / 2.0), "{slot:?} {popped:?}");
    assert!((f64::midpoint(popped.x0, popped.x1) - centre.x).abs() < 1e-6, "grows about its centre");
    assert_eq!(wall.visible().iter().filter(|&&visible| visible == key).count(), 1, "still one page");
    // Away from every page, it settles back.
    assert!(wall.hover(Point::new(-1.0, -1.0), HOVER_MILLISECONDS));
    assert!(!wall.over_page());
    let settled = last_rect(&draw_at(&mut wall, &pages, 3 * HOVER_MILLISECONDS), key);
    assert_eq!(settled, slot);
}

#[test]
fn the_slider_end_is_the_zoomed_state() {
    let mut wall = Wall::new();
    let pages = pages(24);
    draw_at(&mut wall, &pages, 0);
    // Wheeled all the way right: the first page on screen zooms.
    wall.wheel_bar(-10_000.0, 0);
    assert_eq!(wall.zoomed(), Some(pages[0].key));
    // A little way back from the end: the zoom closes.
    wall.wheel_bar(30.0, 1_000);
    assert_eq!(wall.zoomed(), None);
}

#[test]
fn a_click_zoom_moves_the_thumb_to_the_end_and_escape_puts_it_back() {
    let mut wall = Wall::new();
    let pages = pages(24);
    draw_at(&mut wall, &pages, 0);
    let before = wall.width;
    wall.zoom(pages[1].key, 0);
    assert!((wall.width - wall.max).abs() < f32::EPSILON);
    assert!(wall.unzoom(10));
    assert!((wall.width - before).abs() < f32::EPSILON);
}

#[test]
fn the_wheel_steps_through_pages_and_each_snaps_into_place() {
    let mut wall = Wall::new();
    let pages = pages(24);
    draw_at(&mut wall, &pages, 0);
    wall.zoom(pages[1].key, 0);
    let start = 1_000;
    draw_at(&mut wall, &pages, start);
    let full = image_within(wall.zoomed_rect());
    // Towards the user: the next page.
    assert_eq!(wall.wheel_pages(WHEEL_PER_PAGE, start), Some(pages[2].key));
    // Part-way, both pages are on the strip, the new one rising.
    let middle = draw_at(&mut wall, &pages, start + SLIDE_MILLISECONDS / 3);
    assert_eq!(wall.visible(), [pages[1].key, pages[2].key]);
    assert!(last_rect(&middle, pages[2].key).y0 > full.y0);
    // At the end, the new page alone, exactly in place.
    let rested = draw_at(&mut wall, &pages, start + SLIDE_MILLISECONDS);
    assert_eq!(wall.visible(), [pages[2].key]);
    assert_eq!(last_rect(&rested, pages[2].key), full);
    // Away from the user, back; and no page before the first.
    assert_eq!(wall.wheel_pages(-WHEEL_PER_PAGE, start + 1_000), Some(pages[1].key));
    assert_eq!(wall.wheel_pages(-WHEEL_PER_PAGE, start + 1_000), Some(pages[0].key));
    assert_eq!(wall.wheel_pages(-WHEEL_PER_PAGE, start + 1_000), None);
    // Small movements add up to a step.
    assert_eq!(wall.wheel_pages(WHEEL_PER_PAGE / 2.0, start + 2_000), None);
    assert_eq!(wall.wheel_pages(WHEEL_PER_PAGE / 2.0, start + 2_000), Some(pages[1].key));
}

#[test]
fn every_notch_of_a_mouse_wheel_moves_a_page() {
    // A slow notch on macOS, as measured: a tenth of a line.
    const SLOW_NOTCH: f64 = 0.1;
    let mut wall = Wall::new();
    let pages = pages(24);
    draw_at(&mut wall, &pages, 0);
    wall.zoom(pages[1].key, 0);
    assert_eq!(wall.wheel_notch(SLOW_NOTCH, 1_000), Some(pages[2].key));
    // A quick flick's sped-up events during that slide move no further.
    assert_eq!(wall.wheel_notch(0.9, 1_020), None);
    assert_eq!(wall.wheel_notch(2.8, 1_080), None);
    // The next notch after the slide moves again, either way.
    assert_eq!(wall.wheel_notch(-SLOW_NOTCH, 1_000 + SLIDE_MILLISECONDS), Some(pages[1].key));
    assert_eq!(wall.wheel_notch(-SLOW_NOTCH, 1_000 + 2 * SLIDE_MILLISECONDS), Some(pages[0].key));
    // At the first page there is nowhere to go.
    assert_eq!(wall.wheel_notch(-SLOW_NOTCH, 1_000 + 3 * SLIDE_MILLISECONDS), None);
}

#[test]
fn a_step_during_a_slide_carries_on_without_a_jump() {
    let mut wall = Wall::new();
    let pages = pages(24);
    draw_at(&mut wall, &pages, 0);
    wall.zoom(pages[1].key, 0);
    wall.step_zoom(1, 1_000);
    let part = 1_000 + SLIDE_MILLISECONDS / 4;
    let offset = wall.slide.value(part);
    wall.step_zoom(1, part);
    assert!((wall.slide.value(part) - (offset + 1.0)).abs() < 1e-6, "the strip stays where it was");
    assert_eq!(wall.zoomed(), Some(pages[3].key));
}

#[test]
fn a_zoomed_page_that_leaves_the_wall_ends_the_zoom() {
    let mut wall = Wall::new();
    wall.zoom(PageKey { channel: 9, page: 0x500 }, 0);
    draw_at(&mut wall, &pages(4), 0);
    assert_eq!(wall.zoomed(), None);
}

#[test]
fn the_buttons_step_the_page_size() {
    let mut wall = Wall::new();
    let mut canvas = Recorder::new();
    wall.draw(&mut canvas, PANEL, &pages(4), None, 0);
    let before = wall.width;
    let plus = wall.plus;
    assert!(wall.pointer_pressed(Point::new(f64::midpoint(plus.x0, plus.x1), f64::midpoint(plus.y0, plus.y1)), 0));
    assert!((wall.width - stepped_width(before, 1)).abs() < f32::EPSILON);
}
