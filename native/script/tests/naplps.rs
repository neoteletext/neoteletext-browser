//! The NAPLPS painter's bytes, drawn by the NAPLPS renderer at the pinned
//! commit (script-api.md sections 7.1 to 7.3, work package N1): for each
//! call, what the painter writes draws what a hand-written reference
//! draws, or exactly the dots the call names (neoteletext-common's
//! `docs/naplps-raster.md` RAS-2, RAS-6 and RAS-8).
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "tests"
)]

mod naplps_drawn;

use naplps::limits::DISPLAY_FIRST_LINE;
use naplps::{Event, Rounding, dot_steps, multi_value_operand};
use naplps_drawn::{HEIGHT, Program, WIDTH, draw, lit, opening, paint, paint_as, pixel};
use neoteletext_script::host::naplps::Fault;
use neoteletext_script::host::naplps::program::Paint;
use neoteletext_script::host::naplps::writer::{NoDefinitions, Painter, Run, Target};

const SO: u8 = 0x0E;
const END: u8 = 0x1A;
const WHITE: u8 = 7;
/// The letters and digits.
const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// The six-byte operand of the canvas dot (`x`, `y`), as RAS-2 writes it.
fn dot(x: i64, y: i64, out: &mut Vec<u8>) {
    let (across, up) = dot_steps(x, y + i64::from(DISPLAY_FIRST_LINE), 1, 6, Rounding::Nearest);
    multi_value_operand(across, up, 6, out);
}

/// The difference of two dots' operands, as a relative operand.
fn between(from: (i64, i64), to: (i64, i64), out: &mut Vec<u8>) {
    let top = i64::from(DISPLAY_FIRST_LINE);
    let a = dot_steps(from.0, from.1 + top, 1, 6, Rounding::Nearest);
    let b = dot_steps(to.0, to.1 + top, 1, 6, Rounding::Nearest);
    multi_value_operand(b.0 - a.0, b.1 - a.1, 6, out);
}

/// A hand-written stream: the generation's opening, SELECT COLOR white,
/// then `body` in the PDI set.
fn reference(body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut bytes = opening();
    bytes.extend_from_slice(&[0x3E, 0x40 | (WHITE << 2)]);
    body(&mut bytes);
    bytes.push(END);
    bytes
}

/// Section 7.3, `fillRect` (RAS-6): a filled rectangle covers exactly the
/// dots it names and no other.
#[test]
fn a_filled_rectangle_covers_exactly_its_dots() {
    let canvas = draw(&paint(&Program::new().call(Paint::FILL_RECT, &[12, 20, 12, 20])));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let inside = (12..24).contains(&x) && (20..40).contains(&y);
            assert_eq!(pixel(&canvas, x, y), if inside { WHITE } else { 0 }, "dot ({x}, {y})");
        }
    }
    // The whole canvas, to its last dot and line.
    let canvas = draw(&paint(&Program::new().call(Paint::FILL_RECT, &[0, 0, 480, 460])));
    assert_eq!(lit(&canvas), (WIDTH * HEIGHT) as usize);
}

/// Section 7.3, `rectBy` and `fillRectBy`: from the drawing point, which
/// then moves by the width, so that rectangles adjoin with no dot shared
/// and none between.
#[test]
fn rectangles_by_the_drawing_point_adjoin() {
    let program =
        Program::new().call(Paint::MOVE_TO, &[0, 40]).call(Paint::FILL_RECT_BY, &[12, 20]).call(Paint::INK, &[1]).call(Paint::FILL_RECT_BY, &[12, 20]);
    let canvas = draw(&paint(&program));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let want = match (x, y) {
                (0..12, 40..60) => WHITE,
                (12..24, 40..60) => 1,
                _ => 0,
            };
            assert_eq!(pixel(&canvas, x, y), want, "dot ({x}, {y})");
        }
    }
    // The outlined form draws the same rectangle's edge.
    let outline = draw(&paint(&Program::new().call(Paint::MOVE_TO, &[30, 30]).call(Paint::RECT_BY, &[10, 8])));
    let absolute = draw(&paint(&Program::new().call(Paint::RECT, &[30, 30, 10, 8])));
    assert_eq!(outline.pixels(), absolute.pixels());
    assert_eq!((pixel(&outline, 30, 30), pixel(&outline, 39, 37), pixel(&outline, 34, 34), pixel(&outline, 40, 30)), (WHITE, WHITE, 0, 0));
}

