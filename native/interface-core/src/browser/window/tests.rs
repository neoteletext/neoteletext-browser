//! The window's frame: the console's place, and REQ-740's colour behind
//! everything below the trusted strip. The colours are read from what was
//! drawn, so these say what the viewer sees, not how it was worked out.

#![allow(clippy::arithmetic_side_effects, reason = "a test's positions")]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use neoteletext_core::colour::ColourMap;
use neoteletext_core::consts::COLOUR_MAP_ENTRIES;

use super::*;
use crate::browser::bookmarks_bar::BAR_HEIGHT;
use crate::browser::fixture::{HEIGHT, WIDTH, browser, cell_centre, middle};
use crate::browser::overview::ROW_FLASH;
use crate::browser::{CertificateProblem, Denied, StripState};
use crate::channel::{BANKS, Status};
use crate::geometry::Point;
use crate::library::BarView;
use crate::test_canvas::Recorder;
use crate::text::HAIRLINE;
use crate::theme::{
    BANK_COLOURS, BLACK, BLUE, Colour, LINK, MARK_CONTRAST, MUTED, PANEL, RAISED, RED, RULE, RULE_SOFT, SELECTED, TEXT_CONTRAST, UPDATED, WHITE, YELLOW,
    contrast, ink_on, mix,
};
use crate::wall::since;

/// A colour none of the strip's parts draw in.
const SCREEN: Colour = Colour { red: 0x77, green: 0x00, blue: 0x77 };
/// Where the strip ends, with the bookmarks bar hidden.
const STRIP_BOTTOM: f64 = BODY_TOP - MARGIN;
/// Inside a panel, in the gap between its edge and what it holds.
const PANEL_GAP: f64 = 3.0;

/// The window with `screen` as every page's colour.
fn drawn(browser: &mut Browser, screen: Option<Colour>) -> Recorder {
    let mut canvas = Recorder::new();
    canvas.screen = screen;
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    canvas
}

/// A window showing the page alone, as a window starts, and its first frame.
fn alone(screen: Option<Colour>) -> (Browser, Recorder) {
    let (mut browser, _) = browser();
    browser.toggle_wall(0);
    let canvas = drawn(&mut browser, screen);
    assert!(browser.wall.single());
    (browser, canvas)
}

/// The colour on top at `point`, of everything but text.
fn colour_at(canvas: &Recorder, point: Point) -> Colour {
    canvas.fills.iter().rev().find(|(rect, _)| rect.contains(point)).map(|&(_, colour)| colour).expect("something drawn there")
}

fn ink_of(canvas: &Recorder, text: &str) -> Colour {
    canvas.coloured_text(text).unwrap_or_else(|| panic!("{text} not drawn"))
}

/// The colour `text` was drawn in inside `rect`, as the overview's axis has
/// a list row's numbers too.
fn ink_in(canvas: &Recorder, rect: Rect, text: &str) -> Colour {
    let drawn = canvas.texts.iter().zip(&canvas.inks).find(|((place, drawn), _)| drawn == text && rect.contains(middle(*place)));
    let Some((_, &(_, colour))) = drawn else { panic!("{text} not drawn in {rect:?}") };
    colour
}

/// On a row's top edge, its sides being under the list's border.
fn row_edge(row: Rect) -> Point {
    Point::new(middle(row).x, row.y0 + HAIRLINE)
}

fn margin() -> Point {
    Point::new(MARGIN / 2.0, HEIGHT / 2.0)
}

/// Inside the magazines' panel, inside a list row not chosen, inside the
/// page area beside the page, and inside the overview.
fn panel_places(browser: &Browser) -> [Point; 4] {
    let bank = browser.regions.banks[1];
    let area = browser.wall.area();
    let overview = browser.regions.overview;
    [
        Point::new(bank.x0 - PANEL_GAP, middle(bank).y),
        middle(browser.regions.rows[2].0),
        Point::new(area.x0 + PANEL_GAP, area.y0 + PANEL_GAP),
        Point::new(overview.x0 + PANEL_GAP, overview.y0 + PANEL_GAP),
    ]
}

