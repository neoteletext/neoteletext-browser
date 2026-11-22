//! The sixel bitmap at the host call (script-api.md section 5.7 and
//! operations 130 to 138 of section 10.2; work package T3): what each
//! operation does to the back buffer, that nothing shows before
//! `present()`, what `present()` stages for the decoder and counts, how
//! the bitmap's triplets share the page's with the overlay, and for each
//! bound that the refusal names it and nothing changed.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "tests")]

use neoteletext_core::consts::{
    ALPHA_YELLOW, COLUMNS, MODE_BACKGROUND_COLOUR, MODE_FOREGROUND_COLOUR, MODE_SET_ACTIVE_POSITION, MODE_TERMINATION, MOSAIC_RED, ROW_ADDRESS_BASE,
    ROW_ADDRESS_LAST, SPACE,
};
use protocol::script::{Batch, ContextFlags, ContextId, ContextOpen, Level, Renderer};

use super::{BITMAP_CELLS, BITMAP_HEIGHT, BITMAP_WIDTH, MAX_BITMAP_COORDINATE, NO_ROWS, PRESENT_LOSSY_FACTOR};
use crate::host::{Host, Payload, Reply};
use crate::ops::{EventKind, Op};
use crate::random::SEED_BYTES;
use crate::teletext::NONE;
use crate::teletext::limits::LOCAL_TRIPLETS;

const PAGE: i32 = 2;
const CHANNEL: i32 = 1;

fn opening(context: ContextId, level: Level) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer: Renderer::Teletext,
        level,
        page: 0x101,
        subcode: 0,
        flags: ContextFlags::default(),
        tick_divisor: 1,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/101.js".into(),
        script_url: "https://example.test/chan/101.js".into(),
        channel_title: "Example".into(),
        seed: [7; SEED_BYTES],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: 1,
    }
}

/// A host with both contexts open on a teletext page shown at `level`,
/// in a callback of the page context at tick 0.
fn host_at(level: Level) -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, level), 1);
    host.open(opening(ContextId::Page, level), 1);
    host.begin_step(0, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
    host
}

fn host() -> Host {
    host_at(Level::TwoFive)
}

fn call(host: &mut Host, op: Op, ints: &[i32]) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::None)
}

/// A call that must succeed with nothing to give.
#[track_caller]
fn done(host: &mut Host, op: Op, ints: &[i32]) {
    assert_eq!(call(host, op, ints), Reply::Undefined, "{op:?} {ints:?}");
}

fn get(host: &mut Host, x: i32, y: i32) -> Reply {
    call(host, Op::BM_GET, &[x, y])
}

/// `present()`: the cells changed and the cells that lost detail.
#[track_caller]
fn present(host: &mut Host) -> (i32, i32) {
    match call(host, Op::BM_PRESENT, &[]) {
        Reply::Int(packed) => (packed % PRESENT_LOSSY_FACTOR, packed / PRESENT_LOSSY_FACTOR),
        other => panic!("present gave {other:?}"),
    }
}

/// The step's batch, written as the message a script host sends and read
/// back, so that what the bitmap stages is always a message the link
/// takes.
#[track_caller]
fn batch(host: &mut Host) -> Batch {
    use protocol::script::{FromScriptHost, read_to_decoder, write_to_decoder};
    let batch = host.take_batch(1, &[]);
    let mut written = Vec::new();
    write_to_decoder(&mut written, &FromScriptHost::Batch(Box::new(batch.clone()))).unwrap_or_else(|error| panic!("{error:?}\n{batch:#?}"));
    assert_eq!(read_to_decoder(&mut written.as_slice()).unwrap(), Some(FromScriptHost::Batch(Box::new(batch.clone()))));
    batch
}

/// The rows a batch carries, by number.
fn rows(batch: &Batch) -> Vec<u8> {
    batch.teletext_rows.iter().flatten().map(|row| row.row).collect()
}

