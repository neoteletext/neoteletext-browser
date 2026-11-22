//! The teletext API at the host call (script-api.md sections 5.1, 5.2,
//! 5.4, 5.5, 5.6, 10.1 and 10.5): what each operation writes to the page
//! plane and to the page's local enhancement data, and for each bound
//! that the refusal names it and nothing changed.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "tests")]

use neoteletext_core::charset::language::language_value;
use neoteletext_core::consts::{
    ALPHA_WHITE, ALPHA_YELLOW, BLACK_BACKGROUND, COLUMNS, CONCEAL, CONTROL_JOINED, DOUBLE_HEIGHT, FLASH, FLASH_MODE_NORMAL, FLASH_MODE_STEADY, HOLD_MOSAICS,
    MODE_ADDITIONAL_FLASH, MODE_BACKGROUND_COLOUR, MODE_CONTROL, MODE_FOREGROUND_COLOUR, MODE_G0_LEVEL_25, MODE_G0_NO_DIACRITIC, MODE_MODIFIED_DESIGNATION,
    MODE_ROW_SET, MODE_SET_ACTIVE_POSITION, MODE_TERMINATION, MOSAIC_RED, NEW_BACKGROUND, ROW_ADDRESS_BASE, ROW_ADDRESS_LAST, SEPARATED_MOSAICS, SPACE,
    START_BOX, TERMINATION_LOCAL_DATA_END,
};
use neoteletext_rules::animate::{Operation, OperationKind};
use neoteletext_rules::table::Target;
use protocol::script::batch::{Label, TeletextRow, TeletextSettings};
use protocol::script::teletext::control_handle;
use protocol::script::{Batch, ContextFlags, ContextId, ContextOpen, Level, Renderer};

use super::limits::{BLIT_CODES, LOCAL_TRIPLETS, MAX_COLOUR_SETS_PER_SECOND, MAX_CONTROLS, TELETEXT_DEFINABLE_COLOURS};
use super::{BOX_SHIFT, CONCEAL_SHIFT, FLASH_SHIFT, HOLD_SHIFT, MOSAIC_SHIFT, NONE, ON, SEPARATED};
use crate::host::{Host, Payload, Reply};
use crate::limits::TICKS_PER_SECOND;
use crate::ops::{EventKind, Op};
use crate::random::SEED_BYTES;

const PAGE: i32 = 2;
const CHANNEL: i32 = 1;
/// The page's language in these tests unless a test names another.
const ENGLISH: &str = "en";

fn opening(context: ContextId, renderer: Renderer, language: &str) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer,
        level: Level::TwoFive,
        page: 0x101,
        subcode: 0,
        flags: ContextFlags::default(),
        tick_divisor: 1,
        language: language.into(),
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

/// A host with both contexts open on a teletext page in `language`, in a
/// callback of the page context at tick 0.
fn host_in(language: &str) -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, Renderer::Teletext, language), 1);
    host.open(opening(ContextId::Page, Renderer::Teletext, language), 1);
    host.begin_step(0, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
    host
}

fn host() -> Host {
    host_in(ENGLISH)
}

fn call(host: &mut Host, op: Op, ints: &[i32]) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::None)
}

fn text(host: &mut Host, op: Op, ints: &[i32], text: &str) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::Text(text.as_bytes()))
}

fn bytes(host: &mut Host, op: Op, ints: &[i32], bytes: &[u8]) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::Bytes(bytes))
}

/// `page.put` in the page's language: ink, paper, height and flags.
fn put(host: &mut Host, row: i32, column: i32, words: &str, style: [i32; 4]) -> Reply {
    let [ink, paper, height, flags] = style;
    text(host, Op::TT_PUT, &[row, column, ink, paper, height, flags, NONE], words)
}

/// A style of no member.
const PLAIN: [i32; 4] = [NONE, NONE, 0, 0];

/// The place of the language `tag` in `nt.teletext.languages()`.
fn language(host: &mut Host, tag: &str) -> i32 {
    let Reply::Text(json) = call(host, Op::TT_LANGUAGES, &[]) else { panic!("no languages") };
    let before = json.split(&format!("\"tag\":\"{tag}\"")).next().unwrap();
    assert!(before.len() < json.len(), "the engine has no language {tag}");
    i32::try_from(before.matches("\"tag\"").count()).unwrap()
}

/// Asserts a refusal: a `RangeError` whose message begins with `bound`
/// and a colon, and nothing staged for the decoder by the call.
#[track_caller]
fn refused(host: &mut Host, reply: &Reply, bound: &str) {
    match reply {
        Reply::Range(message) => assert!(message.starts_with(&format!("{bound}: ")), "{message}"),
        other => panic!("not a RangeError naming {bound}: {other:?}"),
    }
    unchanged(host);
}

/// Asserts a `TypeError` and nothing staged.
#[track_caller]
fn type_refused(host: &mut Host, reply: &Reply) {
    assert!(matches!(reply, Reply::Type(_)), "{reply:?}");
    unchanged(host);
}

/// Asserts the next batch holds no teletext section: nothing changed
/// since the last batch was taken.
#[track_caller]
fn unchanged(host: &mut Host) {
    let batch = host.take_batch(9, &[]);
    assert_eq!((batch.teletext_rows, batch.teletext_enhancement, batch.teletext_settings, batch.animate, batch.labels), (None, None, None, None, None));
}

/// The step's batch, written as the message a script host sends and read
/// back, so that what teletext stages is always a message the link takes.
#[track_caller]
fn batch(host: &mut Host) -> Batch {
    use protocol::script::{FromScriptHost, read_to_decoder, write_to_decoder};
    let batch = host.take_batch(1, &[]);
    let mut written = Vec::new();
    write_to_decoder(&mut written, &FromScriptHost::Batch(Box::new(batch.clone()))).unwrap_or_else(|error| panic!("{error:?}\n{batch:#?}"));
    assert_eq!(read_to_decoder(&mut written.as_slice()).unwrap(), Some(FromScriptHost::Batch(Box::new(batch.clone()))));
    batch
}

fn rows(batch: &Batch) -> Vec<TeletextRow> {
    batch.teletext_rows.clone().unwrap_or_default()
}