/// Requirement 19.8: the console docks in the overview's place.
#[test]
fn the_console_takes_the_overviews_place() {
    let (mut browser, _) = browser();
    browser.toggle_console();
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
    assert_eq!(browser.regions.overview, Rect::ZERO, "no overview");
    let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
    assert!(texts.contains(&"Network 0") && texts.contains(&"Pause"), "{texts:?}");
    assert!(browser.console.area().y1 <= HEIGHT && browser.console.area().y0 > BODY_TOP);
}

/// REQ-740 item 2: from the window's first frame with the page alone, all of
/// the window below the strip is the page's colour, its panels included.
#[test]
fn the_pages_colour_is_behind_everything_below_the_strip() {
    let (browser, canvas) = alone(Some(SCREEN));
    assert!(canvas.fills.contains(&(Rect::new(0.0, STRIP_BOTTOM, WIDTH, HEIGHT), SCREEN)), "the whole of it");
    assert_eq!(colour_at(&canvas, margin()), SCREEN, "the margin");
    assert_eq!(colour_at(&canvas, Point::new(WIDTH - MARGIN / 2.0, STRIP_BOTTOM + 1.0)), SCREEN, "up to the strip and the window's edge");
    for place in panel_places(&browser) {
        assert_eq!(colour_at(&canvas, place), SCREEN, "{place:?}");
    }
}

/// REQ-737 item 2 with REQ-740: the corrected page alone stands on its
/// colour, the window's fill unchanged and its screen around its image.
#[test]
fn the_corrected_page_alone_stands_on_its_colour() {
    let (mut browser, off) = alone(Some(SCREEN));
    crate::browser::fixture::set_aspect(&mut browser, true);
    let on = drawn(&mut browser, Some(SCREEN));
    let (image, off_image) = (on.pages[0].1, off.pages[0].1);
    assert!(image.width() > off_image.width(), "{image:?} {off_image:?}");
    assert!(on.fills.contains(&(Rect::new(0.0, STRIP_BOTTOM, WIDTH, HEIGHT), SCREEN)), "the whole of it");
    assert!(on.fills.contains(&(browser.wall.area(), SCREEN)), "the page area");
    let page = on.fills.iter().rev().find(|(rect, colour)| *colour == SCREEN && rect.contains(middle(image))).map(|&(rect, _)| rect).unwrap();
    assert!(page.x0 < image.x0 && page.x1 > image.x1 && page.y0 < image.y0 && page.y1 > image.y1, "around it: {page:?} {image:?}");
}

/// REQ-737 item 2 with REQ-740: on the wall each page keeps its own border
/// on all four sides of the stretched image.
#[test]
fn a_corrected_page_on_the_wall_keeps_its_border() {
    let (mut browser, off) = browser();
    crate::browser::fixture::set_aspect(&mut browser, true);
    let on = drawn(&mut browser, Some(SCREEN));
    let image = on.pages[0].1;
    assert!(image.width() > off.pages[0].1.width() * 1.2, "{image:?}");
    let border = on.fills.iter().find(|(rect, colour)| *colour == SCREEN && rect.contains(middle(image))).map(|&(rect, _)| rect).unwrap();
    let cell = image.width() / f64::from(crate::canvas::COLUMNS);
    assert!((image.x0 - border.x0 - cell).abs() < 1e-6 && (border.x1 - image.x1 - cell).abs() < 1e-6, "one cell: {border:?} {image:?}");
    assert!(image.y0 > border.y0 && image.y1 < border.y1, "{border:?} {image:?}");
}

/// The fills and the texts of the window's strip.
type Strip = (Vec<(Rect, Colour)>, Vec<(String, Colour)>);