/// The codes a batch carries for `row`.
#[track_caller]
fn codes(batch: &Batch, row: u8) -> [u8; COLUMNS] {
    batch.teletext_rows.iter().flatten().find(|held| held.row == row).unwrap_or_else(|| panic!("row {row} did not leave")).codes
}

/// A batch's enhancement data as (address, mode, data).
fn triplets(batch: &Batch) -> Vec<(u8, u8, u8)> {
    batch.teletext_enhancement.clone().unwrap_or_default().iter().map(|triplet| (triplet.address, triplet.mode, triplet.data)).collect()
}

/// The column triplets of `row` in enhancement data, by (column, mode,
/// data): those between the row's Set Active Position triplet and the
/// next row address.
fn row_triplets(all: &[(u8, u8, u8)], row: u8) -> Vec<(u8, u8, u8)> {
    all.iter()
        .skip_while(|triplet| **triplet != (ROW_ADDRESS_BASE + row, MODE_SET_ACTIVE_POSITION, 0))
        .skip(1)
        .take_while(|(address, _, _)| *address < ROW_ADDRESS_BASE)
        .copied()
        .collect()
}

/// Asserts the next batch holds no teletext section: nothing was staged
/// since the last batch was taken.
#[track_caller]
fn unchanged(host: &mut Host) {
    let batch = host.take_batch(9, &[]);
    assert_eq!((batch.teletext_rows, batch.teletext_enhancement, batch.teletext_settings), (None, None, None));
}

#[track_caller]
fn range_refused(reply: &Reply, bound: &str) {
    match reply {
        Reply::Range(message) => assert!(message.starts_with(&format!("{bound}: ")), "{message}"),
        other => panic!("not a RangeError naming {bound}: {other:?}"),
    }
}

#[track_caller]
fn type_refused(reply: &Reply, starts: &str) {
    match reply {
        Reply::Type(message) => assert!(message.starts_with(starts), "{message}"),
        other => panic!("not a TypeError: {other:?}"),
    }
}

fn budget(host: &mut Host, entry: &str) -> (usize, usize) {
    let Reply::Text(json) = call(host, Op::BUDGET, &[]) else { panic!("no budget") };
    let after = json.split(&format!("\"{entry}\":{{\"used\":")).nth(1).unwrap_or_else(|| panic!("no {entry} in {json}"));
    let used = after.split(',').next().unwrap().parse().unwrap();
    let limit = after.split("\"limit\":").nth(1).unwrap().split('}').next().unwrap().parse().unwrap();
    (used, limit)
}

/// Section 5.7, "the script draws into the back buffer; nothing shows
/// until `present()`": drawing stages nothing; the first present takes
/// rows 1 to 23 and sends the rows that are not spaces; a present after
/// one sixel changed counts one cell and sends that cell's row alone; and
/// a present of an unchanged buffer counts and sends nothing.
#[test]
fn nothing_shows_until_present_and_then_only_the_rows_that_changed_leave() {
    let mut host = host();
    // Red over the cells of rows 3 and 4, columns 2 to 5.
    done(&mut host, Op::BM_RECT, &[4, 6, 8, 6, 1, 1]);
    assert_eq!(get(&mut host, 4, 6), Reply::Int(1));
    unchanged(&mut host);

    let (cells, lossy) = present(&mut host);
    assert_eq!((cells, lossy), (8, 0));
    let first = batch(&mut host);
    assert_eq!(rows(&first), vec![3, 4]);
    // The cells before the block are empty, so its colour, Mosaic Red,
    // goes in one of them at no loss, and the block's cells are whole
    // mosaics (0x7F).
    let row = codes(&first, 3);
    assert_eq!(row.iter().filter(|code| **code != SPACE).copied().collect::<Vec<u8>>(), vec![MOSAIC_RED, 0x7F, 0x7F, 0x7F, 0x7F]);
    assert!(row[..2].contains(&MOSAIC_RED) && row[2..6] == [0x7F; 4], "{row:?}");
    assert_eq!(first.teletext_enhancement, None);

    // One more sixel, in row 10.
    done(&mut host, Op::BM_SET, &[40, 28, 2]);
    assert_eq!(present(&mut host), (1, 0));
    assert_eq!(rows(&batch(&mut host)), vec![10]);

    assert_eq!(present(&mut host), (0, 0));
    unchanged(&mut host);
}