/// Section 7.3, the absolute forms: POINT, LINE, SET and LINE, SET and
/// RECTANGLE, SET and POLYGON and ARC, each drawn as a hand-written
/// stream of the standard's opcodes draws it.
#[test]
fn absolute_calls_draw_what_hand_written_naplps_draws() {
    // pointAt: POINT (Absolute), 2/6.
    let painted = draw(&paint(&Program::new().call(Paint::POINT_AT, &[479, 459])));
    let written = draw(&reference(|out| {
        out.push(0x26);
        dot(479, 459, out);
    }));
    assert_eq!(painted.pixels(), written.pixels());
    assert_eq!((lit(&painted), pixel(&painted, 479, 459)), (1, WHITE));

    // moveTo then lineTo: POINT SET (Absolute) 2/4 and LINE (Absolute) 2/8.
    let painted = draw(&paint(&Program::new().call(Paint::MOVE_TO, &[3, 5]).call(Paint::LINE_TO, &[100, 50])));
    let written = draw(&reference(|out| {
        out.push(0x24);
        dot(3, 5, out);
        out.push(0x28);
        dot(100, 50, out);
    }));
    assert_eq!(painted.pixels(), written.pixels());
    assert!(lit(&painted) >= 98);

    // polyline: SET and LINE (Absolute) 2/10 for the first pair, LINE for the rest.
    let painted = draw(&paint(&Program::new().points(Paint::POLYLINE, &[(10, 10), (60, 10), (60, 90), (20, 200)])));
    let written = draw(&reference(|out| {
        out.push(0x2A);
        dot(10, 10, out);
        dot(60, 10, out);
        out.push(0x28);
        dot(60, 90, out);
        dot(20, 200, out);
    }));
    assert_eq!(painted.pixels(), written.pixels());

    // polygon and fillPolygon: SET and POLYGON 3/6 and 3/7, the first
    // vertex absolute and each next a step.
    let corners = [(100, 100), (160, 110), (140, 180), (90, 150)];
    for (call, opcode) in [(Paint::POLYGON, 0x36), (Paint::FILL_POLYGON, 0x37)] {
        let painted = draw(&paint(&Program::new().points(call, &corners.map(|(x, y)| (x as i32, y as i32)))));
        let written = draw(&reference(|out| {
            out.push(opcode);
            dot(corners[0].0, corners[0].1, out);
            for pair in corners.windows(2) {
                between(pair[0], pair[1], out);
            }
        }));
        assert_eq!(painted.pixels(), written.pixels(), "{call:?}");
        assert_eq!(pixel(&painted, 125, 135) == WHITE, call == Paint::FILL_POLYGON, "{call:?}: the middle");
    }

    // arc and fillArc: ARC 2/12 and 2/13 from the drawing point through
    // a middle point to an end point.
    for (call, opcode) in [(Paint::ARC, 0x2C), (Paint::FILL_ARC, 0x2D)] {
        let painted = draw(&paint(&Program::new().call(Paint::MOVE_TO, &[200, 300]).call(call, &[250, 250, 300, 300])));
        let written = draw(&reference(|out| {
            out.push(0x24);
            dot(200, 300, out);
            out.push(opcode);
            between((200, 300), (250, 250), out);
            between((250, 250), (300, 300), out);
        }));
        assert_eq!(painted.pixels(), written.pixels(), "{call:?}");
        assert!((249..=251).any(|y| pixel(&painted, 250, y) == WHITE), "{call:?}: the arc passes through its middle point");
        assert!((0..245).all(|y| pixel(&painted, 250, y) == 0), "{call:?}: and no higher");
        assert_eq!(pixel(&painted, 250, 280) == WHITE, call == Paint::FILL_ARC, "{call:?}: inside the chord");
    }

    // circle and disc: an ARC of one step, the circle on that diameter.
    for (call, opcode) in [(Paint::CIRCLE, 0x2C), (Paint::DISC, 0x2D)] {
        let painted = draw(&paint(&Program::new().call(call, &[240, 230, 40])));
        let written = draw(&reference(|out| {
            out.push(0x24);
            dot(200, 230, out);
            out.push(opcode);
            between((200, 230), (280, 230), out);
        }));
        assert_eq!(painted.pixels(), written.pixels(), "{call:?}");
        assert_eq!(
            (pixel(&painted, 200, 230), pixel(&painted, 280, 230), pixel(&painted, 199, 230), pixel(&painted, 281, 230)),
            (WHITE, WHITE, 0, 0),
            "{call:?}"
        );
        assert_eq!(pixel(&painted, 240, 230) == WHITE, call == Paint::DISC, "{call:?}: the centre");
    }
}

/// Section 7.3, the relative forms: a shape written in steps from the
/// drawing point is the shape written at absolute places, dot for dot,
/// however many steps it has, because each step is taken from where the
/// receiver's drawing point is.
#[test]
fn relative_calls_draw_what_their_absolute_forms_draw() {
    // Three hundred steps of one dot land where one line of 300 dots does.
    let mut steps = Program::new().call(Paint::MOVE_TO, &[10, 10]);
    for _ in 0..300 {
        steps = steps.call(Paint::LINE_BY, &[1, 1]);
    }
    let line = Program::new().call(Paint::MOVE_TO, &[10, 10]).call(Paint::LINE_TO, &[310, 310]);
    let stepped = draw(&paint(&steps));
    assert_eq!(stepped.pixels(), draw(&paint(&line)).pixels());
    assert_eq!((lit(&stepped), pixel(&stepped, 310, 310)), (301, WHITE));

    let by = Program::new().call(Paint::MOVE_TO, &[10, 10]).points(Paint::POLYLINE_BY, &[(50, 0), (0, 80), (-40, 110)]);
    let to = Program::new().points(Paint::POLYLINE, &[(10, 10), (60, 10), (60, 90), (20, 200)]);
    assert_eq!(draw(&paint(&by)).pixels(), draw(&paint(&to)).pixels());

    for (stepped, named) in [(Paint::POLYGON_BY, Paint::POLYGON), (Paint::FILL_POLYGON_BY, Paint::FILL_POLYGON)] {
        let by = Program::new().call(Paint::MOVE_TO, &[100, 100]).points(stepped, &[(60, 10), (-20, 70), (-50, -30)]);
        let to = Program::new().points(named, &[(100, 100), (160, 110), (140, 180), (90, 150)]);
        assert_eq!(draw(&paint(&by)).pixels(), draw(&paint(&to)).pixels(), "{stepped:?}");
    }

    let by = Program::new().call(Paint::MOVE_TO, &[400, 400]).call(Paint::MOVE_BY, &[-10, 5]).call(Paint::POINT_BY, &[3, 3]);
    let canvas = draw(&paint(&by));
    assert_eq!((lit(&canvas), pixel(&canvas, 393, 408)), (1, WHITE));
}