/// REQ-740 item 3 (REQ-750): everything the strip draws is the same on any
/// page, and the colour stops where it ends, with its bookmarks bar too.
#[test]
fn the_strip_never_takes_the_colour() {
    for bar in [false, true] {
        let strip_bottom = STRIP_BOTTOM + if bar { BAR_HEIGHT } else { 0.0 };
        let strip = |canvas: &Recorder| -> Strip {
            let inks = canvas.texts.iter().zip(&canvas.inks).filter(|((rect, _), _)| rect.y0 < strip_bottom).map(|(_, ink)| ink.clone());
            (canvas.fills.iter().filter(|(rect, _)| rect.y0 < strip_bottom).copied().collect(), inks.collect())
        };
        let (mut browser, _) = alone(Some(SCREEN));
        browser.set_bar(BarView { shown: bar, ..BarView::default() });
        let (black, coloured) = (drawn(&mut browser, Some(BLACK)), drawn(&mut browser, Some(SCREEN)));
        assert!(coloured.fills.contains(&(Rect::new(0.0, strip_bottom, WIDTH, HEIGHT), SCREEN)), "the colour starts at the strip's end");
        assert!(!strip(&coloured).0.is_empty() && !strip(&coloured).1.is_empty(), "the strip is drawn");
        assert_eq!(strip(&coloured), strip(&black));
        assert!(strip(&coloured).0.iter().all(|&(_, colour)| colour != SCREEN));
    }
}

/// The colour changes what is drawn, never where.
#[test]
fn the_colour_moves_nothing_a_press_finds() {
    fn places(browser: &Browser) -> Vec<Rect> {
        let regions = &browser.regions;
        let rows = regions.rows.iter().map(|&(row, _)| row);
        let single = [regions.list, regions.overview, regions.overview_toggle, regions.overview_close, regions.cells, regions.divider, browser.wall.area()];
        regions.banks.iter().copied().chain(rows).chain(single).collect()
    }
    let (mut browser, _) = alone(Some(BLACK));
    drawn(&mut browser, Some(BLACK));
    let on_black = places(&browser);
    drawn(&mut browser, Some(SCREEN));
    assert_ne!(on_black, []);
    assert_eq!(places(&browser), on_black);
}

/// REQ-740 item 2 and 7: a black page fills the margins black too, and every
/// panel draws as it always has.
#[test]
fn on_a_black_page_the_panels_are_what_they_were() {
    let (browser, canvas) = alone(Some(BLACK));
    assert_eq!(colour_at(&canvas, margin()), BLACK, "the margin");
    for place in panel_places(&browser) {
        assert_eq!(colour_at(&canvas, place), PANEL, "{place:?}");
    }
    assert_eq!(ink_of(&canvas, "Channel magazines"), WHITE);
    for (bank, colour) in BANK_COLOURS.into_iter().enumerate() {
        let number = bank + 1;
        assert_eq!(ink_of(&canvas, &format!("{number}00-{number}99")), colour);
    }
    assert_eq!(ink_of(&canvas, "Page"), MUTED);
    assert_eq!(ink_of(&canvas, "Page 1"), WHITE);
    assert_eq!(ink_of(&canvas, "Empty"), MUTED);
    let bank = browser.regions.banks[1];
    assert_eq!(colour_at(&canvas, Point::new(bank.x0, middle(bank).y)), RULE_SOFT, "around a magazine");
    let panel = bank.x0 - PANEL_GAP * 2.0;
    assert_eq!(colour_at(&canvas, Point::new(panel, middle(bank).y)), RULE, "around the panel");
}

/// The colours of Table 30, as the engine holds them.
fn table_30() -> impl Iterator<Item = Colour> {
    (0..COLOUR_MAP_ENTRIES).map(|index| {
        let [red, green, blue] = ColourMap::DEFAULT.rgb888(u8::try_from(index).unwrap());
        Colour { red, green, blue }
    })
}

fn assert_contrast(colour: Colour, back: Colour, floor: f64, what: &str) {
    let ratio = contrast(colour, back);
    assert!(ratio >= floor, "{what}, {colour:?} on {back:?}: {ratio:.2} under {floor}");
}