/// Section 5.7's colours and its rule for Levels 2.5 and 3.5: a defined
/// entry is set by X/26 foreground and background triplets, written with
/// the row's Set Active Position triplet and the Termination Marker; the
/// page's triplets in use are the whole of that data. At Level 1 the same
/// bitmap is drawn in the nearest of the eight colours and stages no
/// enhancement data.
#[test]
fn a_defined_colour_is_triplets_at_level_2_5_and_one_of_the_eight_at_level_1() {
    let draw = |host: &mut Host| {
        // Row 3, column 5: the left sixels in entry 17, the right in 16.
        done(host, Op::BM_RECT, &[10, 6, 1, 3, 17, 1]);
        done(host, Op::BM_RECT, &[11, 6, 1, 3, 16, 1]);
    };
    let mut host = host();
    draw(&mut host);
    assert_eq!(present(&mut host), (1, 0));
    let shown = batch(&mut host);
    let all = triplets(&shown);
    assert_eq!(all.first(), Some(&(ROW_ADDRESS_BASE + 3, MODE_SET_ACTIVE_POSITION, 0)));
    assert_eq!(all.last().map(|triplet| (triplet.0, triplet.1)), Some((ROW_ADDRESS_LAST, MODE_TERMINATION)));
    let colours: Vec<(u8, u8, u8)> =
        row_triplets(&all, 3).into_iter().filter(|(column, mode, _)| *column == 5 && [MODE_FOREGROUND_COLOUR, MODE_BACKGROUND_COLOUR].contains(mode)).collect();
    // A tie goes to the lower entry as the background (the cell rule).
    assert_eq!(colours, vec![(5, MODE_FOREGROUND_COLOUR, 17), (5, MODE_BACKGROUND_COLOUR, 16)]);
    assert_eq!(budget(&mut host, "triplets"), (all.len(), LOCAL_TRIPLETS));

    let mut level_1 = host_at(Level::One);
    draw(&mut level_1);
    let (cells, _) = present(&mut level_1);
    assert_eq!(cells, 1);
    let shown = batch(&mut level_1);
    assert_eq!(rows(&shown), vec![3]);
    assert_eq!(shown.teletext_enhancement, None);
    assert_eq!(budget(&mut level_1, "triplets").0, 0);
}

/// Section 5.7, "the overlay lies over both", and DEC-7360 item 6: the
/// bitmap's triplets and the overlay's are one stream in row and column
/// order. An overlay cell over a bitmap row states its own colour where
/// it starts, and the bitmap's colour is stated again after it.
#[test]
fn the_overlay_lies_over_a_bitmap_row_in_one_stream() {
    let mut host = host();
    // The whole of row 3 in entry 17.
    done(&mut host, Op::BM_RECT, &[0, 6, 80, 3, 17, 1]);
    assert_eq!(present(&mut host), (40, 0));
    let before = triplets(&batch(&mut host));
    let backgrounds = |all: &[(u8, u8, u8)]| row_triplets(all, 3).into_iter().filter(|(_, mode, _)| *mode == MODE_BACKGROUND_COLOUR).collect::<Vec<_>>();
    assert_eq!(backgrounds(&before), vec![(0, MODE_BACKGROUND_COLOUR, 17)]);

    done(&mut host, Op::TT_OVERLAY_FILL, &[3, 10, 1, 2, NONE, 20]);
    let after = triplets(&batch(&mut host));
    assert_eq!(backgrounds(&after), vec![(0, MODE_BACKGROUND_COLOUR, 17), (10, MODE_BACKGROUND_COLOUR, 20), (12, MODE_BACKGROUND_COLOUR, 17)]);
    assert_eq!(budget(&mut host, "triplets").0, after.len());
    // A present with the overlay there changes nothing of the bitmap's.
    assert_eq!(present(&mut host), (0, 0));
    unchanged(&mut host);
}