/// Section 7.3: `pen` is the logical pel in dots, hanging right and down
/// from the drawing point; `stroke` the line texture; `fillStyle` the
/// fill pattern; `inkOn` fills a texture's gaps with its background.
#[test]
fn the_pen_and_the_textures_draw_as_the_standard_says() {
    let canvas = draw(&paint(&Program::new().call(Paint::PEN, &[3, 2]).call(Paint::POINT_AT, &[100, 100])));
    for y in 95..110 {
        for x in 95..110 {
            assert_eq!(pixel(&canvas, x, y) == WHITE, (100..103).contains(&x) && (100..102).contains(&y), "dot ({x}, {y})");
        }
    }
    assert_eq!(lit(&canvas), 6);

    // Dashed (ANSI X3.110-1983 section 5.3.2.4.2): three pels on, three off.
    // (A line's last pel is always drawn, so the lines here run past what is read.)
    let dashed = draw(&paint(&Program::new().call(Paint::STROKE, &[2]).call(Paint::MOVE_TO, &[0, 9]).call(Paint::LINE_TO, &[14, 9])));
    let row: Vec<bool> = (0..12).map(|x| pixel(&dashed, x, 9) == WHITE).collect();
    assert_eq!(row, [true, true, true, false, false, false, true, true, true, false, false, false]);
    // Dotted: one on, one off; solid again afterwards writes TEXTURE again.
    let dotted = draw(&paint(
        &Program::new()
            .call(Paint::STROKE, &[1])
            .call(Paint::MOVE_TO, &[0, 9])
            .call(Paint::LINE_BY, &[8, 0])
            .call(Paint::STROKE, &[0])
            .call(Paint::MOVE_TO, &[0, 20])
            .call(Paint::LINE_BY, &[8, 0]),
    ));
    assert_eq!((0..8).map(|x| pixel(&dotted, x, 9) == WHITE).collect::<Vec<_>>(), [true, false, true, false, true, false, true, false]);
    assert!((0..8).all(|x| pixel(&dotted, x, 20) == WHITE));

    // Horizontal hatching: every line of the rectangle is drawn whole or
    // left whole, and both happen; in colour mode 2 the gaps are the
    // background entry.
    for (program, gap) in
        [(Program::new().call(Paint::FILL_STYLE, &[2]), 0), (Program::new().call(Paint::INK_ON, &[i32::from(WHITE), 4]).call(Paint::FILL_STYLE, &[2]), 4)]
    {
        let canvas = draw(&paint(&program.call(Paint::FILL_RECT, &[40, 40, 20, 8])));
        let rows: Vec<u8> = (40..48)
            .map(|y| {
                let first = pixel(&canvas, 40, y);
                assert!((40..60).all(|x| pixel(&canvas, x, y) == first), "line {y} is one colour");
                first
            })
            .collect();
        assert!(rows.contains(&WHITE) && rows.contains(&gap) && rows.iter().all(|entry| *entry == WHITE || *entry == gap), "{rows:?}");
    }
    // Vertical hatching is the same turned.
    let canvas = draw(&paint(&Program::new().call(Paint::FILL_STYLE, &[1]).call(Paint::FILL_RECT, &[40, 40, 8, 20])));
    let columns: Vec<u8> = (40..48).map(|x| pixel(&canvas, x, 45)).collect();
    assert!(columns.contains(&WHITE) && columns.contains(&0), "{columns:?}");

    // Highlight: a filled shape is outlined in black.
    let canvas = draw(&paint(&Program::new().call(Paint::INK, &[2]).call(Paint::HIGHLIGHT, &[1]).call(Paint::FILL_RECT, &[100, 100, 20, 20])));
    assert_eq!((pixel(&canvas, 100, 100), pixel(&canvas, 110, 110)), (0, 2));
}

/// Section 7.3: the painter writes only what differs. A second call in
/// the same ink, pen and texture writes its own PDI and nothing else, and
/// a drawing point already where a call starts is not set again.
#[test]
fn only_what_differs_is_written() {
    let one = paint_as(Target::Backdrop, &Program::new().call(Paint::INK, &[3]).call(Paint::MOVE_TO, &[10, 10]).call(Paint::LINE_TO, &[20, 10])).unwrap();
    let two = paint_as(
        Target::Backdrop,
        &Program::new()
            .call(Paint::INK, &[3])
            .call(Paint::MOVE_TO, &[10, 10])
            .call(Paint::LINE_TO, &[20, 10])
            .call(Paint::INK, &[3])
            .call(Paint::PEN, &[1, 1])
            .call(Paint::STROKE, &[0])
            .call(Paint::MOVE_TO, &[20, 10])
            .call(Paint::LINE_TO, &[30, 10]),
    )
    .unwrap();
    // The second line costs LINE and one operand: seven bytes.
    assert_eq!(two.len(), one.len() + 7, "{one:02X?}\n{two:02X?}");
    // Calls that only set the pen write nothing at all.
    let none =
        paint_as(Target::Backdrop, &Program::new().call(Paint::INK, &[3]).call(Paint::MOVE_TO, &[10, 10]).call(Paint::PEN, &[4, 4]).call(Paint::STROKE, &[2]))
            .unwrap();
    assert_eq!(none, Vec::<u8>::new());
}

/// Section 7.3, `trace` and `fillTrace`: INCREMENTAL LINE 3/10 and
/// INCREMENTAL POLYGON 3/11, a chain of moves of one step.
#[test]
fn a_trace_is_a_chain_of_moves() {
    let traced = draw(&paint(&Program::new().call(Paint::MOVE_TO, &[10, 10]).call(Paint::TRACE, &[2, 3]).text("xxxyyyd")));
    let lines = draw(&paint(&Program::new().points(Paint::POLYLINE, &[(10, 10), (16, 10), (16, 19), (18, 22)])));
    assert_eq!(traced.pixels(), lines.pixels());
    // The pen is where the moves ended: a line from there starts there.
    let onward = draw(&paint(&Program::new().call(Paint::MOVE_TO, &[10, 10]).call(Paint::TRACE, &[2, 3]).text("xxxyyyd").call(Paint::LINE_BY, &[10, 0])));
    assert_eq!((pixel(&onward, 28, 22), pixel(&onward, 29, 22)), (WHITE, 0));
    // Pen up, pen down and a reversed direction.
    let gapped = draw(&paint(&Program::new().call(Paint::MOVE_TO, &[100, 100]).call(Paint::TRACE, &[4, 4]).text("xuxnx<x")));
    let row: Vec<bool> = (100..=112).map(|x| pixel(&gapped, x, 100) == WHITE).collect();
    assert_eq!(row, [true, true, true, true, true, false, false, false, true, true, true, true, true]);
    // A filled trace is the polygon its moves outline.
    let filled = draw(&paint(&Program::new().call(Paint::MOVE_TO, &[200, 200]).call(Paint::FILL_TRACE, &[10, 10]).text("xxxyyy<xxx")));
    assert_eq!(
        (pixel(&filled, 215, 215), pixel(&filled, 200, 200), pixel(&filled, 230, 230), pixel(&filled, 235, 215), pixel(&filled, 215, 235)),
        (WHITE, WHITE, WHITE, 0, 0)
    );
    assert_eq!(
        paint_as(Target::Backdrop, &Program::new().call(Paint::FILL_TRACE, &[1, 1]).text("xu")),
        Err(Fault::Type("a filled trace has no pen to lift".into()))
    );
    assert!(matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TRACE, &[1, 1]).text("xq")), Err(Fault::Type(_))));
}

