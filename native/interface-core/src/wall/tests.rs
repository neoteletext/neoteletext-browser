use super::*;
use alloc::vec::Vec;

use super::hover::{HOVER_GROWTH, HOVER_MILLISECONDS};
use super::zoom::{SLIDE_MILLISECONDS, WHEEL_PER_PAGE, ZOOM_MILLISECONDS};
use crate::canvas::{CELL_HEIGHT, CELL_WIDTH, COLUMNS, PAGE_HEIGHT, PAGE_WIDTH, PageKey, PageShape, SCREEN_SCALE, image_within};
use crate::geometry::{Point, Rect};
use crate::size::{MAX_WIDTH, MIN_WIDTH, stepped_width};
use crate::test_canvas::Recorder;

const PANEL: Rect = Rect::new(0.0, 0.0, 1000.0, 700.0);

/// A wall showing the wall of pages, which these tests are of; a new one
/// shows the page alone (decision 126).
fn on_wall() -> Wall {
    let mut wall = Wall::new();
    wall.view = View::Wall;
    wall
}

fn pages(count: u16) -> Vec<WallPage<'static>> {
    (0..count).map(|offset| WallPage { key: PageKey { channel: 1, page: 0x100_u16.saturating_add(offset) }, title: "Title", updated: None }).collect()
}

#[test]
fn only_pages_on_screen_are_visible() {
    let mut wall = on_wall();
    let mut canvas = Recorder::new();
    wall.draw(&mut canvas, PANEL, &pages(24), None, 0);
    let visible = wall.visible();
    assert!(!visible.is_empty() && visible.len() < 24, "{} of 24 visible", visible.len());
    assert_eq!(visible[0], PageKey { channel: 1, page: 0x100 });
}

