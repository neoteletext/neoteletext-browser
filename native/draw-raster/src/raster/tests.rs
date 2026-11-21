//! The rasteriser's own tests (DEC-7168): exact-area coverage by the
//! non-zero rule, clips as layers at their edges, compositing, and the
//! image samplers.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::unwrap_used, reason = "a test's own small sizes and known values")]

use super::coverage::{Coverage, Line, alpha};
use super::paint::{Axis, ImagePaint, Pixels, Sampling};
use super::path;
use super::surface::{Source, Surface};
use super::{Paint, Scratch, fill_lines, fill_rect};
use crate::DeviceBox;

const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

/// The alphas `lines` cover in a box `width` by `height` at the origin.
fn alphas(lines: &[Line], width: u16, height: u16) -> Vec<u8> {
    let mut found = vec![0; usize::from(width) * usize::from(height)];
    Coverage::default().fill(lines, DeviceBox::whole(width, height), |row, column, length, mask| {
        for index in 0..length {
            found[usize::from(row) * usize::from(width) + usize::from(column) + index] = mask.map_or(u8::MAX, |mask| mask[index]);
        }
    });
    found
}

fn square(lines: &mut Vec<Line>, (x0, y0, x1, y1): (f64, f64, f64, f64)) {
    path::polygon(lines, [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
}

#[test]
fn a_pixel_is_covered_by_the_exact_area_inside() {
    let mut lines = Vec::new();
    // Half of the first column, all of the second, a quarter of the third.
    square(&mut lines, (0.5, 0.0, 2.25, 1.0));
    assert_eq!(alphas(&lines, 4, 1), vec![128, 255, 64, 0]);
    // A triangle over half of one pixel, along its diagonal.
    lines.clear();
    path::polygon(&mut lines, [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]);
    assert_eq!(alphas(&lines, 2, 1), vec![128, 0]);
}

#[test]
fn overlapping_contours_cover_once_and_a_reversed_one_cuts_a_hole() {
    let mut lines = Vec::new();
    square(&mut lines, (0.0, 0.0, 3.0, 1.0));
    square(&mut lines, (1.0, 0.0, 4.0, 1.0));
    assert_eq!(alphas(&lines, 4, 1), vec![255; 4], "non-zero: wound twice is covered once");
    lines.clear();
    square(&mut lines, (0.0, 0.0, 4.0, 1.0));
    // The same square the other way round: winding zero inside it.
    path::polygon(&mut lines, [(1.0, 0.0), (1.0, 1.0), (3.0, 1.0), (3.0, 0.0)]);
    assert_eq!(alphas(&lines, 4, 1), vec![255, 0, 0, 255]);
}

#[test]
fn a_shape_reaching_past_the_box_fills_what_lies_in_it() {
    let mut lines = Vec::new();
    square(&mut lines, (-50.5, -3.0, 2.5, 9.0));
    let found = alphas(&lines, 4, 2);
    assert_eq!(found, vec![255, 255, 128, 0, 255, 255, 128, 0]);
    // Nothing at all, and a line not a number, cover nothing.
    assert_eq!(alphas(&[], 3, 1), vec![0; 3]);
    let broken = [Line { x0: f32::NAN, y0: 0.0, x1: 1.0, y1: 1.0 }];
    assert_eq!(alphas(&broken, 3, 1), vec![0; 3]);
}

#[test]
fn coverage_rounds_to_the_nearest_alpha_and_holds_at_whole() {
    assert_eq!(alpha(0.0), 0);
    assert_eq!(alpha(0.5), 128);
    assert_eq!(alpha(-1.0), 255, "the magnitude: either winding");
    assert_eq!(alpha(2.0), 255, "non-zero: at most whole");
}

#[test]
fn a_rounded_rectangle_keeps_inside_its_rectangle_and_its_corners_open() {
    let mut lines = Vec::new();
    path::rounded_rect(&mut lines, (0.0, 0.0, 20.0, 10.0), 5.0);
    let found = alphas(&lines, 22, 12);
    let at = |x: usize, y: usize| found[y * 22 + x];
    assert_eq!(at(0, 0), 0, "the corner is cut");
    assert_eq!(at(10, 5), 255, "the middle is whole");
    assert_eq!((at(20, 5), at(10, 10)), (0, 0), "nothing past the rectangle");
    // A radius past half the shorter side is held to it: the ends are
    // half circles, not crossed arcs.
    lines.clear();
    path::rounded_rect(&mut lines, (0.0, 0.0, 20.0, 10.0), 50.0);
    assert_eq!(alphas(&lines, 22, 12)[5 * 22 + 10], 255);
}

/// A surface 4 by 1 pixels drawn whole, black.
fn black() -> Surface {
    let mut surface = Surface::new(4, 1);
    surface.begin(DeviceBox::whole(4, 1));
    surface
}

#[test]
fn source_over_lays_colour_by_its_coverage() {
    let mut surface = black();
    surface.span(0, 0, 4, Source::Solid(WHITE), Some(&[255, 128, 0, 64]));
    surface.end();
    assert_eq!(surface.words, vec![0x00FF_FFFF, 0x0080_8080, 0, 0x0040_4040]);
    // Half-strength black over white: dimming.
    let mut surface = surface_of(0x00FF_FFFF);
    surface.span(0, 0, 1, Source::Solid([0, 0, 0, 128]), None);
    assert_eq!(surface.words[0], 0x007F_7F7F);
}

fn surface_of(word: u32) -> Surface {
    let mut surface = black();
    surface.words.fill(word);
    surface
}

/// A clip is a layer at its edges: what is drawn inside it is laid over
/// what lies beneath through the clip's coverage once, not once for each
/// shape, as the interface was drawn before this rasteriser (DEC-7168).
#[test]
fn a_clip_lays_what_it_holds_over_its_edge_once() {
    let mut surface = black();
    // A clip covering half of the first and last pixels.
    surface.push_clip(0.5, 0.0, 3.5, 1.0);
    surface.span(0, 0, 4, Source::Solid(RED), None);
    surface.span(0, 0, 4, Source::Solid(WHITE), None);
    surface.pop_clip();
    surface.end();
    // White, the last drawn, at half strength over black at the edges;
    // a shape laid on the edge pixel directly each time would leave red.
    assert_eq!(surface.words, vec![0x0080_8080, 0x00FF_FFFF, 0x00FF_FFFF, 0x0080_8080]);
    // A clip covering no pixel lets nothing through.
    let mut surface = black();
    surface.push_clip(10.0, 0.0, 20.0, 1.0);
    surface.span(0, 0, 4, Source::Solid(WHITE), None);
    surface.end();
    assert_eq!(surface.words, vec![0; 4]);
}

#[test]
fn nested_clips_cut_to_both_and_unclosed_ones_close_at_the_end() {
    let mut surface = black();
    surface.push_clip(0.0, 0.0, 3.0, 1.0);
    surface.push_clip(1.0, 0.0, 4.0, 1.0);
    let mut scratch = Scratch::default();
    fill_rect(&mut surface, &mut scratch, (0.0, 0.0, 4.0, 1.0), &Paint::Solid(WHITE));
    surface.end();
    assert_eq!(surface.words, vec![0, 0x00FF_FFFF, 0x00FF_FFFF, 0]);
}

#[test]
fn a_rectangle_and_the_same_path_cover_alike() {
    let rect = (0.25, 0.0, 3.6, 1.0);
    let (mut by_rect, mut by_path) = (black(), black());
    let mut scratch = Scratch::default();
    fill_rect(&mut by_rect, &mut scratch, rect, &Paint::Solid(WHITE));
    scratch.lines.clear();
    square(&mut scratch.lines, rect);
    fill_lines(&mut by_path, &mut scratch, &Paint::Solid(WHITE));
    assert_eq!(by_rect.words, by_path.words);
}

#[test]
fn nearest_sampling_takes_the_pixel_under_each_centre_and_smooth_mixes_two() {
    // Two pixels, black then white, drawn four pixels wide.
    let pixels = Pixels::new(2, 1, vec![[0, 0, 0, 255], WHITE]).unwrap();
    let across = Axis { scale: 0.5, offset: 0.0 };
    let down = Axis { scale: 1.0, offset: 0.0 };
    let mut row = Vec::new();
    ImagePaint::new(&pixels, Sampling::Nearest, across, down, 0, 4).row(0, 0, 4, &mut row);
    let red = |row: &[u32]| row.iter().map(|word| word >> 16).collect::<Vec<_>>();
    assert_eq!(red(&row), vec![0, 0, 255, 255]);
    ImagePaint::new(&pixels, Sampling::Smooth, across, down, 0, 4).row(0, 0, 4, &mut row);
    // Centres at 0.25, 0.75, 1.25 and 1.75 of the image's pixels: the
    // edges carried on outward, a quarter and three quarters between, in
    // 255ths: 255 × 191 / 255, rounded up, is 191.
    assert_eq!(red(&row), vec![0, 64, 191, 255]);
    assert!(Pixels::new(2, 2, vec![WHITE]).is_none(), "the pixels must be the size's");
}