/// Section 7.3, `image`: FIELD 3/8 and INCREMENTAL POINT 3/9, a bitmap of
/// colour-map entries, each pel one dot, the first at the image's top left.
#[test]
fn an_image_puts_each_pel_on_its_dot() {
    let (width, height) = (7_i32, 5_i32);
    let pels: Vec<i32> = (0..width * height).map(|index| 1 + index % 15).collect();
    for (x, y) in [(5, 7), (0, 0), (473, 455), (200, 300)] {
        let canvas = draw(&paint(&Program::new().call(Paint::IMAGE, &[x, y, width, height]).list(&pels)));
        for row in 0..HEIGHT {
            for column in 0..WIDTH {
                let (dx, dy) = (column as i32 - x, row as i32 - y);
                let want = if (0..width).contains(&dx) && (0..height).contains(&dy) { pels[(dy * width + dx) as usize] as u8 } else { 0 };
                assert_eq!(pixel(&canvas, column, row), want, "image at ({x}, {y}): dot ({column}, {row})");
            }
        }
    }
    // The tallest and the widest an image may be keep their rows and columns.
    let tall: Vec<i32> = (0..128 * 3).map(|index| 1 + (index / 3) % 2).collect();
    let canvas = draw(&paint(&Program::new().call(Paint::IMAGE, &[10, 100, 3, 128]).list(&tall)));
    for row in 0..128 {
        assert_eq!(pixel(&canvas, 11, 100 + row), 1 + (row % 2) as u8, "row {row}");
    }
    assert_eq!((pixel(&canvas, 11, 99), pixel(&canvas, 11, 228)), (0, 0));
    let wide: Vec<i32> = (0..480).map(|index| 1 + index % 2).collect();
    let canvas = draw(&paint(&Program::new().call(Paint::IMAGE, &[0, 50, 480, 1]).list(&wide)));
    assert!((0..480).all(|x| pixel(&canvas, x, 50) == 1 + (x % 2) as u8));
    // The pen is the script's again afterwards: the image's own pel is not kept.
    let after = draw(&paint(&Program::new().call(Paint::PEN, &[2, 2]).call(Paint::IMAGE, &[5, 7, 1, 1]).list(&[3]).call(Paint::POINT_AT, &[50, 50])));
    assert_eq!((pixel(&after, 51, 51), lit(&after)), (WHITE, 5));
    // Refusals: the wrong number of pels, a pel that is no entry, too many.
    assert!(matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::IMAGE, &[0, 0, 2, 2]).list(&[1, 2, 3])), Err(Fault::Type(_))));
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::IMAGE, &[0, 0, 1, 1]).list(&[16])), Err(Fault::Range(message)) if message.starts_with("NAPLPS_COLOURS"))
    );
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::IMAGE, &[0, 0, 480, 128]).list(&vec![1; 16_385])), Err(Fault::Range(message)) if message.starts_with("NAPLPS_MAX_IMAGE_PELS"))
    );
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::IMAGE, &[0, 0, 1, 129]).list(&[1; 129])), Err(Fault::Range(message)) if message.starts_with("NAPLPS_MAX_IMAGE_HEIGHT"))
    );
}

/// RAS-8, the test every writer of NAPLPS text must pass, for section
/// 7.3's `text`: a page of rows 1 to 23 of 40 characters each draws every
/// character's Noto cell on exactly its 12 by 20 dots, in the drawing
/// colour on the field's background, and nothing outside the cells.
#[test]
fn text_on_the_grid_puts_one_noto_cell_on_each_cell() {
    let mut program = Program::new().call(Paint::INK_ON, &[i32::from(WHITE), 4]);
    let mut rows: Vec<String> = Vec::new();
    for row in 1..=23_u8 {
        // The letters and digits, which every Latin set holds.
        let text: String = (0..40_u8).map(|column| char::from(LETTERS[usize::from(row + column) % LETTERS.len()])).collect();
        program = program.call(Paint::TEXT, &[i32::from(row), 0, 0]).text("").text(&text);
        rows.push(text);
    }
    let canvas = draw(&paint(&program));
    for (index, text) in rows.iter().enumerate() {
        let runs = neoteletext_script::host::naplps::writer::runs(text, "en").unwrap();
        let [Run::Sets(stretches)] = runs.as_slice() else { panic!("one run of the provided sets: {runs:?}") };
        let codes: Vec<(u8, u8)> = stretches.iter().flat_map(|stretch| stretch.codes.iter().map(|code| (stretch.set, *code))).collect();
        assert_eq!(codes.len(), 40);
        for (column, (set, code)) in codes.into_iter().enumerate() {
            let glyph = naplps_drawn::glyph(set, code);
            for (line, bits) in glyph.iter().enumerate() {
                for dot in 0..12_u32 {
                    let inked = bits & (1 << (11 - dot)) != 0;
                    let (x, y) = (column as u32 * 12 + dot, index as u32 * 20 + line as u32);
                    assert_eq!(pixel(&canvas, x, y), if inked { WHITE } else { 4 }, "row {} column {column} ({set}, {code:#04X}): dot ({x}, {y})", index + 1);
                }
            }
        }
    }
}

/// Section 7.3, `text`'s sizes (RAS-8 item 5): double height is a cell
/// wide and two rows tall, drawn from the upper row; double size two
/// cells wide and two rows tall.
#[test]
fn double_height_and_double_size_text_fill_their_cells() {
    for (size, wide) in [(1, 12), (2, 24)] {
        let canvas = draw(&paint(&Program::new().call(Paint::INK_ON, &[i32::from(WHITE), 4]).call(Paint::TEXT, &[3, 5, size]).text("").text("AB")));
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let inside = (60..60 + 2 * wide).contains(&x) && (40..80).contains(&y);
                assert_eq!(pixel(&canvas, x, y) != 0, inside, "size {size}: dot ({x}, {y})");
            }
        }
        // Each dot of the glyph is two lines tall.
        assert!((40..80).step_by(2).all(|y| (60..60 + 2 * wide).all(|x| pixel(&canvas, x, y) == pixel(&canvas, x, y + 1))));
    }
    // A row with no row beneath it, and text past the last column, are refused.
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[23, 0, 1]).text("").text("A")), Err(Fault::Range(message)) if message.starts_with("LAST_TEXT_ROW"))
    );
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 38, 0]).text("").text("ABC")), Err(Fault::Range(message)) if message.starts_with("GRID_COLUMNS"))
    );
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 20, 2]).text("").text("ABCDEFGHIJK")), Err(Fault::Range(message)) if message.starts_with("GRID_COLUMNS"))
    );
}

/// Section 7.3: text never moves the pen (the painter owns the drawing
/// point), and two texts written at one place both draw there.
#[test]
fn text_leaves_the_drawing_point_alone() {
    let program = Program::new().call(Paint::MOVE_TO, &[100, 300]).call(Paint::TEXT, &[2, 4, 0]).text("").text("HELLO").call(Paint::LINE_BY, &[10, 0]);
    let canvas = draw(&paint(&program));
    assert!((100..=110).all(|x| pixel(&canvas, x, 300) == WHITE));
    assert_eq!(pixel(&canvas, 111, 300), 0);
    // The same cell written twice: the second, in another colour, covers the first.
    let twice = Program::new()
        .call(Paint::INK_ON, &[1, 4])
        .call(Paint::TEXT, &[2, 4, 0])
        .text("")
        .text("HELLO")
        .call(Paint::INK_ON, &[2, 4])
        .call(Paint::TEXT, &[2, 4, 0])
        .text("")
        .text("HELLO");
    let canvas = draw(&paint(&twice));
    let cells: Vec<u8> = (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).map(|(x, y)| pixel(&canvas, x, y)).filter(|entry| *entry != 0).collect();
    assert_eq!(cells.len(), 5 * 12 * 20);
    assert!(cells.iter().all(|entry| *entry == 2 || *entry == 4) && cells.contains(&2));
}