/// The first codes of each row of a batch.
fn first_codes(batch: &Batch) -> Vec<(u8, u8)> {
    rows(batch).iter().map(|row| (row.row, row.codes[0])).collect()
}

fn triplets(batch: &Batch) -> Vec<(u8, u8, u8)> {
    batch.teletext_enhancement.clone().unwrap_or_default().iter().map(|triplet| (triplet.address, triplet.mode, triplet.data)).collect()
}

/// The row address triplet that begins `row`'s triplets.
fn at_row(row: u8) -> (u8, u8, u8) {
    (ROW_ADDRESS_BASE + row, MODE_SET_ACTIVE_POSITION, 0)
}

const END: (u8, u8, u8) = (ROW_ADDRESS_LAST, MODE_TERMINATION, TERMINATION_LOCAL_DATA_END);

/// A row of spaces with `codes` from `column`.
fn row_with(column: usize, codes: &[u8]) -> [u8; COLUMNS] {
    let mut row = [SPACE; COLUMNS];
    row[column..column + codes.len()].copy_from_slice(codes);
    row
}

fn budget(host: &mut Host, entry: &str) -> usize {
    let Reply::Text(json) = call(host, Op::BUDGET, &[]) else { panic!("no budget") };
    let after = json.split(&format!("\"{entry}\":{{\"used\":")).nth(1).unwrap_or_else(|| panic!("no {entry} in {json}"));
    after.split(',').next().unwrap().parse().unwrap()
}

/// Section 5.1 and EN 300 706 Table 26: `put` places the spacing
/// attributes its style needs in the cells immediately before the text,
/// one for each member given, and gives the cells written. The worked
/// example's title has its two codes at columns 0 and 1 (section 5.8).
#[test]
fn put_places_its_styles_spacing_attributes_before_the_text() {
    let mut host = host();
    assert_eq!(put(&mut host, 1, 2, "HARBOUR NEWS", [3, NONE, 2, 0]), Reply::Int(14));
    let mut title = vec![DOUBLE_HEIGHT, ALPHA_YELLOW];
    title.extend_from_slice(b"HARBOUR NEWS");
    assert_eq!(rows(&batch(&mut host)), vec![TeletextRow { row: 1, codes: row_with(0, &title) }]);

    // Paper: the colour, then New Background, then the ink that the
    // colour code displaced: here the white the row had.
    assert_eq!(put(&mut host, 3, 5, "X", [NONE, 4, 0, 0]), Reply::Int(4));
    assert_eq!(rows(&batch(&mut host)), vec![TeletextRow { row: 3, codes: row_with(2, &[0x04, NEW_BACKGROUND, ALPHA_WHITE, b'X']) }]);
    // Black paper is Black Background alone.
    assert_eq!(put(&mut host, 4, 5, "X", [NONE, 0, 0, 0]), Reply::Int(2));
    assert_eq!(rows(&batch(&mut host)), vec![TeletextRow { row: 4, codes: row_with(4, &[BLACK_BACKGROUND, b'X']) }]);
    // Every member at once, in the order the host places them: box,
    // flash, mosaic form, hold, the mosaic ink, conceal last since a
    // colour would end it.
    let flags = (ON << FLASH_SHIFT) | (ON << CONCEAL_SHIFT) | (ON << BOX_SHIFT) | (ON << HOLD_SHIFT) | (SEPARATED << MOSAIC_SHIFT);
    assert_eq!(put(&mut host, 6, 10, "", [1, NONE, 0, flags]), Reply::Int(7));
    let style = [START_BOX, START_BOX, FLASH, SEPARATED_MOSAICS, HOLD_MOSAICS, MOSAIC_RED, CONCEAL];
    assert_eq!(rows(&batch(&mut host)), vec![TeletextRow { row: 6, codes: row_with(3, &style) }]);
    // The ink restored after paper is the foreground in force where the
    // attributes begin: after row 1's yellow, yellow.
    assert_eq!(put(&mut host, 1, 30, "Y", [NONE, 1, 0, 0]), Reply::Int(4));
    assert_eq!(rows(&batch(&mut host))[0].codes[27..31], [0x01, NEW_BACKGROUND, ALPHA_YELLOW, b'Y']);
}

/// Section 5.1: a cell needed for an attribute must be free, or the call
/// is `RangeError TELETEXT_ATTRIBUTE_CELL` naming the row and column and
/// nothing is written; putting the same text again finds its own
/// attribute there and is no refusal (the worked example's clock).
#[test]
fn put_refuses_an_attribute_cell_that_is_not_free() {
    let mut host = host();
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[5, 3], b"A"), Reply::Undefined);
    let _ = batch(&mut host);
    let reply = put(&mut host, 5, 4, "x", [1, NONE, 0, 0]);
    assert_eq!(reply, Reply::Range("TELETEXT_ATTRIBUTE_CELL: row 5, column 3 holds the code 65, and the style needs that cell for a spacing attribute".into()));
    refused(&mut host, &reply, "TELETEXT_ATTRIBUTE_CELL");
    // No cell before column 0, and one cell before column 1 where the
    // style needs three.
    let reply = put(&mut host, 7, 0, "x", [1, NONE, 0, 0]);
    refused(&mut host, &reply, "TELETEXT_ATTRIBUTE_CELL");
    let reply = put(&mut host, 7, 1, "x", [NONE, 4, 0, 0]);
    refused(&mut host, &reply, "TELETEXT_ATTRIBUTE_CELL");
    assert_eq!(call(&mut host, Op::TT_AT, &[7, 1]), Reply::Int(i32::from(SPACE)));
    // The clock: the same place every tick.
    for time in ["12:00:00", "12:00:01"] {
        assert_eq!(put(&mut host, 22, 31, time, [7, NONE, 0, 0]), Reply::Int(9));
    }
    assert_eq!(rows(&batch(&mut host)), vec![TeletextRow { row: 22, codes: row_with(30, b"\x0712:00:01") }]);
    // Another attribute in that cell is a refusal.
    let reply = put(&mut host, 22, 31, "12:00:02", [6, NONE, 0, 0]);
    refused(&mut host, &reply, "TELETEXT_ATTRIBUTE_CELL");
}