/// REQ-740 item 4: on each of the 32 defaults, what the window draws meets
/// the floors of WCAG 2.1's contrast ratio, (lighter + 0.05) / (darker +
/// 0.05) of relative luminances: text 4.5, secondary text and marks 3.
#[test]
fn every_default_of_the_colour_map_reads_in_the_window() {
    for back in table_30() {
        let (mut browser, _) = browser();
        browser.toggle_wall(0);
        browser.channel.banks[1] = "Sport".into();
        // A loaded page of each magazine, to show its mark in the overview.
        let cells: Vec<usize> = (0..BANKS).map(|bank| bank_indices(bank).find(|&index| browser.channel.pages[index].is_some()).unwrap()).collect();
        for &cell in &cells {
            browser.channel.pages[cell].as_mut().unwrap().status = Status::Loaded;
        }
        // The first frame opens the first page; the second has its row chosen.
        drawn(&mut browser, Some(back));
        let canvas = drawn(&mut browser, Some(back));
        assert_eq!(colour_at(&canvas, margin()), back);
        for text in ["Channel magazines", "Page 1", "All 800 pages · overview"] {
            assert_contrast(ink_of(&canvas, text), back, TEXT_CONTRAST, text);
        }
        for text in ["Page", "Title", "Empty", "Available", "200"] {
            assert_contrast(ink_of(&canvas, text), back, MARK_CONTRAST, text);
        }
        for (bank, &cell) in cells.iter().enumerate().skip(1) {
            let number = bank + 1;
            let range = format!("{number}00-{number}99");
            assert_contrast(ink_of(&canvas, &range), back, MARK_CONTRAST, &range);
            let rect = browser.regions.banks[bank];
            let swatch = colour_at(&canvas, Point::new(rect.x0 + PADDING + 1.0, middle(rect).y));
            assert_contrast(swatch, back, MARK_CONTRAST, "the swatch");
            assert_contrast(colour_at(&canvas, cell_centre(&browser, cell)), back, MARK_CONTRAST, "a loaded cell");
        }
        assert_contrast(ink_of(&canvas, "Sport"), back, MARK_CONTRAST, "a magazine's name");
        let heading = Point::new(browser.regions.list.x0 + PANEL_GAP, browser.regions.list.y0 - 2.0 * HEADING_HEIGHT);
        assert_contrast(colour_at(&canvas, heading), back, MARK_CONTRAST, "the list's heading");
        // The chosen row shows by its fill, or else by its edge.
        let (row, _) = *browser.regions.rows.first().unwrap();
        assert_eq!(browser.selected, Some(0));
        assert_eq!(colour_at(&canvas, middle(row)), SELECTED);
        assert_eq!(ink_in(&canvas, row, "100"), WHITE, "the chosen row keeps white text");
        let edge = colour_at(&canvas, row_edge(row));
        if back != BLACK {
            let shown = contrast(SELECTED, back).max(contrast(edge, back));
            assert!(shown >= MARK_CONTRAST, "the chosen row on {back:?}: {shown:.2}");
        }
    }
}

/// REQ-740 item 4: a magazine's yellow on a yellow screen, its mark, its
/// heading and its list's border.
#[test]
fn a_magazine_on_its_own_colour_does_not_vanish() {
    let (mut browser, _) = alone(Some(YELLOW));
    browser.bank = 2;
    let canvas = drawn(&mut browser, Some(YELLOW));
    let list = browser.regions.list;
    let heading = colour_at(&canvas, Point::new(list.x0 + 1.0, list.y0 - 2.0 * HEADING_HEIGHT));
    assert_eq!(heading, BLACK, "the heading in the ink");
    assert_eq!(ink_of(&canvas, "Magazine 3"), ink_on(BLACK), "its words on that");
    assert_eq!(ink_of(&canvas, "600-699"), BLUE, "another magazine keeps its colour");
    assert_eq!(ink_of(&canvas, "300-399"), YELLOW, "the chosen magazine stands on a button's face");
    browser.bank = 0;
    let canvas = drawn(&mut browser, Some(YELLOW));
    assert_eq!(ink_of(&canvas, "300-399"), BLACK, "on the surface, in the ink");
}