/// Section 7.3, `textAt`: the character field in dots, rotation, path,
/// spacing and row spacing as TEXT 2/2 carries them, underline and
/// reverse as the C1 controls.
#[test]
fn free_text_takes_its_field_and_attributes() {
    let options =
        |width, height, rotation, path, spacing, rows, underline, reverse| [50, 100, width, height, rotation, path, spacing, rows, underline, reverse];
    // A field of 24 by 40 dots fills exactly that, twice, for two characters.
    let canvas =
        draw(&paint(&Program::new().call(Paint::INK_ON, &[i32::from(WHITE), 4]).call(Paint::TEXT_AT, &options(24, 40, 0, 0, 0, 0, 0, 0)).text("").text("AB")));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            assert_eq!(pixel(&canvas, x, y) != 0, (50..98).contains(&x) && (100..140).contains(&y), "dot ({x}, {y})");
        }
    }
    // The path down puts the second character beneath the first.
    let canvas =
        draw(&paint(&Program::new().call(Paint::INK_ON, &[i32::from(WHITE), 4]).call(Paint::TEXT_AT, &options(12, 20, 0, 3, 0, 0, 0, 0)).text("").text("AB")));
    assert!(pixel(&canvas, 55, 125) != 0 && pixel(&canvas, 67, 105) == 0);
    // Spacing of three halves leaves half a field between characters.
    let canvas =
        draw(&paint(&Program::new().call(Paint::INK_ON, &[i32::from(WHITE), 4]).call(Paint::TEXT_AT, &options(12, 20, 0, 0, 2, 0, 0, 0)).text("").text("AB")));
    assert!(pixel(&canvas, 70, 105) != 0 && pixel(&canvas, 82, 105) == 0);
    // The fixed-format bytes: spacing, path and rotation; the drawing
    // point leading the cursor, and row spacing.
    let bytes = paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT_AT, &options(12, 20, 1, 2, 3, 2, 1, 1)).text("").text("A")).unwrap();
    let at = bytes.iter().position(|byte| *byte == 0x22).unwrap();
    assert_eq!(bytes[at + 1..at + 3], [0x40 | (3 << 4) | (2 << 2) | 1, 0x40 | (0b10 << 2) | 2]);
    // Underline and reverse on (ESC 5/9, ESC 4/8), and off again for text without them.
    let both = paint_as(
        Target::Backdrop,
        &Program::new()
            .call(Paint::TEXT_AT, &options(12, 20, 0, 0, 0, 0, 1, 1))
            .text("")
            .text("A")
            .call(Paint::TEXT_AT, &options(12, 20, 0, 0, 0, 0, 0, 0))
            .text("")
            .text("B"),
    )
    .unwrap();
    let escapes: Vec<u8> = both.windows(2).filter(|pair| pair[0] == 0x1B && [0x48, 0x49, 0x59, 0x5A].contains(&pair[1])).map(|pair| pair[1]).collect();
    assert_eq!(escapes, [0x59, 0x48, 0x5A, 0x49]);
    let underlined = draw(&paint(&Program::new().call(Paint::TEXT_AT, &options(12, 20, 0, 0, 0, 0, 1, 0)).text("").text(" ")));
    assert!(lit(&underlined) >= 12, "an underlined space draws its underline");
}

/// Section 7.3, "Text and languages": the host designates NeoTeletext's
/// sets by their private designations (REQ-6668) for the language, writes
/// right-to-left text in visual order and leaves the plain Arabic-script
/// letters for the renderer to join; a character no set of the language
/// holds is a `TypeError`.
#[test]
fn text_is_written_in_the_sets_of_its_language() {
    // Greek is set 17: ESC 2/8 2/1 3/1 into G0.
    let greek = paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 0, 0]).text("el").text("αβγ")).unwrap();
    assert!(greek.windows(4).any(|bytes| bytes == [0x1B, 0x28, 0x21, 0x31]), "{greek:02X?}");
    assert!(lit(&draw(&paint(&Program::new().call(Paint::TEXT, &[1, 0, 0]).text("el").text("αβγ")))) > 30);
    // Persian is the plain Arabic-script set 44, ESC 2/8 2/2 3/12, which
    // the renderer joins: the joined word differs from its letters apart.
    let persian = Program::new().call(Paint::TEXT, &[1, 0, 0]).text("fa").text("سلام");
    let bytes = paint_as(Target::Backdrop, &persian).unwrap();
    assert!(bytes.windows(4).any(|window| window == [0x1B, 0x28, 0x22, 0x3C]), "{bytes:02X?}");
    let joined = draw(&paint(&persian));
    let apart = draw(&paint(&Program::new().call(Paint::TEXT, &[1, 0, 0]).text("fa").text("س ل ا م")));
    assert!(lit(&joined) > 30 && joined.pixels() != apart.pixels());
    // The page's own language is used where a call names none.
    let own = paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 0, 0]).text("").text("A")).unwrap();
    let named = paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 0, 0]).text("en").text("A")).unwrap();
    assert_eq!(own, named);
    // Refusals.
    assert!(matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 0, 0]).text("").text("漢")), Err(Fault::Type(_))));
    assert!(matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 0, 0]).text("xx-nowhere").text("A")), Err(Fault::Type(_))));
    assert!(matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT, &[1, 0, 0]).text("").text("A\u{7}")), Err(Fault::Type(_))));
    let long: String = "A".repeat(1_001);
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::TEXT_AT, &[0, 0, 1, 1, 0, 0, 0, 0, 0, 0]).text("").text(&long)), Err(Fault::Range(message)) if message.starts_with("MAX_TEXT_CHARS"))
    );
}