#[test]
fn a_revealed_page_is_drawn() {
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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

/// REQ-754: with the platform's reduced motion, a zoom, a slide to the next
/// page, the page size and a hover end where they go at once, with nothing
/// moving; and once the preference is off they move again. On the code
/// before, the wall had no such preference and every move took its time.
#[test]
fn reduced_motion_ends_every_move_at_once() {
    let mut wall = on_wall();
    wall.set_reduced_motion(true);
    let pages = pages(24);
    let key = pages[1].key;
    draw_at(&mut wall, &pages, 0);
    wall.zoom(key, 0);
    assert!(!wall.animating(0), "nothing moves");
    let end = draw_at(&mut wall, &pages, 0);
    assert_eq!(last_rect(&end, key), image_within(wall.zoomed_rect()), "the page is in its zoomed place at once");
    assert!(wall.step_zoom(1, 1).is_some());
    assert!(!wall.animating(1), "a step does not slide");
    wall.unzoom(2);
    assert!(!wall.animating(2), "nor does the way back");
    wall.set_reduced_motion(false);
    draw_at(&mut wall, &pages, 3);
    wall.zoom(key, 3);
    assert!(wall.animating(4), "without the preference the zoom moves again");
}

#[test]
fn the_close_button_zooms_back_to_the_wall() {
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
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
    let mut wall = on_wall();
    wall.zoom(PageKey { channel: 9, page: 0x500 }, 0);
    draw_at(&mut wall, &pages(4), 0);
    assert_eq!(wall.zoomed(), None);
}

#[test]
fn the_buttons_step_the_page_size() {
    let mut wall = on_wall();
    let mut canvas = Recorder::new();
    wall.draw(&mut canvas, PANEL, &pages(4), None, 0);
    let before = wall.width;
    let plus = wall.plus;
    assert!(wall.pointer_pressed(Point::new(f64::midpoint(plus.x0, plus.x1), f64::midpoint(plus.y0, plus.y1)), 0));
    assert!((wall.width - stepped_width(before, 1)).abs() < f32::EPSILON);
}

// REQ-1559 item 1: however many pages there are, no 129th tile.
#[test]
fn the_wall_never_lays_out_a_129th_tile() {
    let mut wall = on_wall();
    let pages = pages(300);
    let mut canvas = Recorder::new();
    // Tall enough for every tile at the smallest grid size.
    wall.draw(&mut canvas, Rect::new(0.0, 0.0, 4_000.0, 40_000.0), &pages, None, 0);
    assert_eq!(canvas.pages.len(), MAX_WALL_TILES);
    assert_eq!(wall.visible().len(), MAX_WALL_TILES);
}

/// The engine's factor as a number, written out so that a test of the shape
/// does not take it from the code it tests (DEC-2049 item 2).
const STRETCH: f64 = 16.0 / 13.0;

fn same(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

/// Where `wall`, in `shape`, draws its pages in `panel`, topmost last, long
/// after anything moved.
fn images(mut wall: Wall, shape: PageShape, panel: Rect, pages: &[WallPage<'_>]) -> (Wall, Vec<(PageKey, Rect)>) {
    wall.set_shape(shape);
    let mut canvas = Recorder::new();
    wall.draw(&mut canvas, panel, pages, None, 0);
    canvas.pages.clear();
    wall.draw(&mut canvas, panel, pages, None, 10_000);
    (wall, canvas.pages)
}

/// DEC-2049 items 2 and 3: the engine's factor, by which a page of 480 dots
/// becomes the 590 the engine's own scaling makes; raster proportions are
/// left exactly as they were.
#[test]
fn the_correction_is_the_engines_factor() {
    assert!(same(PageShape::Corrected.stretch(), STRETCH));
    assert!(same(libm::floor(f64::from(PAGE_WIDTH) * PageShape::Corrected.stretch()), 590.0));
    assert!(same(PageShape::Raster.stretch(), 1.0));
    assert!(same(PageShape::Corrected.height_per_width() * STRETCH, PageShape::Raster.height_per_width()));
    assert_eq!(PageShape::default(), PageShape::Raster, "off until a viewer turns it on");
}

/// REQ-737 item 2, DEC-1018: with the height setting the size, the page
/// alone keeps its height and is wider by the factor.
#[test]
fn the_page_alone_is_stretched_in_width() {
    let pages = pages(1);
    let (_, off) = images(Wall::new(), PageShape::Raster, PANEL, &pages);
    let (_, on) = images(Wall::new(), PageShape::Corrected, PANEL, &pages);
    assert!(same(off[0].1.height(), on[0].1.height()), "{off:?} {on:?}");
    assert!(same(on[0].1.width(), off[0].1.width() * STRETCH), "{off:?} {on:?}");
    assert!(same(f64::midpoint(on[0].1.x0, on[0].1.x1), f64::midpoint(PANEL.x0, PANEL.x1)), "centred: {on:?}");
}

/// DEC-1018: "fits to the greater of height or width": where the width runs
/// out first the corrected page fills it and is shorter, as its proportions
/// say, never wider than the area.
#[test]
fn the_page_alone_fits_at_the_corrected_proportions() {
    let tall = Rect::new(0.0, 0.0, 500.0, 900.0);
    let pages = pages(1);
    let (_, off) = images(Wall::new(), PageShape::Raster, tall, &pages);
    let (_, on) = images(Wall::new(), PageShape::Corrected, tall, &pages);
    assert!(same(off[0].1.width(), on[0].1.width()), "{off:?} {on:?}");
    assert!(same(on[0].1.height(), off[0].1.height() / STRETCH), "{off:?} {on:?}");
    assert!(same(on[0].1.width() * SCREEN_SCALE, tall.width()), "the area's width, with its screen: {on:?}");
}

/// REQ-737 item 2 on the wall: every page wider by the factor at the same
/// page size, and the same height.
#[test]
fn pages_on_the_wall_are_stretched_in_width() {
    let pages = pages(4);
    let (_, off) = images(on_wall(), PageShape::Raster, PANEL, &pages);
    let (_, on) = images(on_wall(), PageShape::Corrected, PANEL, &pages);
    assert!(same(on[0].1.width(), off[0].1.width() * STRETCH), "{off:?} {on:?}");
    assert!(same(on[0].1.height(), off[0].1.height()), "{off:?} {on:?}");
}

/// REQ-737 item 2 on the wall's list, the slider at its left edge.
#[test]
fn pages_in_the_walls_list_are_stretched_in_width() {
    let listed = || {
        let mut wall = on_wall();
        wall.width = MIN_WIDTH;
        wall.shown_width.set(MIN_WIDTH, 0);
        wall
    };
    let pages = pages(4);
    let (wall, off) = images(listed(), PageShape::Raster, PANEL, &pages);
    assert_eq!(wall.layout_for_tests(), Layout::List);
    let (_, on) = images(listed(), PageShape::Corrected, PANEL, &pages);
    assert!(same(on[0].1.width(), off[0].1.width() * STRETCH), "{off:?} {on:?}");
    assert!(same(on[0].1.height(), off[0].1.height()), "{off:?} {on:?}");
}

/// The largest page size still fits the area once it is stretched, so the
/// slider never lays a page out past the panel.
#[test]
fn the_largest_corrected_page_fits_the_wall() {
    let mut wall = on_wall();
    wall.width = MAX_WIDTH;
    let pages = pages(1);
    let (wall, on) = images(wall, PageShape::Corrected, PANEL, &pages);
    assert!(wall.width < MAX_WIDTH, "the slider's end comes in: {}", wall.width);
    assert!(on[0].1.x0 >= wall.area().x0 && on[0].1.x1 <= wall.area().x1, "{on:?} in {:?}", wall.area());
}

/// REQ-737 item 2 for a page zoomed from the wall.
#[test]
fn a_zoomed_page_is_stretched_in_width() {
    let zoomed = |shape| {
        let mut wall = on_wall();
        wall.set_shape(shape);
        let pages = pages(4);
        wall.draw(&mut Recorder::new(), PANEL, &pages, None, 0);
        wall.zoom(pages[1].key, 0);
        wall.draw(&mut Recorder::new(), PANEL, &pages, None, 0);
        images(wall, shape, PANEL, &pages).1.into_iter().find(|(key, _)| *key == pages[1].key).unwrap().1
    };
    let (off, on) = (zoomed(PageShape::Raster), zoomed(PageShape::Corrected));
    assert!(same(on.width(), off.width() * STRETCH), "{off:?} {on:?}");
    assert!(same(on.height(), off.height()), "{off:?} {on:?}");
}

/// A page popping out under the pointer grows in the shape it was drawn in.
#[test]
fn a_page_popping_out_keeps_its_shape() {
    let pages = pages(4);
    let (mut wall, slots) = images(on_wall(), PageShape::Corrected, PANEL, &pages);
    let (key, slot) = slots[1];
    wall.hover(Point::new(f64::midpoint(slot.x0, slot.x1), f64::midpoint(slot.y0, slot.y1)), 0);
    let canvas = draw_at(&mut wall, &pages, HOVER_MILLISECONDS);
    let popped = last_rect(&canvas, key);
    assert!(popped.width() > slot.width(), "{popped:?} {slot:?}");
    assert!(same(popped.width() / popped.height(), slot.width() / slot.height()), "{popped:?} {slot:?}");
}

/// The middle of a cell of a page drawn at `image`.
fn cell_middle(image: Rect, column: u8, row: u8) -> Point {
    Point::new(
        image.x0 + (f64::from(column) + 0.5) / f64::from(COLUMNS) * image.width(),
        image.y0 + (f64::from(row) + 0.5) * f64::from(CELL_HEIGHT) / f64::from(PAGE_HEIGHT) * image.height(),
    )
}

/// REQ-737 item 3: alone, on the wall and zoomed, a press in a cell as it
/// is drawn finds that cell at either shape, the last column's being beyond
/// where the raster's proportions end.
#[test]
fn a_press_finds_the_same_cell_either_way() {
    let pages = pages(4);
    for start in [Wall::new as fn() -> Wall, on_wall] {
        let (off_wall, off) = images(start(), PageShape::Raster, PANEL, &pages);
        let (on_wall, on) = images(start(), PageShape::Corrected, PANEL, &pages);
        let ((key, off_image), (_, on_image)) = (off[0], on[0]);
        for (column, row) in [(0, 0), (20, 12), (39, 24), (0, 24), (39, 0)] {
            let expected = Some(CellClick { key, version: 0, column, row });
            assert_eq!(off_wall.cell_at(cell_middle(off_image, column, row)), expected);
            assert_eq!(on_wall.cell_at(cell_middle(on_image, column, row)), expected);
        }
        assert!(
            on_image.width() > off_image.width() && (cell_middle(on_image, 39, 0).x - cell_middle(off_image, 39, 0).x).abs() > 1.0,
            "{off_image:?} {on_image:?}"
        );
    }
}

/// REQ-737: nothing moves until a viewer turns the correction on: the image
/// is where the screen's own arithmetic has always put it in a page of the
/// raster's proportions.
#[test]
fn with_the_correction_off_the_image_is_where_it_was() {
    let aspect = PageShape::Raster.height_per_width();
    for (left, top, width) in [(0.0, 0.0, 480.0), (181.0, 64.0, 320.0), (612.25, 133.5, 864.0)] {
        let rect = Rect::new(left, top, left + width, top + width * aspect);
        let scale = width / (f64::from(PAGE_WIDTH) + 2.0 * f64::from(CELL_WIDTH));
        let height = f64::from(PAGE_HEIGHT) * scale;
        let image = image_within(rect);
        assert!(same(image.x0, left + f64::from(CELL_WIDTH) * scale) && same(image.width(), f64::from(PAGE_WIDTH) * scale), "{image:?}");
        assert!(same(image.y0, f64::midpoint(rect.y0, rect.y1) - height / 2.0) && same(image.height(), height), "{image:?}");
    }
}

/// REQ-740, DEC-953: the screen is one cell wide at the sides of the page as
/// drawn, and as wide in proportion above and below, whatever its shape.
#[test]
fn the_screen_is_one_cell_of_the_page_as_drawn() {
    for shape in [PageShape::Raster, PageShape::Corrected] {
        let (width, height) = (800.0, 800.0 * shape.height_per_width());
        let rect = Rect::new(100.0, 50.0, 100.0 + width, 50.0 + height);
        let image = image_within(rect);
        assert!(same(image.width() / image.height(), rect.width() / rect.height()), "{image:?}");
        let columns = f64::from(COLUMNS);
        assert!(same(image.x0 - rect.x0, image.width() / columns) && same(rect.x1 - image.x1, image.width() / columns), "{shape:?} {image:?}");
        assert!(same(image.y0 - rect.y0, image.height() / columns) && same(rect.y1 - image.y1, image.height() / columns), "{shape:?} {image:?}");
    }
}

/// REQ-6387: a page's image is exactly a whole multiple of its actual
/// size while its room is that multiple or up to a twentieth more, on a
/// display of any scale, and fills its room otherwise; the band lies only
/// above the multiple, since a page cannot be larger than its room.
#[test]
fn a_width_within_a_twentieth_above_a_whole_multiple_is_held_at_it() {
    use crate::canvas::{REGIS_WIDTH, STICK_SHARE, sticky_width};
    for actual in [f64::from(PAGE_WIDTH), f64::from(REGIS_WIDTH)] {
        for scale in [1.0, 2.0] {
            for multiple in [1_u32, 2, 3] {
                let exact = f64::from(multiple) * actual / scale;
                assert_eq!(sticky_width(exact, actual, scale), (exact, Some(multiple)), "at the multiple");
                assert_eq!(sticky_width(exact * 1.03, actual, scale), (exact, Some(multiple)), "inside the band");
                assert_eq!(sticky_width(exact * (1.0 + STICK_SHARE) - 1e-6, actual, scale), (exact, Some(multiple)), "at the band's end");
                let past = exact * (1.0 + STICK_SHARE) + 0.5;
                assert_eq!(sticky_width(past, actual, scale), (past, None), "past the band");
                let below = exact - 0.5;
                assert_ne!(sticky_width(below, actual, scale).1, Some(multiple), "below the multiple");
                assert!(sticky_width(below, actual, scale).0 <= below, "never larger than its room");
            }
        }
    }
    assert_eq!(sticky_width(300.0, 480.0, 1.0), (300.0, None), "smaller than its actual size, nothing holds it");
    assert_eq!(sticky_width(0.0, 480.0, 1.0), (0.0, None));
    assert_eq!(sticky_width(500.0, 480.0, 0.0), (500.0, None));
}

/// The image of the one page open alone in a panel `width` across, far
/// taller than the page needs, on a display of `scale`.
fn image_alone(wall: &mut Wall, width: f64, scale: f64) -> Rect {
    let mut canvas = Recorder::new();
    canvas.scale = scale;
    let pages = pages(1);
    wall.draw(&mut canvas, Rect::new(0.0, 0.0, width, 4_000.0), &pages, None, 10_000);
    last_rect(&canvas, pages[0].key)
}

/// REQ-6387: as the window grows, the page open alone grows with it but
/// for a stretch at each whole multiple of its actual size, where it
/// stays exactly that size, centred: 480 display pixels and 960 for a
/// teletext page, 800 for a page drawn in ReGIS, on displays of scale 1
/// and 2. A page drawn in NAPLPS is the teletext raster (REQ-5819), so it
/// is held at the teletext page's multiples.
#[test]
fn the_page_alone_is_held_at_whole_multiples_as_its_room_grows() {
    use crate::canvas::{REGIS_WIDTH, STICK_SHARE};
    let key = pages(1)[0].key;
    for (picture, actual) in [(None, f64::from(PAGE_WIDTH)), (Some(false), f64::from(REGIS_WIDTH)), (Some(true), f64::from(PAGE_WIDTH))] {
        for scale in [1.0, 2.0] {
            let mut wall = Wall::new();
            if let Some(naplps) = picture {
                wall.set_regis_pages(core::iter::once(key), core::iter::once(key).filter(|_| naplps));
            }
            let mut held_at = Vec::new();
            let mut last = 0.0;
            for width in 100..2_400_u32 {
                let image = image_alone(&mut wall, f64::from(width), scale);
                // Centred, to within the display pixel a held page is moved onto (REQ-6401).
                assert!((f64::midpoint(image.x0, image.x1) - f64::from(width) / 2.0).abs() <= 1.0 / scale, "centred: {image:?}");
                if same(image.width() * scale / actual, (image.width() * scale / actual).round()) && image.width() * scale >= actual - 1e-6 {
                    assert!(
                        same(image.x0 * scale, (image.x0 * scale).round()) && same(image.y0 * scale, (image.y0 * scale).round()),
                        "a held page starts on a display pixel: {image:?}"
                    );
                }
                assert!(image.width() >= last - 1e-9, "never smaller in a larger room: {image:?}");
                last = image.width();
                let multiples = image.width() * scale / actual;
                let whole = multiples.floor();
                if whole >= 1.0 {
                    assert!(same(multiples, whole) || multiples > whole * (1.0 + STICK_SHARE) - 1e-9, "no size just past a multiple: {multiples} at {width}");
                }
                if same(multiples, whole) && whole >= 1.0 {
                    held_at.push((whole, width));
                }
            }
            for multiple in [1.0, 2.0] {
                let stretch = held_at.iter().filter(|(whole, _)| same(*whole, multiple)).count();
                // The band in panel widths: a twentieth of the multiple,
                // less a pixel at each end for where the steps fall.
                let band = multiple * actual / scale * STICK_SHARE;
                assert!(
                    f64::from(u32::try_from(stretch).unwrap()) >= band - 2.0,
                    "held at {multiple} for {stretch} of about {band} widths (picture {picture:?}, scale {scale})"
                );
            }
        }
    }
}

/// REQ-6387: with the width correction on, a held teletext page is a
/// whole multiple of its dots in height, which is what the correction
/// leaves whole.
#[test]
fn a_corrected_page_is_held_by_its_height() {
    let mut wall = Wall::new();
    wall.set_shape(PageShape::Corrected);
    let held = f64::from(PAGE_WIDTH) * STRETCH * SCREEN_SCALE * 1.02;
    let image = image_alone(&mut wall, held, 1.0);
    assert!(same(image.height(), f64::from(PAGE_HEIGHT)), "{image:?}");
}

/// REQ-6387: the label is for the moment a page comes to be held: it
/// names the multiple for a second and a half from then, is not renewed
/// by frames at the same size, and comes again only when the page is
/// held anew.
#[test]
fn the_held_label_lasts_from_the_moment_the_page_is_held() {
    use crate::canvas::STICK_LABEL_MILLISECONDS;
    let pages = pages(1);
    let mut wall = Wall::new();
    let mut canvas = Recorder::new();
    canvas.scale = 1.0;
    let held = Rect::new(0.0, 0.0, f64::from(PAGE_WIDTH) * SCREEN_SCALE * 1.02, 4_000.0);
    let free = Rect::new(0.0, 0.0, f64::from(PAGE_WIDTH) * SCREEN_SCALE * 1.5, 4_000.0);
    wall.draw(&mut canvas, free, &pages, None, 0);
    assert_eq!(wall.held_label(0), None, "not held, no label");
    wall.draw(&mut canvas, held, &pages, None, 100);
    assert_eq!(wall.held_label(100), Some((1, 100 + STICK_LABEL_MILLISECONDS)));
    wall.draw(&mut canvas, held, &pages, None, 1_000);
    assert_eq!(wall.held_label(1_000), Some((1, 100 + STICK_LABEL_MILLISECONDS)), "a later frame does not renew it");
    assert_eq!(wall.held_label(100 + STICK_LABEL_MILLISECONDS), None, "gone when its time is up");
    wall.draw(&mut canvas, free, &pages, None, 5_000);
    assert_eq!(wall.held_label(5_000), None);
    wall.draw(&mut canvas, held, &pages, None, 6_000);
    assert_eq!(wall.held_label(6_000), Some((1, 6_000 + STICK_LABEL_MILLISECONDS)), "held anew");
}

/// REQ-5819, DEC-6667: a page known to be drawn in NAPLPS stands in black
/// over its whole image, the teletext raster its picture fills, not in a
/// ReGIS picture's band.
#[test]
fn a_naplps_page_without_its_picture_stands_in_black_over_its_whole_image() {
    use crate::theme::BLACK;
    let pages = pages(1);
    let key = pages[0].key;
    let mut wall = on_wall();
    wall.set_regis_pages(core::iter::once(key), core::iter::once(key));
    let mut canvas = Recorder::new();
    canvas.screen = None;
    wall.draw(&mut canvas, PANEL, &pages, None, 10_000);
    let image = wall.drawn.iter().find(|drawn| drawn.key == key).map(|drawn| drawn.image).unwrap();
    assert!(canvas.fills.contains(&(image, BLACK)), "black over the whole image {image:?}: {:?}", canvas.fills);
}

/// The owner's request of 18 November 2026 (REQ-740, DEC-953: a page is
/// laid out in its own shape): before a page known to be drawn as a picture
/// has any image, what stands in for it is black, in the place and shape its
/// picture will take, not the teletext page's dark frame, so nothing changes
/// shape when the picture comes. A teletext page keeps its frame.
#[test]
fn a_picture_page_without_its_picture_stands_in_black_in_the_pictures_place() {
    use crate::canvas::{REGIS_HEIGHT_PER_WIDTH, regis_within};
    use crate::theme::{BLACK, WINDOW};
    let pages = pages(2);
    let (picture, teletext) = (pages[0].key, pages[1].key);
    for contained in [false, true] {
        let mut wall = on_wall();
        wall.set_regis_pages(core::iter::once(picture), core::iter::empty());
        if contained {
            wall.set_pictures(core::iter::once(picture));
        }
        let mut canvas = Recorder::new();
        canvas.screen = None;
        wall.draw(&mut canvas, PANEL, &pages, None, 10_000);
        let place = |key| wall.drawn.iter().find(|drawn| drawn.key == key).map(|drawn| (drawn.rect, drawn.image)).unwrap();
        let (rect, image) = place(picture);
        let expected = if contained { crate::canvas::regis_layout(image, true, false).picture } else { regis_within(image) };
        assert!(same(expected.height() / expected.width(), REGIS_HEIGHT_PER_WIDTH), "the picture's own shape: {expected:?}");
        assert!(canvas.fills.contains(&(expected, BLACK)), "black in the picture's place, contained {contained}: {:?}", canvas.fills);
        assert!(!canvas.fills.contains(&(rect, WINDOW)), "no teletext-shaped frame for the picture page");
        assert!(canvas.fills.contains(&(place(teletext).0, WINDOW)), "a teletext page keeps its frame");
    }
}