/// Sections 5.1 and 10.6: each bound of the page plane, by its name, with
/// nothing changed.
#[test]
fn the_page_planes_bounds_are_named_and_change_nothing() {
    let mut host = host();
    for (row, column) in [(0, 0), (24, 0), (-1, 0)] {
        let reply = put(&mut host, row, column, "x", PLAIN);
        refused(&mut host, &reply, "TELETEXT_ROWS");
    }
    let reply = put(&mut host, 1, 40, "x", PLAIN);
    refused(&mut host, &reply, "COLUMNS");
    let reply = put(&mut host, 1, 38, "xyz", PLAIN);
    refused(&mut host, &reply, "COLUMNS");
    for (ink, paper) in [(8, NONE), (NONE, 8), (-2, NONE)] {
        let reply = put(&mut host, 1, 5, "x", [ink, paper, 0, 0]);
        refused(&mut host, &reply, "COLOURS");
    }
    let reply = put(&mut host, 1, 0, &"x".repeat(1025), PLAIN);
    refused(&mut host, &reply, "MAX_TEXT_CHARS");
    // Double height takes the row below, which must be a row of the plane.
    let reply = put(&mut host, 23, 5, "x", [NONE, NONE, 2, 0]);
    refused(&mut host, &reply, "DOUBLE_HEIGHT_LAST_ROW");
    assert_eq!(put(&mut host, 22, 5, "x", [NONE, NONE, 2, 0]), Reply::Int(2));
    let _ = batch(&mut host);
    let reply = put(&mut host, 1, 5, "x", [NONE, NONE, 3, 0]);
    type_refused(&mut host, &reply);
    let reply = put(&mut host, 1, 5, "x", [NONE, NONE, 0, 1 << 10]);
    type_refused(&mut host, &reply);
    let reply = bytes(&mut host, Op::TT_CODES, &[1, 0], &[0x41, 0x80]);
    refused(&mut host, &reply, "MAX_ROW_CODE");
    let reply = bytes(&mut host, Op::TT_CODES, &[1, 39], &[0x41, 0x42]);
    refused(&mut host, &reply, "COLUMNS");
    let reply = bytes(&mut host, Op::TT_BLIT, &[], &vec![SPACE; BLIT_CODES - 1]);
    refused(&mut host, &reply, "BLIT_CODES");
    let mut plane = vec![SPACE; BLIT_CODES];
    plane[BLIT_CODES - 1] = 0xFF;
    let reply = bytes(&mut host, Op::TT_BLIT, &[], &plane);
    refused(&mut host, &reply, "MAX_ROW_CODE");
    let reply = call(&mut host, Op::TT_CLEAR, &[5, 4]);
    refused(&mut host, &reply, "TELETEXT_ROWS");
    let reply = call(&mut host, Op::TT_SCROLL, &[1, 24, 1]);
    refused(&mut host, &reply, "TELETEXT_ROWS");
    let reply = call(&mut host, Op::TT_SETTINGS, &[32, NONE, NONE]);
    refused(&mut host, &reply, "COLOUR_MAP_ENTRIES");
    let reply = call(&mut host, Op::TT_AT, &[0, 0]);
    refused(&mut host, &reply, "TELETEXT_ROWS");
    // A wrong payload or count of numbers is a TypeError.
    let reply = text(&mut host, Op::TT_CODES, &[1, 0], "AB");
    type_refused(&mut host, &reply);
    let reply = call(&mut host, Op::TT_AT, &[1]);
    type_refused(&mut host, &reply);
}

/// Section 5.1: `codes`, `blit`, `at`, `clear` and `scroll`, and that
/// only the rows that changed leave in the step's batch.
#[test]
fn codes_blit_at_clear_and_scroll_write_the_plane_and_only_changed_rows_leave() {
    let mut host = host();
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[2, 38], &[0x11, 0x7F]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::TT_AT, &[2, 39]), Reply::Int(0x7F));
    assert_eq!(rows(&batch(&mut host)), vec![TeletextRow { row: 2, codes: row_with(38, &[0x11, 0x7F]) }]);
    // Writing a row as it already is sends nothing.
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[2, 38], &[0x11, 0x7F]), Reply::Undefined);
    unchanged(&mut host);

    // A tile map in one call: row r holds the letter of r.
    let plane: Vec<u8> = (0..BLIT_CODES).map(|at| b'A' + u8::try_from(at / COLUMNS).unwrap()).collect();
    assert_eq!(bytes(&mut host, Op::TT_BLIT, &[], &plane), Reply::Undefined);
    assert_eq!(rows(&batch(&mut host)).len(), 23);
    assert_eq!(call(&mut host, Op::TT_AT, &[23, 0]), Reply::Int(i32::from(b'W')));

    // Up by two inside rows 3 to 6: 3 and 4 take 5 and 6, which are spaces.
    assert_eq!(call(&mut host, Op::TT_SCROLL, &[3, 6, -2]), Reply::Undefined);
    assert_eq!(first_codes(&batch(&mut host)), vec![(3, b'E'), (4, b'F'), (5, SPACE), (6, SPACE)]);
    // Down by one inside rows 3 to 4, and by more than the band holds.
    assert_eq!(call(&mut host, Op::TT_SCROLL, &[3, 4, 1]), Reply::Undefined);
    assert_eq!(first_codes(&batch(&mut host)), vec![(3, SPACE), (4, b'E')]);
    assert_eq!(call(&mut host, Op::TT_SCROLL, &[1, 2, i32::MIN]), Reply::Undefined);
    assert_eq!(first_codes(&batch(&mut host)), vec![(1, SPACE), (2, SPACE)]);

    assert_eq!(call(&mut host, Op::TT_CLEAR, &[20, 23]), Reply::Undefined);
    assert_eq!(rows(&batch(&mut host)).iter().map(|row| row.row).collect::<Vec<_>>(), vec![20, 21, 22, 23]);
    assert_eq!(call(&mut host, Op::TT_AT, &[23, 0]), Reply::Int(i32::from(SPACE)));
    assert_eq!(call(&mut host, Op::TT_AT, &[19, 0]), Reply::Int(i32::from(b'S')));
}