/// Section 7.3, `wait`: WAIT 3/13 with its format byte, the tenths summed
/// from interval bytes of six bits; the backdrop's alone.
#[test]
fn a_wait_is_played_by_the_receiver() {
    let waits = |tenths: i32| -> Vec<u32> {
        let bytes = paint(&Program::new().call(Paint::FILL_RECT, &[0, 0, 4, 4]).call(Paint::WAIT, &[tenths]).call(Paint::FILL_RECT, &[8, 0, 4, 4]));
        naplps_drawn::events(&bytes).into_iter().filter_map(|event| if let Event::Wait(tenths) = event { Some(tenths) } else { None }).collect()
    };
    assert_eq!(waits(5), [5]);
    assert_eq!(waits(63), [63]);
    assert_eq!(waits(130), [130]);
    assert_eq!(waits(600), [600]);
    assert!(
        matches!(paint_as(Target::Backdrop, &Program::new().call(Paint::WAIT, &[601])), Err(Fault::Range(message)) if message.starts_with("WAIT_BUDGET_TICKS"))
    );
    assert!(matches!(paint_as(Target::Symbol, &Program::new().call(Paint::WAIT, &[1])), Err(Fault::Type(_))));
}

/// Shapes drawn from a place: what the test below draws stamped and painted.
type Shape<'a> = &'a dyn Fn(Program, i32, i32) -> Program;

