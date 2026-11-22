//! Teletext sprites at the host call (script-api.md sections 5.3, 3.1
//! and 10.1): the triplets a frame compiles to, where its object lies on
//! the Object page, what each operation puts in the step's batch, and for
//! each bound that the refusal names it and nothing changed.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "tests")]

use neoteletext_core::consts::{
    ATTRIBUTE_UNDERLINE, MODE_BACKGROUND_COLOUR, MODE_DISPLAY_ATTRIBUTES, MODE_DRCS_CHARACTER, MODE_FOREGROUND_COLOUR, MODE_G0_LEVEL_25, MODE_G1,
    MODE_MODIFIED_DESIGNATION, MODE_SET_ACTIVE_POSITION, ROW_ADDRESS_BASE,
};
use neoteletext_rules::animate::{Operation, OperationKind, code};
use neoteletext_rules::table::Target;
use protocol::script::batch::Triplet;
use protocol::script::sprites::{PAGE_SHARE_OBJECT_SUBPAGES, object_number, object_place};
use protocol::script::{Batch, ContextFlags, ContextId, ContextOpen, Level, Renderer};

use super::limits::{CELL_BYTES, KEEP_LAYER, MAX_OBJECT_BODY, NO_SETS, OBJECTS_PER_SUBPAGE, SUBPAGE_TRIPLETS};
use super::object::{CELL_CHARACTER, CELL_CONTIGUOUS, CELL_NONE, CELL_SEPARATED, ObjectPage, compile};
use super::{FLAG_SHOWN, MOVE_BY, MOVE_TO};
use crate::host::{Host, Payload, Reply};
use crate::ops::{EventKind, Op};
use crate::random::SEED_BYTES;

const PAGE: i32 = 2;

fn opening(context: ContextId) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer: Renderer::Teletext,
        level: Level::TwoFive,
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

/// A host with the page context open on a teletext page, in a callback
/// at tick 0.
fn host() -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Page), 1);
    host.begin_step(0, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
    host
}