/// REQ-740 item 4 and the rows' rule: on a blue screen the chosen row keeps
/// its fill and white text, edged in the ink; a row on a light screen takes
/// the surface's ink, and the chosen one still has white.
#[test]
fn rows_with_a_fill_keep_white_text_and_rows_on_the_surface_take_the_ink() {
    let (mut browser, _) = alone(Some(SELECTED));
    let canvas = drawn(&mut browser, Some(SELECTED));
    let (row, _) = browser.regions.rows[0];
    assert_eq!(colour_at(&canvas, middle(row)), SELECTED);
    assert_eq!(colour_at(&canvas, row_edge(row)), WHITE, "edged");
    let canvas = drawn(&mut browser, Some(WHITE));
    assert_eq!(ink_in(&canvas, row, "100"), WHITE, "the chosen row");
    assert_eq!(ink_of(&canvas, "101"), BLACK, "a row on the surface");
    assert_eq!(ink_of(&canvas, "Page 1"), BLACK);
    browser.hovered_row = Some(1);
    let canvas = drawn(&mut browser, Some(WHITE));
    assert_eq!(colour_at(&canvas, middle(browser.regions.rows[1].0)), RAISED);
    assert_eq!(ink_of(&canvas, "101"), WHITE, "a row under the pointer");
    assert_eq!(colour_at(&canvas, row_edge(row)), SELECTED, "a fill that shows needs no edge");
}

/// A row's update flash starts from what the row stands on, and its age
/// reads on the surface.
#[test]
fn a_rows_flash_mixes_from_the_surface() {
    let (mut browser, _) = alone(Some(BLUE));
    browser.updates.push((1, 0));
    let canvas = drawn(&mut browser, Some(BLUE));
    let row = browser.regions.rows[1].0;
    assert_eq!(colour_at(&canvas, middle(row)), mix(BLUE, UPDATED, ROW_FLASH));
    let canvas = drawn(&mut browser, Some(WHITE));
    assert_eq!(colour_at(&canvas, middle(row)), mix(WHITE, UPDATED, ROW_FLASH));
    let age = since(Some(0), 0).expect("its age");
    assert_contrast(ink_of(&canvas, &age), WHITE, MARK_CONTRAST, "the age");
}

/// The chosen page's sub-pages stand on the surface as the pages do.
#[test]
fn the_sub_pages_stand_on_the_colour() {
    let (mut browser, _) = alone(Some(YELLOW));
    browser.bank = 2;
    browser.selected = browser.bank_pages().first().copied();
    let page = page_number(browser.selected.unwrap()).unwrap();
    browser.set_subpage_count(7, page, 3);
    browser.set_subpage_rows(7, page, 3, 0, alloc::vec![(1, "One".into()), (2, "Two".into()), (3, "Three".into())]);
    let canvas = drawn(&mut browser, Some(YELLOW));
    assert_eq!(ink_of(&canvas, "Two"), BLACK);
    let words = format!("Sub-pages of {page:03X} (3)");
    assert_contrast(ink_of(&canvas, &words), YELLOW, MARK_CONTRAST, "the heading");
    let divider = middle(browser.regions.subpage_divider);
    assert_eq!(colour_at(&canvas, divider), BLACK, "the divider in the ink");
}

/// The ring of a list that has the keys shows on the ring's own colour.
#[test]
fn the_keys_ring_shows_on_every_colour() {
    let (mut browser, _) = alone(Some(LINK));
    browser.list_focus = Some(ListFocus::Pages);
    let canvas = drawn(&mut browser, Some(LINK));
    let list = browser.regions.list;
    assert_contrast(colour_at(&canvas, Point::new(list.x0 + 1.0, middle(list).y)), LINK, MARK_CONTRAST, "the ring");
}

/// REQ-740 item 5: on the wall of pages the window is the browser's and each
/// tile has its own colour; the page opened alone takes it again.
#[test]
fn the_wall_keeps_the_browsers_background() {
    let (mut browser, canvas) = alone(Some(SCREEN));
    assert_eq!(colour_at(&canvas, margin()), SCREEN, "the page alone");
    browser.toggle_wall(0);
    let canvas = drawn(&mut browser, Some(SCREEN));
    assert_eq!(colour_at(&canvas, margin()), WINDOW);
    let [panel, row, _, overview] = panel_places(&browser);
    for place in [panel, row, overview] {
        assert_eq!(colour_at(&canvas, place), PANEL, "{place:?}");
    }
    let tile = browser.wall.shown().next().expect("a tile").1;
    assert_eq!(colour_at(&canvas, Point::new(tile.x0 - 1.0, middle(tile).y)), SCREEN, "the tile's border");
    assert_eq!(ink_of(&canvas, "Channel magazines"), WHITE);
}