/// Section 7.4's "drawn entirely in relative forms from its own origin",
/// which this package's painter provides for work package N2: a symbol's
/// contents hold no absolute opcode, and defined as a macro and invoked
/// at a point they draw what the backdrop draws at that place. Points,
/// lines, rectangles and images land on their dots exactly wherever the
/// symbol is stamped. An arc's or a polygon's edge is computed by the
/// renderer from places finer than a dot, where a stamped symbol's steps
/// may be one part in 131,072 of the screen from the backdrop's places:
/// the renderer may then round a circle's radius or an edge the other
/// way, and the shape is the same shape with its edge a dot away.
#[test]
fn a_symbols_contents_are_relative_and_stamp_anywhere() {
    let exact = |program: Program, x: i32, y: i32| {
        program
            .points(Paint::POLYLINE_BY, &[(6, -4), (6, 4), (6, -4)])
            .call(Paint::FILL_RECT, &[x + 2, y + 10, 8, 6])
            .call(Paint::LINE_TO, &[x + 20, y + 20])
            .call(Paint::POINT_AT, &[x + 23, y])
            .call(Paint::IMAGE, &[x + 14, y + 40, 2, 2])
            .list(&[1, 2, 3, 4])
            .call(Paint::RECT, &[x, y + 50, 24, 8])
    };
    let curved = |program: Program, x: i32, y: i32| {
        program
            .call(Paint::CIRCLE, &[x + 12, y + 30, 6])
            .points(Paint::FILL_POLYGON, &[(x, y + 40), (x + 10, y + 40), (x + 5, y + 48)])
            .call(Paint::MOVE_TO, &[x, y + 70])
            .call(Paint::FILL_ARC, &[x + 10, y + 60, x + 20, y + 70])
    };
    // A circle of radius 6 has an edge of some 44 dots, a triangle and a
    // half disc as many again.
    let shapes: [(Shape<'_>, usize); 2] = [(&exact, 0), (&curved, 64)];
    for (shape, may_differ) in shapes {
        // In a symbol a place is a place from its own origin.
        let body = paint_as(Target::Symbol, &shape(Program::new().call(Paint::MOVE_BY, &[0, 6]), 0, 0)).unwrap();
        // Outside operands (bytes 4/0 to 7/15) every byte from 2/0 to 3/15 is an opcode.
        let absolute: Vec<u8> = body.iter().copied().filter(|byte| [0x24, 0x26, 0x28, 0x2A, 0x2B, 0x2E, 0x2F, 0x32, 0x33, 0x36, 0x37].contains(byte)).collect();
        assert_eq!(absolute, Vec::<u8>::new(), "{body:02X?}");
        for (x, y) in [(100, 200), (0, 0), (333, 71), (455, 380)] {
            // DEF MACRO (ESC 4/0) of macro 2/0, END (ESC 4/5); POINT SET at the
            // place; the macro set into G3 and the macro by SS3.
            let mut stream = opening();
            stream.extend_from_slice(&[0x1B, 0x40, 0x20]);
            stream.extend_from_slice(&body);
            stream.extend_from_slice(&[0x1B, 0x45, SO, 0x3E, 0x40 | (WHITE << 2), 0x24]);
            dot(i64::from(x), i64::from(y), &mut stream);
            stream.extend_from_slice(&[0x1B, 0x2B, 0x7A, 0x1D, 0x20]);
            let stamped = draw(&stream);
            // The same calls on the backdrop, each place moved to the stamp's.
            let moved = draw(&paint(&shape(Program::new().call(Paint::MOVE_TO, &[x, y + 6]), x, y)));
            let differing = stamped.pixels().iter().zip(moved.pixels()).filter(|(one, other)| one != other).count();
            assert!(differing <= may_differ, "stamped at ({x}, {y}): {differing} dots differ");
            assert!(lit(&stamped) > 60 && lit(&stamped).abs_diff(lit(&moved)) <= 24, "{} dots stamped, {} painted", lit(&stamped), lit(&moved));
        }
    }
}

/// Section 7.3, "Text and languages": ASCII that the language's own sets
/// do not hold (teletext's English set has an arrow at ASCII's square
/// bracket) is written from the primary set, in the same cells.
#[test]
fn ascii_the_languages_sets_lack_is_written_from_the_primary_set() {
    let runs = neoteletext_script::host::naplps::writer::runs("a[b]{c}", "en").unwrap();
    assert!(
        matches!(runs.as_slice(), [Run::Sets(_), Run::Primary(open), Run::Sets(_), Run::Primary(close), Run::Sets(_), Run::Primary(last)] if open == b"[" && close == b"]{" && last == b"}"),
        "{runs:?}"
    );
    assert_eq!(neoteletext_script::host::naplps::writer::cells(&runs), 7);
    let canvas = draw(&paint(&Program::new().call(Paint::TEXT, &[2, 3, 0]).text("").text("[x]")));
    let inked =
        |column: u32| (20..40).flat_map(|y| (column * 12..column * 12 + 12).map(move |x| (x, y))).filter(|(x, y)| pixel(&canvas, *x, *y) == WHITE).count();
    assert!(inked(3) > 5 && inked(4) > 5 && inked(5) > 5, "each of the three cells holds its character");
    assert_eq!(lit(&canvas), inked(3) + inked(4) + inked(5), "and nothing is drawn outside them");
    // A text that begins with such a character, and one that is only that.
    assert!(lit(&draw(&paint(&Program::new().call(Paint::TEXT, &[2, 3, 0]).text("").text("[")))) > 5);
    // The drawing goes on after it: the PDI set is shifted out again.
    let after = draw(&paint(&Program::new().call(Paint::TEXT, &[2, 3, 0]).text("").text("[").call(Paint::FILL_RECT, &[100, 100, 4, 4])));
    assert_eq!(pixel(&after, 102, 102), WHITE);
}

/// Section 10.2, "the host checks every word ... and refuses the whole
/// program on the first fault": a program that ends inside a call, names
/// no call, or passes a bound is refused by a `TypeError` or a
/// `RangeError` that begins with the bound's name.
#[test]
fn a_faulty_program_is_refused_whole() {
    let refused = |program: Program| paint_as(Target::Backdrop, &program).unwrap_err();
    assert!(matches!(refused(Program::words(&[Paint::MOVE_TO.number(), 1])), Fault::Type(_)));
    assert!(matches!(refused(Program::words(&[0])), Fault::Type(_)));
    assert!(matches!(refused(Program::words(&[36])), Fault::Type(_)));
    assert!(matches!(refused(Program::words(&[i32::MIN])), Fault::Type(_)));
    assert!(matches!(refused(Program::words(&[Paint::POLYLINE.number(), 3, 1, 1])), Fault::Type(_)));
    assert!(matches!(refused(Program::words(&[Paint::POLYLINE.number(), -1])), Fault::Type(_)));
    assert!(matches!(refused(Program::words(&[Paint::POLYLINE.number(), i32::MAX])), Fault::Range(message) if message.starts_with("MAX_POINTS")));
    assert!(matches!(refused(Program::words(&[Paint::POLYGON.number(), 4_097])), Fault::Range(message) if message.starts_with("MAX_VERTICES")));
    assert!(matches!(refused(Program::new().call(Paint::MOVE_TO, &[480, 0])), Fault::Range(message) if message.starts_with("MAX_X")));
    assert!(matches!(refused(Program::new().call(Paint::MOVE_TO, &[0, 920])), Fault::Range(message) if message.starts_with("MAX_Y")));
    assert!(matches!(refused(Program::new().call(Paint::MOVE_TO, &[i32::MIN, i32::MAX])), Fault::Range(_)));
    assert!(matches!(refused(Program::new().call(Paint::MOVE_TO, &[470, 0]).call(Paint::LINE_BY, &[i32::MAX, 0])), Fault::Range(_)));
    // A step longer than the screen is wide, which no operand holds; the same line to a named place is drawn.
    assert!(
        matches!(refused(Program::new().call(Paint::MOVE_TO, &[-480, 0]).call(Paint::LINE_BY, &[959, 0])), Fault::Range(message) if message.starts_with("MAX_X"))
    );
    assert!(paint_as(Target::Backdrop, &Program::new().call(Paint::MOVE_TO, &[-480, 0]).call(Paint::LINE_TO, &[479, 0])).is_ok());
    assert!(matches!(refused(Program::new().call(Paint::FILL_RECT, &[0, 0, 0, 5])), Fault::Range(message) if message.starts_with("CANVAS_WIDTH")));
    assert!(matches!(refused(Program::new().call(Paint::FILL_RECT, &[0, 0, 5, 461])), Fault::Range(message) if message.starts_with("CANVAS_HEIGHT")));
    assert!(matches!(refused(Program::new().call(Paint::INK, &[16])), Fault::Range(message) if message.starts_with("NAPLPS_COLOURS")));
    assert!(matches!(refused(Program::new().call(Paint::INK_ON, &[1, -1])), Fault::Range(_)));
    assert!(matches!(refused(Program::new().call(Paint::PEN, &[0, 1])), Fault::Range(_)));
    assert!(matches!(refused(Program::new().call(Paint::STROKE, &[4])), Fault::Type(_)));
    assert!(matches!(refused(Program::new().call(Paint::FILL_STYLE, &[8])), Fault::Type(_)));
    assert!(matches!(refused(Program::new().call(Paint::HIGHLIGHT, &[2])), Fault::Type(_)));
    assert!(matches!(refused(Program::new().call(Paint::MASK, &[129, 1])), Fault::Range(message) if message.starts_with("MASK_SIDE")));
    assert!(matches!(refused(Program::new().call(Paint::CIRCLE, &[10, 10, 0])), Fault::Range(_)));
    // The handles of definitions are work package N2's: none is live here.
    assert_eq!(refused(Program::new().call(Paint::FILL_STYLE, &[4, 1])), Fault::Type("no such texture".into()));
    assert_eq!(refused(Program::new().call(Paint::GLYPH, &[1])), Fault::Type("no such DRCS character".into()));
    assert_eq!(refused(Program::new().call(Paint::STAMP, &[1, 0, 0])), Fault::Type("no such symbol".into()));
    // A string's words that are no characters, and one longer than its bound.
    assert!(matches!(refused(Program::words(&[Paint::TEXT.number(), 1, 0, 0, 0, 1, 0xD800])), Fault::Type(_)));
    assert!(matches!(refused(Program::words(&[Paint::TEXT.number(), 1, 0, 0, 17])), Fault::Range(message) if message.starts_with("MAX_LANGUAGE_BYTES")));
    // The room a painting is given is held to, by the bound's name.
    let mut painter = Painter::backdrop();
    let mut out = Vec::new();
    let big = Program::new().points(Paint::POLYLINE, &vec![(1, 1); 1_024]);
    assert!(
        matches!(painter.run(big.as_slice(), "en", &NoDefinitions, &mut out, 100, "NAPLPS_MAX_BYTES_PER_CALLBACK"), Err(Fault::Range(message)) if message.starts_with("NAPLPS_MAX_BYTES_PER_CALLBACK"))
    );
}

/// The places a coordinate may name are all places a six-byte operand
/// holds: none is moved by the operand's range (the bounds `MIN_X` to
/// `MAX_Y`).
#[test]
fn every_allowed_place_is_one_an_operand_holds() {
    use neoteletext_script::host::naplps::limits::{MAX_X, MAX_Y, MIN_X, MIN_Y};
    let full = 1_i64 << 17;
    let top = i64::from(DISPLAY_FIRST_LINE);
    for (x, y) in [(MIN_X, MIN_Y), (MAX_X, MAX_Y), (MIN_X, MAX_Y), (MAX_X, MIN_Y)] {
        let (across, up) = dot_steps(i64::from(x), i64::from(y) + top, 1, 6, Rounding::Nearest);
        let (next_across, next_up) = dot_steps(i64::from(x) + i64::from(x.signum()), i64::from(y) + top + i64::from(y.signum()), 1, 6, Rounding::Nearest);
        assert!((-full..full).contains(&across) && (-full..full).contains(&up), "({x}, {y})");
        // Not held: one dot further out is a different value, or is the range's own end.
        assert!(across != next_across || across == -full, "({x}, {y}) across");
        assert!(up != next_up, "({x}, {y}) up");
    }
}

/// Whole numbers at the edges of the painter's ranges, as the fuzz
/// target `painter_programs` draws them.
const EDGES: [i32; 20] = [0, 1, -1, 2, 3, 15, 16, 63, 64, 128, 459, 460, 479, 480, -480, 600, 919, 1_024, 4_096, i32::MAX];

/// Bytes from a fixed seed, read a piece at a time (Knuth's MMIX linear
/// congruential generator: The Art of Computer Programming, volume 2,
/// section 3.3.4, Table 1, line 26).
struct Chance(u64);

impl Chance {
    fn byte(&mut self) -> u8 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 56) as u8
    }

    fn int(&mut self) -> i32 {
        let choice = self.byte();
        match choice {
            0..100 => i32::from(choice),
            100..180 => EDGES[usize::from(choice) % EDGES.len()],
            180..220 => -i32::from(self.byte()),
            _ => i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()]),
        }
    }
}