/// The step's batch, and the next step begun at `tick`.
fn step(host: &mut Host, tick: u64) -> Batch {
    host.end_callback();
    let batch = host.take_batch(1, &[]);
    host.begin_step(tick, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
    batch
}

/// A cell's six bytes.
fn cell(kind: u8, code: u8, ink: u8, paper: u8) -> [u8; CELL_BYTES] {
    let mode = if kind == CELL_CHARACTER { MODE_G0_LEVEL_25 } else { 0 };
    [kind, code, ink, paper, mode, if kind == CELL_CHARACTER { NO_SETS } else { 0 }]
}

/// A frame of one cell: the letter `letter`, white on black.
fn letter(letter: u8) -> Vec<u8> {
    cell(CELL_CHARACTER, letter, 7, 0).to_vec()
}

/// `sprite(...)` of 1 by 1 frames at row 5, column 5, layer 2.
fn sprite(host: &mut Host, frames: &[Vec<u8>]) -> Reply {
    let cells = frames.concat();
    host.call(PAGE, Op::TT_SPRITE.number(), &[1, 1, i32::try_from(frames.len()).unwrap(), 2, 0, 5, 5, FLAG_SHOWN], Payload::Bytes(&cells))
}

fn animate(host: &mut Host, handle: i32, operation: u8, numbers: [i32; 4]) -> Reply {
    let [a, b, c, d] = numbers;
    host.call(PAGE, Op::TT_ANIMATE.number(), &[handle, i32::from(operation), a, b, c, d], Payload::None)
}

fn object(number: u16, kind: OperationKind) -> Operation {
    Operation { target: Target::object(number), kind }
}

fn column(column: u8, mode: u8, data: u8) -> Triplet {
    Triplet { address: column, mode, data }
}

/// Section 5.3 and EN 300 706 clause 13.5: a frame is a Passive object's
/// body in screen order. Each row's first cell is written with its
/// colours; a row below the origin's is addressed by a Set Active
/// Position triplet; a colour is written again only where it changes; a
/// separated mosaic is Display Attributes bit D5; a character of a named
/// set is written after its Modified G0 and G2 Character Set Designation;
/// and a cell never set has no triplet, so it is transparent.
#[test]
fn a_frame_compiles_to_a_passive_objects_body() {
    let mut cells = Vec::new();
    // Row 0: nothing, a full contiguous mosaic in 3 on 4, a separated one
    // in the same colours. Row 1: a letter of the Latin set (designation
    // 0) in 7 on 0, nothing, a space in 7 on 6.
    cells.extend(cell(CELL_NONE, 0, 0, 0));
    cells.extend(cell(CELL_CONTIGUOUS, 0x3F, 3, 4));
    cells.extend(cell(CELL_SEPARATED, 0x21, 3, 4));
    cells.extend([CELL_CHARACTER, b'A', 7, 0, MODE_G0_LEVEL_25, 0]);
    cells.extend(cell(CELL_NONE, 0, 0, 0));
    cells.extend(cell(CELL_CHARACTER, b' ', 7, 6));
    let body = compile(3, 2, &cells).unwrap();
    assert_eq!(
        body,
        vec![
            column(1, MODE_FOREGROUND_COLOUR, 3),
            column(1, MODE_BACKGROUND_COLOUR, 4),
            column(1, MODE_DISPLAY_ATTRIBUTES, 0),
            column(1, MODE_G1, 0x7F),
            column(2, MODE_DISPLAY_ATTRIBUTES, ATTRIBUTE_UNDERLINE),
            // Sixels 0 and 5: the code's bits 0 and 6, with bit 5.
            column(2, MODE_G1, 0x61),
            Triplet { address: ROW_ADDRESS_BASE + 1, mode: MODE_SET_ACTIVE_POSITION, data: 0 },
            column(0, MODE_FOREGROUND_COLOUR, 7),
            column(0, MODE_BACKGROUND_COLOUR, 0),
            column(0, MODE_DISPLAY_ATTRIBUTES, 0),
            column(0, MODE_MODIFIED_DESIGNATION, 0),
            column(0, MODE_G0_LEVEL_25, b'A'),
            column(2, MODE_BACKGROUND_COLOUR, 6),
            column(2, MODE_G0_LEVEL_25, b' '),
        ]
    );
}

/// Section 10.1: a script's bytes are outside input. A cell that is no
/// kind, a colour past the Colour Map, a mosaic past six sixels, and a
/// "character" whose mode is not one that writes a character (a DRCS
/// invocation, Display Attributes, a mode past five bits) or whose code
/// or designation is none are refused, so no frame holds a triplet the
/// encoder would not write for text.
#[test]
fn a_hostile_cell_is_refused() {
    let refused = |bytes: [u8; CELL_BYTES]| compile(1, 1, &bytes).unwrap_err();
    assert!(matches!(refused([9, 0, 0, 0, 0, 0]), Reply::Type(_)));
    assert!(matches!(refused(cell(CELL_CONTIGUOUS, 1, 32, 0)), Reply::Range(text) if text.starts_with("COLOUR_MAP_ENTRIES: ")));
    assert!(matches!(refused(cell(CELL_CONTIGUOUS, 1, 0, 255)), Reply::Range(text) if text.starts_with("COLOUR_MAP_ENTRIES: ")));
    assert!(matches!(refused(cell(CELL_SEPARATED, 0x40, 0, 0)), Reply::Range(text) if text.starts_with("MOSAIC_PATTERNS: ")));
    for mode in [0, MODE_G1, MODE_DRCS_CHARACTER, MODE_DISPLAY_ATTRIBUTES, MODE_MODIFIED_DESIGNATION, 0x20, 0xFF] {
        assert!(matches!(refused([CELL_CHARACTER, b'A', 7, 0, mode, NO_SETS]), Reply::Type(_)), "mode {mode}");
    }
    for code in [0, 0x1F, 0x80, 0xFF] {
        assert!(matches!(refused([CELL_CHARACTER, code, 7, 0, MODE_G0_LEVEL_25, NO_SETS]), Reply::Type(_)), "code {code}");
    }
    assert!(matches!(refused([CELL_CHARACTER, b'A', 7, 0, MODE_G0_LEVEL_25, 0x80]), Reply::Type(_)));
}

/// Section 5.3 and clause 13.1: a frame whose object would pass 507
/// triplets with its Object Definition triplet and Termination Marker is
/// refused by `OBJECT_BODY_TRIPLETS`; one of exactly 505 of its own is
/// taken.
#[test]
fn a_frame_over_an_objects_triplets_is_refused_by_name() {
    // Each cell of a row after the first costs one triplet when the
    // colours hold; a change of ink costs one more.
    let frame = |changes: usize| {
        let mut cells = Vec::new();
        for at in 0..40 * 12 {
            let ink = if at % 40 != 0 && at / 40 == 11 && at % 40 <= changes { u8::try_from(at % 2).unwrap() + 1 } else { 7 };
            cells.extend(cell(CELL_CONTIGUOUS, 1, ink, 0));
        }
        cells
    };
    // Twelve rows of forty cells in one colour: 40 characters, two
    // colours and, after the first row, an address: 42 + 11 × 43 = 515.
    assert!(
        matches!(compile(40, 12, &frame(0)), Err(Reply::Range(text)) if text.starts_with("OBJECT_BODY_TRIPLETS: a frame's object is at most 507 triplets"))
    );
    let mut cells = Vec::new();
    for _ in 0..40 * 11 {
        cells.extend(cell(CELL_CONTIGUOUS, 1, 7, 0));
    }
    // Eleven rows are 42 + 10 × 43 = 472; 33 cells of a twelfth row with
    // their address and colours make 507, two past; 31 make 505.
    let with = |more: usize| {
        let mut cells = cells.clone();
        for at in 0..40 {
            cells.extend(cell(if at < more { CELL_CONTIGUOUS } else { CELL_NONE }, 1, 7, 0));
        }
        compile(40, 12, &cells)
    };
    assert_eq!(with(30).unwrap().len(), MAX_OBJECT_BODY);
    assert!(matches!(with(31), Err(Reply::Range(text)) if text.starts_with("OBJECT_BODY_TRIPLETS: ")));
}

/// Section 5.3 and clause 10.5.1: the Object page as the host lays it
/// out. Frames that are the same are one object; an object's number is
/// its sub-page and its place there (Table 28); a sub-page takes objects
/// while their definitions fit its 507 triplets, less the pointer packets
/// past the second, and holds at most 32; the page context's objects are
/// on the first sub-pages and the channel context's on the last; and an
/// object no frame is goes.
#[test]
fn the_object_page_holds_what_an_object_page_holds() {
    let body = |length: usize, mark: u8| vec![Triplet { address: 0, mode: MODE_G1, data: mark }; length];
    let mut page = ObjectPage::default();
    let first = page.take(ContextId::Page, &body(3, 1)).unwrap();
    assert_eq!((first, page.take(ContextId::Page, &body(3, 1))), (object_number(0, 0).unwrap(), Some(first)));
    assert_eq!((page.len(), page.triplets_used()), (1, 5));
    // The same body for the channel context is its own object, on its
    // own sub-pages.
    let channels = page.take(ContextId::Channel, &body(3, 1)).unwrap();
    assert_eq!(object_place(channels), Some((PAGE_SHARE_OBJECT_SUBPAGES, 0)));
    // 5 + 500 triplets fit sub-page 0; the next object of 2 does too,
    // exactly (507); one more triplet's worth goes to sub-page 1.
    assert_eq!(page.take(ContextId::Page, &body(498, 2)).and_then(object_place), Some((0, 1)));
    assert_eq!(page.take(ContextId::Page, &body(1, 3)).and_then(object_place), Some((1, 0)));
    assert_eq!(page.take(ContextId::Page, &body(0, 4)).and_then(object_place), Some((0, 2)));
    assert_eq!(page.take(ContextId::Page, &body(0, 4)).and_then(object_place), Some((0, 2)));
    // A released object's triplets are free again once no frame is it.
    page.release(first);
    assert_eq!(page.len(), 5);
    page.release(first);
    assert_eq!(page.len(), 4);
    assert_eq!(page.take(ContextId::Page, &body(3, 9)).and_then(object_place), Some((0, 0)));

    // Thirty-two objects a sub-page, and the seventeenth and twenty-fifth
    // each cost a pointer packet of thirteen triplets: with bodies of 12,
    // an object is 14 and sixteen fill 224; then 13 + 17 × 14 = 251 ...
    // all thirty-two fit (26 + 448 = 474), and the thirty-third is on the
    // next sub-page.
    let mut page = ObjectPage::default();
    for index in 0..OBJECTS_PER_SUBPAGE {
        let mark = u8::try_from(index).unwrap();
        assert_eq!(page.take(ContextId::Page, &body(12, mark)).and_then(object_place), Some((0, index)));
    }
    assert_eq!(page.take(ContextId::Page, &body(12, 99)).and_then(object_place), Some((1, 0)));
    // With bodies of 29, an object is 31: sixteen take 496 and fit, and
    // a seventeenth would need a third pointer packet, 13 + 527.
    let mut page = ObjectPage::default();
    for index in 0..16 {
        assert_eq!(page.take(ContextId::Page, &body(29, index)).and_then(object_place), Some((0, usize::from(index))));
    }
    const { assert!(16 * 31 <= SUBPAGE_TRIPLETS) };
    assert_eq!(page.take(ContextId::Page, &body(1, 77)).and_then(object_place), Some((1, 0)));
    // The channel context's two sub-pages fill, and it is told so.
    let mut page = ObjectPage::default();
    assert!(page.take(ContextId::Channel, &body(505, 1)).is_some() && page.take(ContextId::Channel, &body(505, 2)).is_some());
    assert_eq!(page.take(ContextId::Channel, &body(0, 3)), None);
    assert!(page.take(ContextId::Page, &body(505, 3)).is_some());
}

/// Sections 5.3 and 11.2: what a step's batch carries. A sprite made is
/// its objects, the page's sprites whole, and nothing of the page's local
/// enhancement data; a move, a hide and an order are one animation
/// operation each, with the sprites not declared again; and several moves
/// in one step are one.
#[test]
fn a_sprites_changes_leave_as_the_receivers_operations() {
    let mut host = host();
    assert_eq!(sprite(&mut host, &[letter(b'a'), letter(b'b'), letter(b'a')]), Reply::Int(1));
    let made = step(&mut host, 1);
    let objects = made.teletext_objects.unwrap();
    assert_eq!(objects.iter().map(|object| object.number).collect::<Vec<_>>(), vec![0, 16]);
    let declared = made.sprites.unwrap();
    assert_eq!((declared.len(), declared[0].objects.clone(), declared[0].row, declared[0].column), (1, vec![0, 16, 0], 5, 5));
    assert_eq!((made.animate, made.teletext_enhancement), (None, None));

    assert_eq!(animate(&mut host, 1, code::MOVE, [MOVE_BY, 2, 1, 0]), Reply::Undefined);
    assert_eq!(animate(&mut host, 1, code::MOVE, [MOVE_BY, 1, 0, 0]), Reply::Undefined);
    assert_eq!(animate(&mut host, 1, code::HIDE, [0; 4]), Reply::Undefined);
    assert_eq!(animate(&mut host, 1, code::ORDER, [i32::from(KEEP_LAYER), -3, 0, 0]), Reply::Undefined);
    let moved = step(&mut host, 2);
    assert_eq!(
        moved.animate,
        Some(vec![
            object(1, OperationKind::Hide),
            object(1, OperationKind::Move { relative: false, x: 8, y: 6 }),
            object(1, OperationKind::Order { layer: KEEP_LAYER, order: -3 }),
        ])
    );
    assert_eq!((moved.sprites, moved.teletext_objects), (None, None));
    // A step that changes nothing carries nothing.
    let still = step(&mut host, 3);
    assert_eq!((still.animate, still.sprites, still.teletext_objects), (None, None, None));
    assert_eq!(animate(&mut host, 1, code::ORDER, [3, -3, 0, 0]), Reply::Undefined);
    assert_eq!(animate(&mut host, 1, code::MOVE, [MOVE_TO, -39, -24, 0]), Reply::Undefined);
    assert_eq!(
        step(&mut host, 4).animate,
        Some(vec![object(1, OperationKind::Move { relative: false, x: -39, y: -24 }), object(1, OperationKind::Order { layer: 3, order: -3 })])
    );
}

/// Section 5.3 and ANIM-6, ANIM-14: a frame, a cycle and its stop are the
/// operations 17, 18 and 19. A `SPRITES` record forgets every sprite's
/// animation state, so when a sprite is made or removed the page's
/// sprites are declared as they stand and each sprite's frame or cycle
/// is sent after: a cycle that runs once from the frame it had reached,
/// one that repeats from its first.
#[test]
fn a_frame_and_a_cycle_are_the_receivers_and_are_sent_again_after_a_declaration() {
    let mut host = host();
    let frames = [letter(b'a'), letter(b'b'), letter(b'c'), letter(b'd')];
    for handle in 1..=3 {
        assert_eq!(sprite(&mut host, &frames), Reply::Int(handle));
    }
    let _ = step(&mut host, 1);
    assert_eq!(animate(&mut host, 1, code::FRAME, [2, 0, 0, 0]), Reply::Undefined);
    assert_eq!(animate(&mut host, 2, code::CYCLE, [0, 3, 10, 1]), Reply::Undefined);
    assert_eq!(animate(&mut host, 3, code::CYCLE, [1, 3, 5, 0]), Reply::Undefined);
    assert_eq!(animate(&mut host, 1, code::MOVE, [MOVE_TO, 9, 9, 0]), Reply::Undefined);
    let once = OperationKind::Cycle { first: 0, last: 3, period: 10, once: true };
    let repeating = OperationKind::Cycle { first: 1, last: 3, period: 5, once: false };
    assert_eq!(
        step(&mut host, 2).animate,
        Some(vec![object(1, OperationKind::Move { relative: false, x: 9, y: 9 }), object(1, OperationKind::Frame(2)), object(2, once), object(3, repeating)])
    );
    // The cycles began at tick 2. At tick 25 the once cycle shows frame
    // 2. A fourth sprite is made: all are declared, sprite 1 where it now
    // is, and the frame and the cycles follow.
    let _ = step(&mut host, 25);
    assert_eq!(sprite(&mut host, &frames), Reply::Int(4));
    let declared = step(&mut host, 26);
    let sprites = declared.sprites.unwrap();
    assert_eq!(sprites.iter().map(|one| (one.number, one.row, one.column)).collect::<Vec<_>>(), vec![(1, 9, 9), (2, 5, 5), (3, 5, 5), (4, 5, 5)]);
    assert_eq!(declared.teletext_objects, None, "the frames are objects the page has");
    assert_eq!(
        declared.animate,
        Some(vec![object(1, OperationKind::Frame(2)), object(2, OperationKind::Cycle { first: 2, last: 3, period: 10, once: true }), object(3, repeating),])
    );
    // Stopped, a cycle is operation 19; a frame of 0 needs no state.
    assert_eq!(animate(&mut host, 3, code::STOP_CYCLE, [0; 4]), Reply::Undefined);
    assert_eq!(animate(&mut host, 1, code::FRAME, [0; 4]), Reply::Undefined);
    assert_eq!(step(&mut host, 27).animate, Some(vec![object(1, OperationKind::Frame(0)), object(3, OperationKind::StopCycle)]));
    // The once cycle, begun again at tick 26 from frame 2, has ended by
    // tick 40: a removal then declares sprite 2 at its last frame.
    let _ = step(&mut host, 40);
    assert_eq!(host.call(PAGE, Op::TT_REMOVE.number(), &[4], Payload::None), Reply::Undefined);
    let after = step(&mut host, 41);
    assert_eq!(after.sprites.map(|sprites| sprites.len()), Some(3));
    assert_eq!(after.animate.unwrap().first(), Some(&object(2, OperationKind::Frame(3))));
}

/// Section 10.1 rules 3 and 4: an operation on a handle that names no
/// sprite of the context, a removed one's or a forged one, is a
/// `TypeError`; so is an operation a sprite does not take, and one with
/// more whole numbers than it has; and each bound is refused by name with
/// the sprite as it was.
#[test]
fn a_stale_handle_and_a_value_out_of_range_change_nothing() {
    let mut host = host();
    assert_eq!(sprite(&mut host, &[letter(b'a'), letter(b'b')]), Reply::Int(1));
    let _ = step(&mut host, 1);
    for handle in [0, 2, -1, 65_535, i32::MIN] {
        assert!(matches!(animate(&mut host, handle, code::SHOW, [0; 4]), Reply::Type(text) if text == "no such thing"), "handle {handle}");
        assert!(matches!(host.call(PAGE, Op::TT_REMOVE.number(), &[handle], Payload::None), Reply::Type(_)));
        assert!(matches!(host.call(PAGE, Op::TT_LABEL.number(), &[handle], Payload::Text(b"x")), Reply::Type(_)));
        // 0 and -1 are the shared core's two things, `nt` and `nt.sound`.
        if !(-1..=0).contains(&handle) {
            assert!(matches!(host.call(PAGE, Op::LISTEN.number(), &[handle, EventKind::POINTER.number(), 1], Payload::None), Reply::Type(_)));
        }
    }
    // The channel context cannot name the page context's sprite.
    assert!(matches!(host.call(1, Op::TT_ANIMATE.number(), &[1, i32::from(code::HIDE), 0, 0, 0, 0], Payload::None), Reply::Type(_)));
    assert!(matches!(animate(&mut host, 1, code::LOOK, [1, 0, 0, 0]), Reply::Type(_)));
    assert!(matches!(animate(&mut host, 1, code::RESET, [0; 4]), Reply::Type(_)));
    assert!(matches!(animate(&mut host, 1, code::SHOW, [0, 0, 0, 1]), Reply::Type(_)));
    assert!(matches!(animate(&mut host, 1, code::MOVE, [2, 0, 0, 0]), Reply::Type(_)));
    for (operation, numbers, bound) in [
        (code::MOVE, [MOVE_TO, 40, 0, 0], "MAX_SPRITE_COLUMN"),
        (code::MOVE, [MOVE_BY, 0, i32::MAX, 0], "MAX_SPRITE_ROW"),
        (code::ORDER, [0, 0, 0, 0], "LAST_LAYER"),
        (code::ORDER, [2, 128, 0, 0], "SPRITE_ORDER"),
        (code::FRAME, [2, 0, 0, 0], "MAX_OBJECT_FRAMES"),
        (code::CYCLE, [0, 1, 0, 0], "MAX_CYCLE_PERIOD"),
        (code::CYCLE, [0, 1, 3601, 0], "MAX_CYCLE_PERIOD"),
        (code::CYCLE, [1, 0, 5, 0], "MAX_OBJECT_FRAMES"),
    ] {
        assert!(matches!(animate(&mut host, 1, operation, numbers), Reply::Range(text) if text.starts_with(&format!("{bound}: "))), "{bound}");
    }
    let batch = step(&mut host, 2);
    assert_eq!((batch.animate, batch.sprites), (None, None));
    // A sprite's cells are held to its size before any is read.
    let short = host.call(PAGE, Op::TT_SPRITE.number(), &[2, 2, 1, 2, 0, 5, 5, FLAG_SHOWN], Payload::Bytes(&letter(b'a')));
    assert!(matches!(short, Reply::Type(_)));
    assert!(matches!(host.call(PAGE, Op::TT_SPRITE.number(), &[1, 1, 1, 2, 0, 5, 5, 8], Payload::Bytes(&letter(b'a'))), Reply::Type(_)));
}