/// Fills the back buffer so that every cell holds two defined colours
/// that differ from its neighbours': a page no Level 1 row can show and
/// that needs two colour triplets a cell.
fn busy(host: &mut Host) {
    let mut data = vec![0_u8; BITMAP_WIDTH * BITMAP_HEIGHT];
    for (at, byte) in data.iter_mut().enumerate() {
        let (x, y) = (at % BITMAP_WIDTH, at / BITMAP_WIDTH);
        let cell = x / 2 + y / 3;
        *byte = if x % 2 == 0 { 16 + u8::try_from(cell % 8).unwrap() } else { 24 + u8::try_from(cell % 8).unwrap() };
    }
    let reply = host.call(PAGE, Op::BM_BLIT.number(), &[0, 0, 80, 69, 255], Payload::Bytes(&data));
    assert_eq!(reply, Reply::Undefined);
}

/// Section 5.7, "within what `LOCAL_TRIPLETS` leaves", and DEC-7360 item
/// 6: the bitmap takes what the page's triplets have left and never
/// more. A page whose bitmap wants far more colour than 208 triplets
/// hold is shown within them, the cells that could not be shown counted
/// as lossy and reported in the budget; with the overlay holding some of
/// the page's triplets the bitmap is given fewer and loses more, and the
/// overlay's cells stay.
#[test]
fn the_bitmap_takes_only_the_triplets_the_page_has_left() {
    let mut host = host();
    busy(&mut host);
    let (cells, lossy_alone) = present(&mut host);
    assert!(cells > 0 && lossy_alone > 0, "{cells} cells, {lossy_alone} lossy");
    let alone = triplets(&batch(&mut host));
    assert!(!alone.is_empty() && alone.len() <= LOCAL_TRIPLETS, "{} triplets", alone.len());
    assert_eq!(budget(&mut host, "triplets").0, alone.len());
    assert_eq!(budget(&mut host, "bitmapCellsLossy"), (usize::try_from(lossy_alone).unwrap(), BITMAP_CELLS));

    // The same bitmap on a page whose overlay already holds 121 triplets:
    // forty cells of row 23, three triplets a cell (a character and its
    // two colours, each differing from the cell before), and the row's
    // address.
    let mut shared = self::host();
    for column in 0..40 {
        let reply = shared.call(PAGE, Op::TT_OVERLAY_PUT.number(), &[23, column, 1 + column % 7, 16 + column % 16, 0, NONE], Payload::Text(b"x"));
        assert_eq!(reply, Reply::Undefined);
    }
    let overlay_alone = budget(&mut shared, "triplets").0;
    assert!(overlay_alone > 100, "{overlay_alone}");
    busy(&mut shared);
    let (_, lossy_shared) = present(&mut shared);
    let both = triplets(&batch(&mut shared));
    assert!(both.len() <= LOCAL_TRIPLETS, "{} triplets", both.len());
    assert!(lossy_shared > lossy_alone, "{lossy_shared} lossy beside the overlay, {lossy_alone} alone");
    // Every overlay cell's character is still in the stream.
    let characters = row_triplets(&both, 23).iter().filter(|(_, _, data)| *data == b'x').count();
    assert_eq!(characters, 40);

    // And the other way: the overlay asks for more once the bitmap holds
    // the page's triplets, is refused by the page's bound, and nothing
    // changed.
    let mut full = self::host();
    busy(&mut full);
    let _ = present(&mut full);
    let held = triplets(&batch(&mut full));
    let mut refusals = 0;
    for column in 0..40 {
        let reply = full.call(PAGE, Op::TT_OVERLAY_PUT.number(), &[23, column, 1 + column % 7, 16 + column % 16, 0, NONE], Payload::Text(b"x"));
        if reply != Reply::Undefined {
            range_refused(&reply, "LOCAL_TRIPLETS");
            refusals += 1;
        }
    }
    assert!(refusals > 0);
    let after = budget(&mut full, "triplets").0;
    assert!(after >= held.len() && after <= LOCAL_TRIPLETS, "{after}");
}