/// Section 11.4, target 22, as a test that runs with the others: what
/// the fuzz target `painter_programs` asserts of arbitrary programs holds
/// for some thousands made from a fixed seed. A program is taken or
/// refused whole without a panic; one that is taken stays within its
/// room and is NAPLPS the renderer reads with no byte passed over and
/// nothing refused but for its pixel work.
#[test]
fn programs_made_by_chance_are_refused_or_written_as_naplps_the_renderer_reads() {
    let mut chance = Chance(22);
    let (mut taken, mut refused, mut drew) = (0_u32, 0_u32, 0_u32);
    for round in 0..6_000_u32 {
        let target = [Target::Backdrop, Target::Symbol, Target::Pattern][usize::from(chance.byte()) % 3];
        let room = [64, 4_096, 65_536][usize::from(chance.byte()) % 3];
        let language = ["en", "", "fa", "no-such-language"][usize::from(chance.byte()) % 4];
        let mut words = Vec::new();
        // Mostly calls with what they take, in small numbers, so that
        // programs get past their first call; one call in eight is
        // followed by whatever chance gives.
        for _ in 0..=(chance.byte() % 6) {
            let (_, call) = Paint::ALL[usize::from(chance.byte()) % Paint::ALL.len()];
            words.push(call.number());
            let wild = chance.byte().is_multiple_of(8);
            let number = |chance: &mut Chance, most: u8| if wild { chance.int() } else { i32::from(chance.byte() % most) };
            let (count, most) = match call {
                Paint::INK | Paint::STROKE | Paint::FILL_STYLE | Paint::HIGHLIGHT | Paint::GLYPH => (1, 4),
                Paint::WAIT => (1, 100),
                Paint::RECT | Paint::FILL_RECT | Paint::ARC | Paint::FILL_ARC => (4, 200),
                Paint::CIRCLE | Paint::DISC | Paint::STAMP => (3, 100),
                Paint::TEXT => (3, 3),
                Paint::IMAGE => (2, 200),
                Paint::TEXT_AT | Paint::POLYLINE | Paint::POLYLINE_BY | Paint::POLYGON | Paint::FILL_POLYGON | Paint::POLYGON_BY | Paint::FILL_POLYGON_BY => {
                    (0, 1)
                }
                _ => (2, 100),
            };
            for _ in 0..count {
                words.push(number(&mut chance, most));
            }
            let length = usize::from(chance.byte() % 12);
            let text = |chance: &mut Chance, words: &mut Vec<i32>| {
                words.push(length as i32);
                words.extend((0..length).map(|_| i32::from(b"xyd<^un aZ[{"[usize::from(chance.byte()) % 12])));
            };
            match call {
                Paint::POLYLINE | Paint::POLYLINE_BY | Paint::POLYGON | Paint::FILL_POLYGON | Paint::POLYGON_BY | Paint::FILL_POLYGON_BY => {
                    words.push(length as i32);
                    words.extend((0..length * 2).map(|_| number(&mut chance, 40)));
                }
                Paint::TRACE | Paint::FILL_TRACE => text(&mut chance, &mut words),
                Paint::IMAGE => {
                    let (wide, tall) = (1 + i32::from(chance.byte() % 5), 1 + i32::from(chance.byte() % 5));
                    words.extend([wide, tall, wide * tall]);
                    words.extend((0..wide * tall).map(|_| number(&mut chance, 16)));
                }
                Paint::TEXT => {
                    words.push(0);
                    text(&mut chance, &mut words);
                }
                Paint::TEXT_AT => {
                    words.extend([number(&mut chance, 200), number(&mut chance, 200), 1 + number(&mut chance, 30), 1 + number(&mut chance, 30)]);
                    words.extend((0..4).map(|_| number(&mut chance, 4)));
                    words.extend((0..2).map(|_| number(&mut chance, 2)));
                    words.push(0);
                    text(&mut chance, &mut words);
                }
                _ => {}
            }
        }
        let mut stream = opening();
        let mut painter = if target == Target::Backdrop { Painter::backdrop() } else { Painter::definition(target) };
        painter.end(&mut Vec::new());
        let mut out = Vec::new();
        if painter.run(&words, language, &NoDefinitions, &mut out, room, "ROOM").is_err() {
            refused += 1;
            continue;
        }
        taken += 1;
        drew += u32::from(out.len() > 1);
        assert!(out.len() <= room, "round {round}: {} bytes in a room of {room}", out.len());
        stream.extend_from_slice(&out);
        let mut validator = naplps::Validator::new();
        validator.feed(&stream);
        validator.finish();
        let diagnostics = validator.diagnostics();
        assert_eq!(diagnostics.passed_over, 0, "round {round}: {words:?}\n{stream:02X?}\n{diagnostics:?}");
        for kind in naplps::Refusal::ALL {
            assert!(kind == naplps::Refusal::WorkLimit || diagnostics.count(kind) == 0, "round {round}: {kind:?} {words:?}\n{stream:02X?}");
        }
    }
    // The programs are not all refused, nor all empty.
    assert!(taken > 1_000 && drew > 800 && refused > 1_000, "{taken} taken, {drew} drew, {refused} refused");
}