/// Section 5.2 and EN 300 706 clause 12.3: the overlay is the page's
/// local enhancement data, whole, in the standard's order: each row's
/// address triplet, its column triplets by column, the Termination
/// Marker. An overlay cell carries its own look and does not touch a
/// neighbour: the plane's own colours are written again at the first
/// cell after it.
#[test]
fn the_overlay_is_written_as_local_enhancement_data_and_touches_no_neighbour() {
    let mut host = host();
    assert_eq!(text(&mut host, Op::TT_OVERLAY_PUT, &[5, 10, 3, 4, ON << FLASH_SHIFT, NONE], "AB"), Reply::Undefined);
    assert_eq!(
        triplets(&batch(&mut host)),
        vec![
            at_row(5),
            (10, MODE_FOREGROUND_COLOUR, 3),
            (10, MODE_BACKGROUND_COLOUR, 4),
            (10, MODE_ADDITIONAL_FLASH, FLASH_MODE_NORMAL),
            (10, MODE_G0_LEVEL_25, b'A'),
            (11, MODE_G0_LEVEL_25, b'B'),
            // The plane's own look again: white on black, steady.
            (12, MODE_FOREGROUND_COLOUR, 7),
            (12, MODE_BACKGROUND_COLOUR, 0),
            (12, MODE_ADDITIONAL_FLASH, FLASH_MODE_STEADY),
            END,
        ]
    );
    assert_eq!(budget(&mut host, "triplets"), 10);
    // The plane was not written.
    assert_eq!(call(&mut host, Op::TT_AT, &[5, 10]), Reply::Int(i32::from(SPACE)));
    // A Flash code on the plane makes the row flash from the next cell;
    // an overlay cell there that names no flash takes the row's, and
    // costs only its own colour and the plane's again after it.
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[5, 20], &[FLASH]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::TT_OVERLAY_FILL, &[5, 21, 1, 1, NONE, 6]), Reply::Undefined);
    let flashing = triplets(&batch(&mut host));
    assert_eq!(flashing[9..], [(21, MODE_BACKGROUND_COLOUR, 6), (22, MODE_BACKGROUND_COLOUR, 0), END]);

    // A band across a whole row has no cell after it to restore.
    assert_eq!(call(&mut host, Op::TT_OVERLAY_CLEAR, &[0, 0, 0, 0]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::TT_CLEAR, &[1, 23]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::TT_OVERLAY_FILL, &[22, 0, 1, 40, 0, 16]), Reply::Undefined);
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(22), (0, MODE_FOREGROUND_COLOUR, 0), (0, MODE_BACKGROUND_COLOUR, 16), END]);
    // The worked example's clock is put on that row with a white ink: the
    // spacing attribute changes the row's foreground from column 31, and
    // the band's own ink is written again there, so the band stays whole.
    assert_eq!(put(&mut host, 22, 31, "12:00:00", [7, NONE, 0, 0]), Reply::Int(9));
    assert_eq!(
        triplets(&batch(&mut host)),
        vec![at_row(22), (0, MODE_FOREGROUND_COLOUR, 0), (0, MODE_BACKGROUND_COLOUR, 16), (31, MODE_FOREGROUND_COLOUR, 0), END]
    );
    // Clearing a rectangle removes those cells alone: from column 10 the
    // plane's own colours, which from column 31 are the clock's white.
    assert_eq!(call(&mut host, Op::TT_OVERLAY_CLEAR, &[22, 10, 1, 30]), Reply::Undefined);
    assert_eq!(
        triplets(&batch(&mut host)),
        vec![
            at_row(22),
            (0, MODE_FOREGROUND_COLOUR, 0),
            (0, MODE_BACKGROUND_COLOUR, 16),
            (10, MODE_FOREGROUND_COLOUR, 7),
            (10, MODE_BACKGROUND_COLOUR, 0),
            END
        ]
    );
    // `fill` keeps a cell's character; `put` replaces the whole cell.
    assert_eq!(call(&mut host, Op::TT_OVERLAY_CLEAR, &[0, 0, 0, 0]), Reply::Undefined);
    assert_eq!(text(&mut host, Op::TT_OVERLAY_PUT, &[2, 0, NONE, NONE, 0, NONE], "Z"), Reply::Undefined);
    assert_eq!(call(&mut host, Op::TT_OVERLAY_FILL, &[2, 0, 1, 1, 1, NONE]), Reply::Undefined);
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(2), (0, MODE_FOREGROUND_COLOUR, 1), (0, MODE_G0_LEVEL_25, b'Z'), (1, MODE_FOREGROUND_COLOUR, 7), END]);
    assert_eq!(text(&mut host, Op::TT_OVERLAY_PUT, &[2, 0, NONE, 2, 0, NONE], "Q"), Reply::Undefined);
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(2), (0, MODE_BACKGROUND_COLOUR, 2), (0, MODE_G0_LEVEL_25, b'Q'), (1, MODE_BACKGROUND_COLOUR, 0), END]);
    // No overlay, no enhancement data at all.
    assert_eq!(call(&mut host, Op::TT_OVERLAY_CLEAR, &[0, 0, 0, 0]), Reply::Undefined);
    assert_eq!(batch(&mut host).teletext_enhancement, Some(Vec::new()));
    assert_eq!(budget(&mut host, "triplets"), 0);

    let reply = call(&mut host, Op::TT_OVERLAY_FILL, &[1, 0, 1, 1, 32, NONE]);
    refused(&mut host, &reply, "COLOUR_MAP_ENTRIES");
    let reply = call(&mut host, Op::TT_OVERLAY_FILL, &[20, 0, 5, 1, 1, NONE]);
    refused(&mut host, &reply, "TELETEXT_ROWS");
    let reply = call(&mut host, Op::TT_OVERLAY_FILL, &[1, 30, 1, 11, 1, NONE]);
    refused(&mut host, &reply, "COLUMNS");
    let reply = text(&mut host, Op::TT_OVERLAY_PUT, &[1, 39, 1, NONE, 0, NONE], "AB");
    refused(&mut host, &reply, "COLUMNS");
}