/// Section 5.7's `rows` and "how the models layer": the bitmap and
/// `page` write the same plane, each on the rows it owns. A row that
/// changes owner is cleared; while the bitmap owns it every operation of
/// the page plane that would write it is a `TypeError` and writes
/// nothing; `release()` gives every row back.
#[test]
fn a_row_is_the_bitmaps_or_the_pages_and_is_cleared_when_it_changes_owner() {
    let mut host = host();
    let put = |host: &mut Host, row: i32, height: i32| host.call(PAGE, Op::TT_PUT.number(), &[row, 4, NONE, NONE, height, 0, NONE], Payload::Text(b"page"));
    assert_eq!(put(&mut host, 5, 0), Reply::Int(4));
    assert_eq!(put(&mut host, 8, 0), Reply::Int(4));
    assert_eq!(rows(&batch(&mut host)), vec![5, 8]);

    // The bitmap takes rows 5 and 6: row 5 is spaces again, row 8 is not
    // touched.
    done(&mut host, Op::BM_ROWS, &[5, 6]);
    let taken = batch(&mut host);
    assert_eq!(rows(&taken), vec![5]);
    assert_eq!(codes(&taken, 5), [SPACE; COLUMNS]);

    let theirs = "rows 5 to 6 are the bitmap's";
    type_refused(&put(&mut host, 5, 0), theirs);
    type_refused(&put(&mut host, 6, 0), theirs);
    // Double height on row 4 would hide row 5.
    type_refused(&put(&mut host, 4, 2), theirs);
    type_refused(&host.call(PAGE, Op::TT_CODES.number(), &[6, 0], Payload::Bytes(b"A")), theirs);
    type_refused(&host.call(PAGE, Op::TT_BLIT.number(), &[], Payload::Bytes(&[SPACE; 920])), theirs);
    type_refused(&call(&mut host, Op::TT_CLEAR, &[1, 23]), theirs);
    type_refused(&call(&mut host, Op::TT_CLEAR, &[6, 9]), theirs);
    type_refused(&call(&mut host, Op::TT_SCROLL, &[1, 5, 1]), theirs);
    type_refused(&call(&mut host, Op::TT_SETTINGS, &[NONE, NONE, 1]), "black background colour substitution stays off while the bitmap owns rows 5 to 6");
    unchanged(&mut host);
    // The rows that are the page's are the page's.
    assert_eq!(put(&mut host, 4, 0), Reply::Int(4));
    done(&mut host, Op::TT_CLEAR, &[7, 23]);
    done(&mut host, Op::TT_SETTINGS, &[4, NONE, 0]);
    assert_eq!(rows(&batch(&mut host)), vec![4, 8]);

    // The bitmap shows only on its rows, wherever the script drew.
    done(&mut host, Op::BM_CLEAR, &[2]);
    let (cells, lossy) = present(&mut host);
    assert_eq!((cells, lossy), (80, 0));
    assert_eq!(rows(&batch(&mut host)), vec![5, 6]);

    // Moving the bitmap to rows 6 and 7 clears row 5, which it gives
    // back, and row 7, which it takes; row 6 stays as shown.
    done(&mut host, Op::BM_ROWS, &[6, 7]);
    let moved = batch(&mut host);
    assert_eq!(rows(&moved), vec![5]);
    assert_eq!(codes(&moved, 5), [SPACE; COLUMNS]);
    assert_eq!(put(&mut host, 5, 0), Reply::Int(4));
    let _ = batch(&mut host);

    // Released, every row is the page's and the bitmap's rows are spaces.
    done(&mut host, Op::BM_ROWS, &[NO_ROWS, NO_ROWS]);
    let released = batch(&mut host);
    assert_eq!(rows(&released), vec![6]);
    assert_eq!(codes(&released, 6), [SPACE; COLUMNS]);
    assert_eq!(put(&mut host, 6, 0), Reply::Int(4));
    done(&mut host, Op::TT_SETTINGS, &[NONE, NONE, 1]);

    // Under black background colour substitution the bitmap takes no
    // row, by `rows` or by a first `present()` (the engine's DEC-7355).
    let substituted = "the bitmap owns no row while black background colour substitution is on";
    let _ = batch(&mut host);
    type_refused(&call(&mut host, Op::BM_ROWS, &[1, 2]), substituted);
    type_refused(&call(&mut host, Op::BM_PRESENT, &[]), substituted);
    unchanged(&mut host);
    assert_eq!(put(&mut host, 1, 0), Reply::Int(4));

    range_refused(&call(&mut host, Op::BM_ROWS, &[0, 5]), "TELETEXT_ROWS");
    range_refused(&call(&mut host, Op::BM_ROWS, &[1, 24]), "TELETEXT_ROWS");
    range_refused(&call(&mut host, Op::BM_ROWS, &[5, 4]), "TELETEXT_ROWS");
}