/// REQ-740 item 5: before the page has arrived there is no colour to take.
#[test]
fn a_page_not_yet_arrived_keeps_the_browsers_background() {
    assert_eq!(colour_at(&alone(Some(SCREEN)).1, margin()), SCREEN, "once it has");
    let (browser, canvas) = alone(None);
    assert_eq!(colour_at(&canvas, margin()), WINDOW);
    for place in panel_places(&browser) {
        assert_eq!(colour_at(&canvas, place), PANEL, "{place:?}");
    }
}

/// REQ-740 item 6 and REQ-1686 item 5: what stands in for a page draws on the
/// browser's own background, and nothing of the channel stays.
#[test]
fn what_stands_in_for_the_page_keeps_the_browsers_background() {
    let cases: [fn(&mut StripState); 3] = [
        |strip| strip.redirecting = "https://new.invalid/".into(),
        |strip| strip.refused = Some(CertificateProblem::Expired),
        |strip| strip.denied = Denied::of_status(403),
    ];
    for case in cases {
        let (mut browser, canvas) = alone(Some(SCREEN));
        assert_eq!(colour_at(&canvas, margin()), SCREEN, "the page, before");
        let mut strip = browser.strip.clone();
        case(&mut strip);
        browser.set_strip(strip);
        let canvas = drawn(&mut browser, Some(SCREEN));
        assert_eq!(colour_at(&canvas, margin()), WINDOW);
        let [panel, row, _, overview] = panel_places(&browser);
        for place in [panel, row, overview] {
            assert_eq!(colour_at(&canvas, place), PANEL, "{place:?}");
        }
        assert_eq!(ink_of(&canvas, "Channel magazines"), WHITE);
    }
}

/// The notice clears the page area itself, and draws no page: the colour is
/// nowhere on screen.
#[test]
fn a_redirects_notice_leaves_none_of_the_colour() {
    let (mut browser, _) = alone(Some(SCREEN));
    browser.set_strip(StripState { redirecting: "https://new.invalid/".into(), ..browser.strip.clone() });
    let canvas = drawn(&mut browser, Some(SCREEN));
    assert_eq!(canvas.pages, [], "no page");
    assert!(canvas.fills.iter().all(|&(_, colour)| colour != SCREEN), "none of the colour");
    assert_eq!(colour_at(&canvas, middle(browser.redirecting_area)), BLACK);
}

/// REQ-740 item 3: the console and the browser's own panels keep theirs.
#[test]
fn the_console_and_the_settings_keep_their_own_colours() {
    let (mut browser, canvas) = alone(Some(SCREEN));
    assert_eq!(colour_at(&canvas, margin()), SCREEN, "the page alone");
    browser.toggle_console();
    let canvas = drawn(&mut browser, Some(SCREEN));
    let console = browser.console.area();
    assert_eq!(colour_at(&canvas, Point::new(console.x0 + PANEL_GAP, console.y0 + PANEL_GAP)), PANEL, "the console");
    browser.toggle_console();
    browser.toggle_settings();
    let canvas = drawn(&mut browser, Some(SCREEN));
    let window = middle(Rect::new(0.0, 0.0, WIDTH, HEIGHT));
    assert!(canvas.fills.iter().any(|&(rect, colour)| colour == PANEL && rect.contains(window)), "the settings panel");
}

/// What a channel names a magazine stays bounded in its place, in whatever
/// colour it stands.
#[test]
fn a_magazines_name_is_cut_to_its_place_on_any_colour() {
    let (mut browser, _) = alone(Some(RED));
    browser.channel.banks[1] = "M".repeat(200);
    let canvas = drawn(&mut browser, Some(RED));
    let bank = browser.regions.banks[1];
    let name =
        canvas.texts.iter().find(|(rect, text)| rect.x0 > bank.x0 && rect.y0 > bank.y0 && rect.y1 <= bank.y1 && text.starts_with('M')).expect("the name");
    assert!(name.1.chars().count() < 200, "fitted: {}", name.1);
    assert!(name.0.x1 <= bank.x1, "{:?} in {bank:?}", name.0);
}