/// Section 5.2: every overlay cell, mark and control is paid for from the
/// page's 208 local triplets; a call that would pass them is `RangeError
/// LOCAL_TRIPLETS` and changes nothing, whichever call it is: here an
/// overlay fill, a control, overlay text, and a plain write of a code
/// whose attribute makes the overlay restate its colour.
#[test]
fn a_call_that_would_pass_the_pages_local_triplets_changes_nothing() {
    let mut host = host();
    // Each cell of a checkerboard row costs two triplets (its colour and
    // the plane's again after it) and the row one: 41 a row. A band of
    // one ink across a whole row costs the row's triplet and one more.
    for row in 1..=5 {
        for column in (0..40).step_by(2) {
            assert_eq!(call(&mut host, Op::TT_OVERLAY_FILL, &[row, column, 1, 1, 1, NONE]), Reply::Undefined);
        }
    }
    assert_eq!(call(&mut host, Op::TT_OVERLAY_FILL, &[6, 0, 1, 40, 1, NONE]), Reply::Undefined);
    // With the Termination Marker, exactly the page's triplets.
    assert_eq!(5 * 41 + 2 + 1, LOCAL_TRIPLETS);
    assert_eq!(budget(&mut host, "triplets"), LOCAL_TRIPLETS);
    assert_eq!(triplets(&batch(&mut host)).len(), LOCAL_TRIPLETS);
    let reply = call(&mut host, Op::TT_OVERLAY_FILL, &[7, 0, 1, 1, 1, NONE]);
    refused(&mut host, &reply, "LOCAL_TRIPLETS");
    let reply = call(&mut host, Op::TT_CONTROL, &[9, 0, 5, 0]);
    refused(&mut host, &reply, "LOCAL_TRIPLETS");
    let reply = text(&mut host, Op::TT_OVERLAY_PUT, &[8, 0, NONE, NONE, 0, NONE], "A");
    refused(&mut host, &reply, "LOCAL_TRIPLETS");
    // A green ink code in the band's row changes the row's foreground
    // from the next cell, where the band must then state its own ink
    // again: one triplet more than the page has, so the plane itself
    // refuses the code.
    let reply = bytes(&mut host, Op::TT_CODES, &[6, 10], &[0x02]);
    refused(&mut host, &reply, "LOCAL_TRIPLETS");
    assert_eq!(call(&mut host, Op::TT_AT, &[6, 10]), Reply::Int(i32::from(SPACE)));
    assert_eq!(budget(&mut host, "triplets"), LOCAL_TRIPLETS);
    // Removing pays back: with the band gone the control fits.
    assert_eq!(call(&mut host, Op::TT_OVERLAY_CLEAR, &[6, 0, 1, 40]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::TT_CONTROL, &[9, 0, 5, 0]), Reply::Int(control_handle(9, 0).unwrap()));
    assert_eq!(budget(&mut host, "triplets"), LOCAL_TRIPLETS);
}

/// Section 5.4 and the engine's decision E.127: a control is one control
/// triplet for each of its rows; at most 32; its handle names its first
/// cell; a forced look is an `ANIMATE` look operation on the number the
/// engine gives the control, which is its place in the enhancement data,
/// so a control declared before it renumbers it and the looks are told
/// again by number.
#[test]
fn controls_are_control_triplets_and_looks_follow_the_engines_numbering() {
    let mut host = host();
    let forecast = control_handle(4, 1).unwrap();
    assert_eq!(call(&mut host, Op::TT_CONTROL, &[4, 1, 19, 0]), Reply::Int(forecast));
    assert_eq!(text(&mut host, Op::TT_LABEL, &[forecast], "Shipping forecast"), Reply::Undefined);
    let first = batch(&mut host);
    assert_eq!(triplets(&first), vec![at_row(4), (1, MODE_CONTROL, 19), END]);
    assert_eq!(first.labels, Some(vec![Label { handle: forecast, label: "Shipping forecast".into() }]));
    assert_eq!(first.animate, None);
    // A joined run is one more row of the same control, the same handle.
    assert_eq!(call(&mut host, Op::TT_CONTROL, &[5, 1, 19, 1]), Reply::Int(forecast));
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(4), (1, MODE_CONTROL, 19), at_row(5), (1, MODE_CONTROL, CONTROL_JOINED + 19), END]);
    assert_eq!(budget(&mut host, "controls"), 1);

    // A forced look: control 1 until another is declared above it.
    assert_eq!(call(&mut host, Op::TT_ANIMATE, &[forecast, 7, 2, 0, 0, 0]), Reply::Undefined);
    let look = |number, look| Operation { target: Target::control(number), kind: OperationKind::Look(look) };
    assert_eq!(batch(&mut host).animate, Some(vec![look(1, 2)]));
    let tides = control_handle(2, 0).unwrap();
    assert_eq!(call(&mut host, Op::TT_CONTROL, &[2, 0, 40, 0]), Reply::Int(tides));
    assert_eq!(batch(&mut host).animate, Some(vec![look(1, 0), look(2, 2)]));
    assert_eq!(call(&mut host, Op::TT_REMOVE, &[tides]), Reply::Undefined);
    assert_eq!(batch(&mut host).animate, Some(vec![look(1, 2), look(2, 0)]));

    // What a control may not be.
    let reply = call(&mut host, Op::TT_CONTROL, &[5, 10, 5, 0]);
    type_refused(&mut host, &reply);
    let reply = call(&mut host, Op::TT_CONTROL, &[9, 1, 19, 1]);
    type_refused(&mut host, &reply);
    let reply = call(&mut host, Op::TT_CONTROL, &[9, 30, 11, 0]);
    refused(&mut host, &reply, "COLUMNS");
    let reply = call(&mut host, Op::TT_CONTROL, &[9, 30, 0, 0]);
    refused(&mut host, &reply, "COLUMNS");
    let reply = call(&mut host, Op::TT_ANIMATE, &[forecast, 7, 3, 0, 0, 0]);
    refused(&mut host, &reply, "MAX_LOOK");
    let reply = call(&mut host, Op::TT_ANIMATE, &[forecast, 15, 1, 0, 0, 0]);
    type_refused(&mut host, &reply);
    let reply = text(&mut host, Op::TT_LABEL, &[forecast], &"l".repeat(65));
    refused(&mut host, &reply, "MAX_LABEL_BYTES");
    let reply = text(&mut host, Op::TT_LABEL, &[forecast], "two\nlines");
    type_refused(&mut host, &reply);
    // A stale, forged or foreign handle is a TypeError (section 10.1 rule 4).
    for handle in [tides, 1, i32::MAX, i32::MIN, forecast + 1] {
        let reply = call(&mut host, Op::TT_REMOVE, &[handle]);
        type_refused(&mut host, &reply);
        let reply = call(&mut host, Op::LISTEN, &[handle, EventKind::POINTER.number(), 1]);
        assert!(matches!(reply, Reply::Type(_)), "{handle}: {reply:?}");
    }
    // The host is told a control has a listener; the listener goes with
    // the control.
    assert_eq!(call(&mut host, Op::LISTEN, &[forecast, EventKind::POINTER.number(), 1]), Reply::Undefined);
    assert!(host.listens_to(ContextId::Page, forecast, EventKind::POINTER));
    assert_eq!(call(&mut host, Op::TT_REMOVE, &[forecast]), Reply::Undefined);
    assert!(!host.listens_to(ContextId::Page, forecast, EventKind::POINTER));
    assert_eq!(batch(&mut host).teletext_enhancement, Some(Vec::new()));

    // At most MAX_CONTROLS: one a cell along row 10, and one more refused.
    for column in 0..i32::try_from(MAX_CONTROLS).unwrap() {
        assert!(matches!(call(&mut host, Op::TT_CONTROL, &[10, column, 1, 0]), Reply::Int(_)));
    }
    let _ = batch(&mut host);
    let reply = call(&mut host, Op::TT_CONTROL, &[11, 0, 1, 0]);
    refused(&mut host, &reply, "MAX_CONTROLS");
}