/// Section 5.7's drawing calls and section 10.1 rules 1 to 3: `set`,
/// `get`, `line`, `rect` and `blit` write the back buffer, clipped to
/// the bitmap; a colour that is not 0 to 7 or 16 to 31, a coordinate past
/// `MAX_BITMAP_COORDINATE`, data of the wrong length or of the wrong
/// kind is refused by name, and a refused call has drawn nothing.
#[test]
fn drawing_is_clipped_and_every_bound_is_named_and_changes_nothing() {
    let mut host = host();
    done(&mut host, Op::BM_SET, &[79, 68, 7]);
    assert_eq!(get(&mut host, 79, 68), Reply::Int(7));
    for (x, y) in [(-1, 0), (80, 0), (0, -1), (0, 69), (i32::MAX, i32::MIN)] {
        done(&mut host, Op::BM_SET, &[x, y, 7]);
        assert_eq!(get(&mut host, x, y), Reply::Undefined);
    }
    for colour in [-1, 8, 15, 32, 255, i32::MAX] {
        range_refused(&call(&mut host, Op::BM_SET, &[0, 0, colour]), "BITMAP_COLOURS");
        range_refused(&call(&mut host, Op::BM_CLEAR, &[colour]), "BITMAP_COLOURS");
        range_refused(&call(&mut host, Op::BM_LINE, &[0, 0, 5, 5, colour]), "BITMAP_COLOURS");
        range_refused(&call(&mut host, Op::BM_RECT, &[0, 0, 5, 5, colour, 1]), "BITMAP_COLOURS");
    }
    assert_eq!(get(&mut host, 0, 0), Reply::Int(0));

    // A line draws both its ends, in any direction, and is clipped.
    done(&mut host, Op::BM_LINE, &[5, 5, 0, 0, 3]);
    done(&mut host, Op::BM_LINE, &[-MAX_BITMAP_COORDINATE, 10, MAX_BITMAP_COORDINATE, 10, 4]);
    done(&mut host, Op::BM_LINE, &[70, MAX_BITMAP_COORDINATE, 70, -MAX_BITMAP_COORDINATE, 5]);
    assert_eq!([get(&mut host, 0, 0), get(&mut host, 5, 5), get(&mut host, 3, 3), get(&mut host, 6, 6)], [3, 3, 3, 0].map(Reply::Int));
    assert_eq!([get(&mut host, 0, 10), get(&mut host, 79, 10), get(&mut host, 70, 0), get(&mut host, 70, 68)], [4, 4, 5, 5].map(Reply::Int));
    for ends in [[MAX_BITMAP_COORDINATE + 1, 0, 0, 0], [0, -MAX_BITMAP_COORDINATE - 1, 0, 0], [0, 0, i32::MAX, 0], [0, 0, 0, i32::MIN]] {
        range_refused(&call(&mut host, Op::BM_LINE, &[ends[0], ends[1], ends[2], ends[3], 1]), "MAX_BITMAP_COORDINATE");
    }

    // A rectangle, filled and as an outline one sixel thick.
    done(&mut host, Op::BM_RECT, &[20, 20, 5, 4, 6, 1]);
    done(&mut host, Op::BM_RECT, &[30, 20, 5, 5, 1, 0]);
    done(&mut host, Op::BM_RECT, &[75, 65, 100, 100, 2, 1]);
    done(&mut host, Op::BM_RECT, &[40, 40, 0, 9, 2, 1]);
    assert_eq!([get(&mut host, 20, 20), get(&mut host, 24, 23), get(&mut host, 25, 23), get(&mut host, 24, 24)], [6, 6, 0, 0].map(Reply::Int));
    assert_eq!([get(&mut host, 30, 20), get(&mut host, 34, 24), get(&mut host, 32, 20), get(&mut host, 32, 22)], [1, 1, 1, 0].map(Reply::Int));
    assert_eq!([get(&mut host, 79, 68), get(&mut host, 74, 68), get(&mut host, 40, 40)], [2, 0, 0].map(Reply::Int));
    range_refused(&call(&mut host, Op::BM_RECT, &[0, 0, -1, 5, 1, 1]), "MAX_BITMAP_COORDINATE");
    range_refused(&call(&mut host, Op::BM_RECT, &[0, 0, 5, MAX_BITMAP_COORDINATE + 1, 1, 1]), "MAX_BITMAP_COORDINATE");
    type_refused(&call(&mut host, Op::BM_RECT, &[50, 50, 5, 5, 1, 2]), "fill is true or false");
    assert_eq!(get(&mut host, 50, 50), Reply::Int(0));

    // A blit: one byte a sixel, the transparent byte leaving its sixel.
    let blit = |host: &mut Host, ints: [i32; 5], data: &[u8]| host.call(PAGE, Op::BM_BLIT.number(), &ints, Payload::Bytes(data));
    assert_eq!(blit(&mut host, [50, 50, 2, 2, 255], &[1, 255, 255, 2]), Reply::Undefined);
    assert_eq!([get(&mut host, 50, 50), get(&mut host, 51, 50), get(&mut host, 50, 51), get(&mut host, 51, 51)], [1, 0, 0, 2].map(Reply::Int));
    assert_eq!(blit(&mut host, [50, 50, 2, 1, 9], &[9, 3]), Reply::Undefined);
    assert_eq!([get(&mut host, 50, 50), get(&mut host, 51, 50)], [1, 3].map(Reply::Int));
    assert_eq!(blit(&mut host, [-1, -1, 2, 2, 255], &[7, 7, 7, 6]), Reply::Undefined);
    assert_eq!([get(&mut host, 0, 0), get(&mut host, 1, 0)], [6, 0].map(Reply::Int));
    range_refused(&blit(&mut host, [60, 60, 2, 2, 255], &[1, 1, 1]), "BITMAP_BLIT");
    range_refused(&blit(&mut host, [60, 60, 2, 2, 255], &[1, 1, 1, 1, 1]), "BITMAP_BLIT");
    range_refused(&blit(&mut host, [60, 60, 2, 2, 256], &[1, 1, 1, 1]), "BITMAP_BLIT");
    range_refused(&blit(&mut host, [60, 60, 2, 2, 255], &[1, 1, 1, 8]), "BITMAP_COLOURS");
    range_refused(&blit(&mut host, [60, 60, -2, -2, 255], &[1, 1, 1, 1]), "MAX_BITMAP_COORDINATE");
    range_refused(&blit(&mut host, [MAX_BITMAP_COORDINATE + 1, 60, 1, 1, 255], &[1]), "MAX_BITMAP_COORDINATE");
    assert_eq!(get(&mut host, 60, 60), Reply::Int(0));
    type_refused(&host.call(PAGE, Op::BM_BLIT.number(), &[0, 0, 1, 1, 255], Payload::Text(b"1")), "operation BM_BLIT takes a Uint8Array");
    type_refused(&call(&mut host, Op::BM_SET, &[0, 0]), "operation BM_SET takes 3 whole numbers");
    type_refused(&host.call(PAGE, Op::BM_PRESENT.number(), &[], Payload::Bytes(b"x")), "operation BM_PRESENT takes no payload");

    // `clear` fills the buffer; none of this was shown.
    done(&mut host, Op::BM_CLEAR, &[16]);
    assert_eq!([get(&mut host, 0, 0), get(&mut host, 79, 68)], [16, 16].map(Reply::Int));
    unchanged(&mut host);
}