/// Section 3.3: a control made in a scope is removed when the scope ends.
#[test]
fn a_scopes_controls_go_with_the_scope() {
    let mut host = host();
    let Reply::Int(scope) = call(&mut host, Op::SCOPE_BEGIN, &[]) else { panic!("no scope") };
    let inside = control_handle(3, 0).unwrap();
    assert_eq!(call(&mut host, Op::TT_CONTROL, &[3, 0, 4, 0]), Reply::Int(inside));
    assert_eq!(call(&mut host, Op::LISTEN, &[inside, EventKind::POINTER.number(), 1]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::SCOPE_LEAVE, &[scope]), Reply::Undefined);
    let outside = control_handle(6, 0).unwrap();
    assert_eq!(call(&mut host, Op::TT_CONTROL, &[6, 0, 4, 0]), Reply::Int(outside));
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(3), (0, MODE_CONTROL, 4), at_row(6), (0, MODE_CONTROL, 4), END]);
    assert_eq!(call(&mut host, Op::SCOPE_END, &[scope]), Reply::Undefined);
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(6), (0, MODE_CONTROL, 4), END]);
    assert!(!host.listens_to(ContextId::Page, inside, EventKind::POINTER));
    assert!(matches!(call(&mut host, Op::TT_REMOVE, &[inside]), Reply::Type(_)));
    assert_eq!(budget(&mut host, "triplets"), 3);
}

/// Section 5.5 and EN 300 706 clause 9.4.2.2: `define` gives the entries
/// 16 to 31 in turn, sixteen and no more; `set` redefines one; both leave
/// as the settings section.
#[test]
fn the_colour_map_defines_entries_16_to_31() {
    let mut host = host();
    assert_eq!(text(&mut host, Op::TT_COLOUR_DEFINE, &[0xFA0], "amber"), Reply::Int(16));
    assert_eq!(text(&mut host, Op::TT_COLOUR_DEFINE, &[0x024], "sea"), Reply::Int(17));
    let settings = |colours: &[(u8, u16)]| Some(TeletextSettings { colours: colours.to_vec(), ..TeletextSettings::default() });
    assert_eq!(batch(&mut host).teletext_settings, settings(&[(16, 0xFA0), (17, 0x024)]));
    assert_eq!(call(&mut host, Op::TT_COLOUR_SET, &[17, 0x400]), Reply::Undefined);
    assert_eq!(batch(&mut host).teletext_settings, settings(&[(16, 0xFA0), (17, 0x400)]));
    assert_eq!(budget(&mut host, "colours"), 2);

    let reply = call(&mut host, Op::TT_COLOUR_SET, &[15, 0x400]);
    refused(&mut host, &reply, "TELETEXT_DEFINABLE_COLOURS");
    let reply = call(&mut host, Op::TT_COLOUR_SET, &[32, 0x400]);
    refused(&mut host, &reply, "TELETEXT_DEFINABLE_COLOURS");
    let reply = call(&mut host, Op::TT_COLOUR_SET, &[17, 0x1000]);
    refused(&mut host, &reply, "MAX_RGB12");
    let reply = text(&mut host, Op::TT_COLOUR_DEFINE, &[-1], "none");
    refused(&mut host, &reply, "MAX_RGB12");
    let reply = text(&mut host, Op::TT_COLOUR_DEFINE, &[0], &"n".repeat(65));
    refused(&mut host, &reply, "MAX_LABEL_BYTES");
    // An entry `define` never gave is not the script's to set.
    let reply = call(&mut host, Op::TT_COLOUR_SET, &[18, 0x400]);
    type_refused(&mut host, &reply);
    for entry in 18..16 + i32::try_from(TELETEXT_DEFINABLE_COLOURS).unwrap() {
        assert_eq!(text(&mut host, Op::TT_COLOUR_DEFINE, &[0x111], "more"), Reply::Int(entry));
    }
    let _ = batch(&mut host);
    let reply = text(&mut host, Op::TT_COLOUR_DEFINE, &[0x111], "seventeenth");
    refused(&mut host, &reply, "TELETEXT_DEFINABLE_COLOURS");
}

/// Section 10.5 and the security review's finding S-10 (TODO-7276): one
/// entry changes at most `MAX_COLOUR_SETS_PER_SECOND` times in any
/// second, whatever a script asks; a change past the rate does not throw,
/// the latest value waits, the decoder is told when to wake for it, and
/// it is applied at the first step the rate allows.
#[test]
fn a_colour_entry_changes_at_most_six_times_a_second() {
    let mut host = host();
    assert_eq!(text(&mut host, Op::TT_COLOUR_DEFINE, &[0x000], "flasher"), Reply::Int(16));
    let _ = batch(&mut host);
    // Every tick for three seconds, alternating black and white: the
    // review's flashing page.
    let mut changes: Vec<u64> = vec![0];
    for tick in 1..=3 * TICKS_PER_SECOND {
        host.begin_step(tick, true);
        let asked = if tick % 2 == 0 { 0x000 } else { 0xFFF };
        assert_eq!(call(&mut host, Op::TT_COLOUR_SET, &[16, asked]), Reply::Undefined, "nothing throws");
        if batch(&mut host).teletext_settings.is_some() {
            changes.push(tick);
        }
    }
    for (at, tick) in changes.iter().enumerate() {
        let within = changes[at..].iter().filter(|later| **later < tick + TICKS_PER_SECOND).count();
        assert!(within <= MAX_COLOUR_SETS_PER_SECOND, "{within} changes in the second from tick {tick}: {changes:?}");
    }
    assert!(changes.len() >= 3 * MAX_COLOUR_SETS_PER_SECOND, "the rate allows six a second: {changes:?}");

    // A value that waits is applied with no further call, and the decoder
    // is told the tick to wake at.
    let mut host = self::host();
    assert_eq!(text(&mut host, Op::TT_COLOUR_DEFINE, &[0x000], "flasher"), Reply::Int(16));
    for colour in 1..=6 {
        assert_eq!(call(&mut host, Op::TT_COLOUR_SET, &[16, colour]), Reply::Undefined);
    }
    let first = batch(&mut host);
    assert_eq!(first.teletext_settings.unwrap().colours, vec![(16, 5)], "the define and five sets; the sixth waits");
    let page = first.state.unwrap().into_iter().find(|state| state.context == ContextId::Page).unwrap();
    assert_eq!(page.next_due, Some(TICKS_PER_SECOND));
    host.begin_step(TICKS_PER_SECOND - 1, true);
    assert_eq!(batch(&mut host).teletext_settings, None);
    host.begin_step(TICKS_PER_SECOND, true);
    let later = batch(&mut host);
    assert_eq!(later.teletext_settings.unwrap().colours, vec![(16, 6)]);
    assert_eq!(later.state.unwrap().into_iter().find(|state| state.context == ContextId::Page).unwrap().next_due, None);
}

/// Section 5.1 and EN 300 706 clause 9.4.2: the page's settings, each
/// left as it is when not given; under black background substitution the
/// plane's black background is the row colour, which is what the overlay
/// restores after a cell.
#[test]
fn settings_are_the_default_colours_and_substitution() {
    let mut host = host();
    assert_eq!(call(&mut host, Op::TT_SETTINGS, &[4, NONE, NONE]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::TT_SETTINGS, &[NONE, 17, 1]), Reply::Undefined);
    let expected = TeletextSettings { screen_colour: 4, row_colour: 17, substitution: true, colours: Vec::new() };
    assert_eq!(batch(&mut host).teletext_settings, Some(expected));
    assert_eq!(call(&mut host, Op::TT_OVERLAY_FILL, &[3, 5, 1, 1, NONE, 2]), Reply::Undefined);
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(3), (5, MODE_BACKGROUND_COLOUR, 2), (6, MODE_BACKGROUND_COLOUR, 17), END]);
    assert_eq!(call(&mut host, Op::TT_SETTINGS, &[NONE, NONE, 0]), Reply::Undefined);
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(3), (5, MODE_BACKGROUND_COLOUR, 2), (6, MODE_BACKGROUND_COLOUR, 0), END]);
    let reply = call(&mut host, Op::TT_SETTINGS, &[NONE, NONE, 2]);
    type_refused(&mut host, &reply);
}

/// Section 5.6: text goes through the engine's encoder. On an English
/// page a letter with a diacritical mark is its base letter in the row
/// and an X/26 triplet over it, paid for from the local triplets; a run
/// in another language of a standard set (German) and one of
/// NeoTeletext's own sets (Georgian) are the row's own codes read in the
/// run's set by the row set triplet, with the page's set named again
/// where the row shows something after the run; and a character no set
/// of the page shows is a `TypeError` naming it, never a substitute.
#[test]
fn text_is_encoded_by_the_engines_encoder_for_the_pages_language_or_a_runs_own() {
    let mut host = host();
    assert_eq!(put(&mut host, 2, 0, "caf\u{e9} 5", PLAIN), Reply::Int(6));
    let first = batch(&mut host);
    assert_eq!(rows(&first)[0].codes[..6], *b"cafe 5");
    // Mark 2 is the acute accent (Table 29, modes 10001 to 11111).
    assert_eq!(triplets(&first), vec![at_row(2), (3, MODE_G0_NO_DIACRITIC | 2, b'e'), END]);
    // Overwriting the cell takes the mark with it.
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[2, 3], b"E"), Reply::Undefined);
    assert_eq!(batch(&mut host).teletext_enhancement, Some(Vec::new()));

    let german = language(&mut host, "de");
    let (english_value, german_value) = (language_value("en", true).unwrap(), language_value("de", true).unwrap());
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[4, 20], b"after"), Reply::Undefined);
    assert_eq!(text(&mut host, Op::TT_PUT, &[4, 0, NONE, NONE, 0, 0, german], "Gr\u{fc}\u{df}e"), Reply::Int(5));
    let second = batch(&mut host);
    // German's national option holds ü at 7/D and ß at 7/E (Table 36).
    assert_eq!(rows(&second)[0].codes[..5], [b'G', b'r', 0x7D, 0x7E, b'e']);
    assert_eq!(triplets(&second), vec![at_row(4), (0, MODE_ROW_SET, german_value), (20, MODE_ROW_SET, english_value), END]);

    let georgian = language(&mut host, "ka");
    let georgian_value = language_value("ka", true).unwrap();
    assert_eq!(text(&mut host, Op::TT_PUT, &[6, 3, NONE, NONE, 0, 0, georgian], "\u{10d0}\u{10d1}"), Reply::Int(2));
    let third = batch(&mut host);
    assert!(rows(&third)[0].codes[3..5].iter().all(|code| *code > SPACE), "{:?}", rows(&third));
    assert_eq!(triplets(&third)[3..], [at_row(6), (3, MODE_ROW_SET, georgian_value), END]);

    // Never a substitute: the snowman is in no teletext set.
    let reply = put(&mut host, 8, 0, "a\u{2603}b", PLAIN);
    let Reply::Type(message) = &reply else { panic!("{reply:?}") };
    assert!(message.contains("'\u{2603}'") && message.contains("U+2603"), "{message}");
    unchanged(&mut host);
    assert_eq!(call(&mut host, Op::TT_AT, &[8, 0]), Reply::Int(i32::from(SPACE)));
    let reply = text(&mut host, Op::TT_OVERLAY_PUT, &[8, 0, NONE, NONE, 0, NONE], "\u{2603}");
    type_refused(&mut host, &reply);
    let reply = text(&mut host, Op::TT_PUT, &[8, 0, NONE, NONE, 0, 0, 10_000], "a");
    type_refused(&mut host, &reply);
    // An ESC before the text would leave it in the second set: refused,
    // naming the cell.
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[9, 2], &[0x1B]), Reply::Undefined);
    let _ = batch(&mut host);
    let reply = put(&mut host, 9, 10, "a", PLAIN);
    let Reply::Type(message) = &reply else { panic!("{reply:?}") };
    assert!(message.contains("row 9, column 2"), "{message}");
    unchanged(&mut host);
}