/// Section 5.7's `text`: cells reserved for characters hold no sixels
/// until the buffer is cleared, and show their characters when the
/// bitmap is presented; a character is one of the page's own G0 set.
#[test]
fn text_reserves_cells_until_the_buffer_is_cleared() {
    let mut host = host_at(Level::One);
    let text = |host: &mut Host, ints: [i32; 4], words: &str| host.call(PAGE, Op::BM_TEXT.number(), &ints, Payload::Text(words.as_bytes()));
    assert_eq!(text(&mut host, [2, 10, 3, NONE], "HELLO"), Reply::Undefined);
    // Sixels drawn over reserved cells are in the buffer and not shown.
    done(&mut host, Op::BM_RECT, &[20, 3, 10, 3, 1, 1]);
    assert_eq!(present(&mut host), (5, 0));
    let shown = batch(&mut host);
    assert_eq!(rows(&shown), vec![2]);
    // Alpha Yellow in one of the empty cells before the text.
    let row = codes(&shown, 2);
    assert_eq!(row.iter().filter(|code| **code != SPACE).copied().collect::<Vec<u8>>(), vec![ALPHA_YELLOW, b'H', b'E', b'L', b'L', b'O']);
    assert!(row[..10].contains(&ALPHA_YELLOW) && row[10..15] == *b"HELLO", "{row:?}");

    // Cleared, the cells are sixels again.
    done(&mut host, Op::BM_CLEAR, &[0]);
    assert_eq!(present(&mut host), (5, 0));
    assert_eq!(codes(&batch(&mut host), 2), [SPACE; COLUMNS]);

    range_refused(&text(&mut host, [0, 0, NONE, NONE], "x"), "TELETEXT_ROWS");
    range_refused(&text(&mut host, [24, 0, NONE, NONE], "x"), "TELETEXT_ROWS");
    range_refused(&text(&mut host, [1, 40, NONE, NONE], "x"), "COLUMNS");
    range_refused(&text(&mut host, [1, 38, NONE, NONE], "abc"), "COLUMNS");
    range_refused(&text(&mut host, [1, 0, 8, NONE], "x"), "BITMAP_COLOURS");
    range_refused(&text(&mut host, [1, 0, NONE, 15], "x"), "BITMAP_COLOURS");
    type_refused(&text(&mut host, [1, 0, NONE, NONE], "caf\u{e9}"), "a bitmap's text is characters of the page's own G0 set");
    type_refused(&text(&mut host, [1, 0, NONE, NONE], "\u{2603}"), "the page's character sets cannot show");
    type_refused(&host.call(PAGE, Op::BM_TEXT.number(), &[1, 0, NONE, NONE], Payload::None), "operation BM_TEXT takes a string");
    assert_eq!(present(&mut host), (0, 0));
    unchanged(&mut host);
}

/// Section 3.1: renderer state belongs to the page. The channel context
/// draws nothing on the bitmap.
#[test]
fn the_channel_context_draws_nothing_on_the_bitmap() {
    let mut host = host();
    for (op, ints) in [(Op::BM_SET, vec![0, 0, 1]), (Op::BM_GET, vec![0, 0]), (Op::BM_ROWS, vec![1, 2]), (Op::BM_PRESENT, vec![])] {
        type_refused(&host.call(CHANNEL, op.number(), &ints, Payload::None), "only the page context draws");
    }
    unchanged(&mut host);
}