/// Section 5.6: overlay text is characters by X/26 triplets alone, which
/// take no national option, so a set other than the page's is named by a
/// Modified G0 and G2 Character Set Designation where the overlay's
/// characters begin; and a German page's own text keeps its national
/// option in the row.
#[test]
fn overlay_text_is_triplets_in_the_runs_own_sets() {
    let mut host = host_in("de");
    assert_eq!(put(&mut host, 2, 0, "\u{fc}", PLAIN), Reply::Int(1));
    let first = batch(&mut host);
    assert_eq!((rows(&first)[0].codes[0], first.teletext_enhancement), (0x7D, None));
    // The same letter on the overlay: the base letter with the umlaut,
    // mark 8, since a triplet's G0 set has no national option.
    assert_eq!(text(&mut host, Op::TT_OVERLAY_PUT, &[3, 0, NONE, NONE, 0, NONE], "\u{fc}"), Reply::Undefined);
    assert_eq!(triplets(&batch(&mut host)), vec![at_row(3), (0, MODE_G0_NO_DIACRITIC | 8, b'u'), END]);
    // Greek on a German page: the designation, then the letters.
    let greek = language(&mut host, "el");
    let greek_value = language_value("el", false).unwrap();
    assert_eq!(text(&mut host, Op::TT_OVERLAY_PUT, &[3, 0, NONE, NONE, 0, greek], "\u{3b1}\u{3b2}"), Reply::Undefined);
    let greek_triplets = triplets(&batch(&mut host));
    assert_eq!(greek_triplets[..2], [at_row(3), (0, MODE_MODIFIED_DESIGNATION, greek_value)]);
    assert_eq!(greek_triplets.iter().filter(|triplet| triplet.1 == MODE_G0_LEVEL_25).count(), 2);
    assert_eq!(greek_triplets.len(), 5);
}

/// Section 5.6: `measure` gives the cells a text takes, also when they
/// are more than a row; `languages` lists what the engine has; `level` is
/// the receiver's presentation level. The channel context may ask these
/// and may draw nothing (section 3.1), and a page that is not teletext
/// has no teletext API at the host either.
#[test]
fn measure_languages_and_level_are_every_contexts_and_drawing_is_the_pages() {
    let mut host = host();
    assert_eq!(text(&mut host, Op::TT_MEASURE, &[NONE], "caf\u{e9}"), Reply::Int(4));
    assert_eq!(text(&mut host, Op::TT_MEASURE, &[NONE], &"w".repeat(50)), Reply::Int(50));
    let reply = text(&mut host, Op::TT_MEASURE, &[NONE], "\u{2603}");
    type_refused(&mut host, &reply);
    assert_eq!(call(&mut host, Op::TT_LEVEL, &[]), Reply::Int(25));
    let Reply::Text(json) = call(&mut host, Op::TT_LANGUAGES, &[]) else { panic!("no languages") };
    assert!(json.starts_with("[{\"tag\":\"en\",\"name\":\"English\",\"direction\":\"ltr\"}"), "{json}");
    assert!(json.contains("{\"tag\":\"he\",\"name\":\"Hebrew\",\"direction\":\"rtl\"}"), "{json}");
    assert_eq!(json.matches("\"tag\"").count(), neoteletext_formats::write_text::languages().len());

    assert_eq!(host.call(CHANNEL, Op::TT_LEVEL.number(), &[], Payload::None), Reply::Int(25));
    assert_eq!(host.call(CHANNEL, Op::TT_MEASURE.number(), &[NONE], Payload::Text(b"abc")), Reply::Int(3));
    for (op, ints) in
        [(Op::TT_CLEAR, vec![1, 23]), (Op::TT_OVERLAY_CLEAR, vec![0, 0, 0, 0]), (Op::TT_CONTROL, vec![1, 0, 1, 0]), (Op::TT_COLOUR_SET, vec![16, 0])]
    {
        let reply = host.call(CHANNEL, op.number(), &ints, Payload::None);
        type_refused(&mut host, &reply);
    }

    let mut regis = Host::new();
    regis.open(opening(ContextId::Page, Renderer::Regis, ENGLISH), 1);
    regis.begin_step(0, true);
    assert!(matches!(call(&mut regis, Op::TT_LEVEL, &[]), Reply::Type(_)));
    assert!(matches!(call(&mut regis, Op::TT_CLEAR, &[1, 23]), Reply::Type(_)));
    // The page's state goes with its context, and a new page starts blank.
    assert_eq!(bytes(&mut host, Op::TT_CODES, &[1, 0], b"A"), Reply::Undefined);
    host.close(ContextId::Page);
    host.open(opening(ContextId::Page, Renderer::Teletext, ENGLISH), 1);
    assert_eq!(call(&mut host, Op::TT_AT, &[1, 0]), Reply::Int(i32::from(SPACE)));
    unchanged(&mut host);
}
