//! The snapshot mode's tests (REQ-6885), a child of `engine` so that they
//! compose the grid of what a snapshot gives back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic, reason = "a test")]

use neoteletext_core::charset::to_unicode;
use neoteletext_core::consts::{
    ATTRIBUTE_CONCEAL, COLUMNS, MODE_DISPLAY_ATTRIBUTES, MODE_G0_LEVEL_25, MODE_SET_ACTIVE_POSITION, MODE_TERMINATION, ROW_ADDRESS_BASE, ROW_ADDRESS_LAST,
};
use neoteletext_core::level1::{Glyph, Grid};
use neoteletext_core::limits::SUBCODE_ANY;
use neoteletext_core::packet::{Packet, Triplet};
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::page::{EnhancementPacket, PageFile};
use neoteletext_web::records::naplps_objects::{NaplpsObject, ObjectKind};
use neoteletext_web::records::regis::RegisRecord;
use protocol::decoder::{FromDecoder, SnapshotImage, SnapshotRefusal, ToDecoder, View};

use super::Engine;
use crate::snapshot::{draw, serve};

const PAGE: u16 = 0x100;
/// Spacing attributes (EN 300 706 clause 12.2, Table 26).
const ALPHA_WHITE: u8 = 0x07;
const CONCEAL: u8 = 0x18;
const MOSAIC_WHITE: u8 = 0x17;
const MOSAIC_YELLOW: u8 = 0x13;
const HOLD_MOSAICS: u8 = 0x1E;
const NEW_BACKGROUND: u8 = 0x1D;
/// A full block in the mosaic set (clause 15.7, Figure 23).
const FULL_BLOCK: u8 = 0x7F;

fn address() -> PageAddress {
    PageAddress::from_mpp(PAGE).unwrap()
}

/// A row's 40 codes from `text`, spaces after it.
fn row(text: &[u8]) -> [u8; COLUMNS] {
    let mut codes = [b' '; COLUMNS];
    codes[..text.len()].copy_from_slice(text);
    codes
}

/// A page file of page 100, sub-page `sub`, with `rows` and the triplets
/// of one X/26 packet (the rest of it terminated), as T42 packets.
fn packets(sub: u16, rows: &[(usize, [u8; COLUMNS])], triplets: &[Triplet]) -> Vec<u8> {
    let mut file = PageFile { address: address(), subcode: SubCode::new(sub), ..PageFile::default() };
    for (number, codes) in rows {
        file.set_row(*number, codes);
        file.rows_present |= 1 << number;
    }
    if !triplets.is_empty() {
        let end = Triplet { address: ROW_ADDRESS_LAST, mode: MODE_TERMINATION, data: 0 };
        let mut packet = EnhancementPacket { designation: 0, triplets: [end; 13] };
        packet.triplets[..triplets.len()].copy_from_slice(triplets);
        assert!(file.enhancements.push(packet));
    }
    let mut bytes = Vec::new();
    neoteletext_formats::encode::encode_page(&file, &mut |packet: Packet| bytes.extend_from_slice(&packet.bytes));
    bytes
}

/// The X/26 triplets that put the active position at `row`, `column`.
fn at(row: u8, column: u8) -> [Triplet; 2] {
    [Triplet { address: ROW_ADDRESS_BASE + row, mode: MODE_SET_ACTIVE_POSITION, data: 0 }, Triplet { address: column, mode: MODE_DISPLAY_ATTRIBUTES, data: 0 }]
}

/// A page with concealed text three ways: after a Conceal spacing
/// attribute on row 3 ("XYZZY"), a character an X/26 triplet writes into
/// that concealed run ("J"), and a run row 5's X/26 display attributes
/// conceal ("KMVW"), beside text shown ("PUBLIC"): letters no cell shows,
/// for a code shown elsewhere is kept (REQ-6885 item 4).
fn concealed_page() -> Vec<u8> {
    let triplets = [
        // Row 3, column 14: a G0 character, J, inside the run Conceal began.
        Triplet { address: ROW_ADDRESS_BASE + 3, mode: MODE_SET_ACTIVE_POSITION, data: 0 },
        Triplet { address: 14, mode: MODE_G0_LEVEL_25, data: b'J' },
        // Row 5 from column 10: concealed by its display attributes.
        Triplet { address: ROW_ADDRESS_BASE + 5, mode: MODE_SET_ACTIVE_POSITION, data: 0 },
        Triplet { address: 10, mode: MODE_DISPLAY_ATTRIBUTES, data: ATTRIBUTE_CONCEAL },
    ];
    packets(
        0,
        &[
            (1, row(b"PUBLIC HEADLINE")),
            (3, row(&[ALPHA_WHITE, b'O', b'P', b'E', b'N', b' ', CONCEAL, b'X', b'Y', b'Z', b'Z', b'Y'])),
            (5, row(b"PUBLIC    KMVW")),
        ],
        &triplets,
    )
}

/// The grid of `bytes`' page 100 as a snapshot draws it.
fn grid_of(bytes: &[u8]) -> Grid {
    let mut engine = Engine::new();
    engine.feed_whole(true, false, bytes);
    let view = View { page: PAGE, subpage: Some(0), ..View::default() };
    let stored = *engine.stored(PAGE, Some(0)).unwrap();
    engine.compose(PAGE, &stored, view)
}

/// Every character a grid's cells hold, concealed or not: what it shows
/// with reveal on.
fn revealed_text(grid: &Grid) -> String {
    grid.cells
        .iter()
        .flatten()
        .map(|cell| match cell.glyph {
            Glyph::Char(character) => to_unicode(character).base,
            _ => ' ',
        })
        .collect()
}

/// Only what is shown with reveal off.
fn shown_text(grid: &Grid) -> String {
    grid.cells
        .iter()
        .flatten()
        .map(|cell| match cell.glyph {
            Glyph::Char(character) if !cell.conceal => to_unicode(character).base,
            _ => ' ',
        })
        .collect()
}

fn frame(image: &SnapshotImage) -> &[u8] {
    match image {
        SnapshotImage::Frame { indices, .. } => indices,
        SnapshotImage::Regis(_) => panic!("a teletext page's snapshot is a frame"),
    }
}

/// REQ-6885 items 3 and 4: a complete sub-page is drawn, its sub-page
/// named, and its source given back as whole packets.
#[test]
fn a_complete_sub_page_is_drawn_and_its_source_given_back() {
    for bytes in [packets(0, &[(1, row(b"Headlines"))], &[]), b"PN,10000\nSC,0000\nPS,8000\nOL,1,Headlines\n".to_vec()] {
        let snapshot = draw(PAGE, SUBCODE_ANY, &bytes).unwrap();
        assert_eq!(snapshot.sub, 0);
        assert!(!snapshot.source.is_empty() && snapshot.source.len().is_multiple_of(42));
        assert!(shown_text(&grid_of(&snapshot.source)).contains("Headlines"));
    }
}

/// REQ-6885 item 3: no such sub-page, no such page, and the first sub-page
/// in ascending order for 3F7F.
#[test]
fn a_page_with_no_such_sub_page_is_refused() {
    let mut bytes = packets(2, &[(1, row(b"Second"))], &[]);
    bytes.extend(packets(1, &[(1, row(b"First"))], &[]));
    assert_eq!(draw(0x200, SUBCODE_ANY, &bytes).err(), Some(SnapshotRefusal::NoSuchSubPage));
    assert_eq!(draw(PAGE, 5, &bytes).err(), Some(SnapshotRefusal::NoSuchSubPage));
    assert_eq!(draw(PAGE, SUBCODE_ANY, &bytes).unwrap().sub, 1, "DEC-1526: the first in ascending order");
    assert_eq!(draw(PAGE, 2, &bytes).unwrap().sub, 2);
}

/// REQ-6885 item 3: bytes no reader takes, or packets that are not whole,
/// are refused.
#[test]
fn bytes_a_reader_rejects_are_refused() {
    let mut torn = packets(0, &[(1, row(b"Headlines"))], &[]);
    torn.pop();
    for bytes in [vec![0xFF; 7], torn, Vec::new()] {
        assert!(draw(PAGE, SUBCODE_ANY, &bytes).is_err());
    }
}

/// REQ-6885 item 2: the snapshot is drawn with reveal off: its image is
/// the page's with reveal off, and differs from it revealed.
#[test]
fn concealed_text_is_drawn_concealed_whatever_the_view() {
    let bytes = concealed_page();
    let snapshot = draw(PAGE, SUBCODE_ANY, &bytes).unwrap();
    let mut engine = Engine::new();
    engine.feed_whole(true, false, &bytes);
    engine.set_view(View { page: PAGE, reveal: true, subpage: Some(0), ..View::default() });
    let revealed = engine.render_at(PAGE, 0).unwrap();
    assert_ne!(frame(&snapshot.image), revealed.indices.as_slice(), "the view's reveal is not the snapshot's");
    let original = grid_of(&bytes);
    let (_, _, hidden) = super::snapshot::render(&original, engine.config());
    assert_eq!(frame(&snapshot.image), hidden.as_slice());
}

/// REQ-6885 item 4: the source holds none of the concealed characters,
/// whether a spacing attribute, an X/26 character or X/26 display
/// attributes concealed them, and drawn with reveal off it gives the
/// original's image; the text shown is kept.
#[test]
fn concealed_characters_are_blanked_in_the_source() {
    let bytes = concealed_page();
    let original = grid_of(&bytes);
    let (revealed, shown) = (revealed_text(&original), shown_text(&original));
    for hidden in ["XYZZY", "KMVW", "J"] {
        assert!(revealed.contains(hidden) && !shown.contains(hidden), "the page conceals {hidden}: {revealed:?}");
    }
    let snapshot = draw(PAGE, SUBCODE_ANY, &bytes).unwrap();
    let blanked = grid_of(&snapshot.source);
    let text = revealed_text(&blanked);
    for hidden in ["XYZZY", "KMVW", "J"] {
        assert!(!text.contains(hidden), "{hidden} in {text:?}");
    }
    assert!(text.contains("PUBLIC HEADLINE") && text.contains("OPEN"), "{text:?}");
    let again = draw(PAGE, SUBCODE_ANY, &snapshot.source).unwrap();
    assert_eq!(frame(&again.image), frame(&snapshot.image), "the same image with reveal off");
}

/// REQ-6885 item 4's second property: drawn with reveal on, the blanked
/// source shows no character that the original shows in no cell with
/// reveal off.
#[test]
fn the_blanked_source_drawn_with_reveal_on_shows_nothing_the_original_hid() {
    let bytes = concealed_page();
    let original = grid_of(&bytes);
    let shown = shown_text(&original);
    let blanked = grid_of(&draw(PAGE, SUBCODE_ANY, &bytes).unwrap().source);
    for character in revealed_text(&blanked).chars().filter(|character| *character != ' ') {
        assert!(shown.contains(character), "{character:?} is shown by no cell of the original");
    }
}

/// REQ-6885 item 4: a mosaic under Hold Mosaics in a concealed run that a
/// later spacing attribute's cell shows is kept, so the page looks the
/// same.
#[test]
fn a_held_mosaic_shown_past_a_concealed_run_is_kept() {
    let bytes = packets(0, &[(2, row(&[MOSAIC_WHITE, HOLD_MOSAICS, CONCEAL, FULL_BLOCK, MOSAIC_YELLOW, NEW_BACKGROUND, b'x']))], &[]);
    let original = grid_of(&bytes);
    let snapshot = draw(PAGE, SUBCODE_ANY, &bytes).unwrap();
    let blanked = grid_of(&snapshot.source);
    assert_eq!(blanked.cells[2][3].glyph, original.cells[2][3].glyph, "the held mosaic's source stays");
    let again = draw(PAGE, SUBCODE_ANY, &snapshot.source).unwrap();
    assert_eq!(frame(&again.image), frame(&snapshot.image));
}

/// REQ-6885 items 2 and 4: a ReGIS sub-page is drawn whole as its picture,
/// and its record's packets follow its own in the source.
#[test]
fn a_regis_picture_is_drawn_whole_and_its_records_follow_its_packets() {
    let mut bytes = packets(0, &[(1, row(b"Picture"))], &[]);
    let teletext = bytes.len();
    let mut record = Vec::new();
    let stream = RegisRecord { page: address(), subcode: SubCode::new(0), generation: 1, offset: 0, data: b"P[10,10]V[700,400]" };
    neoteletext_web::records::regis::encode(stream, 0, 0, &mut |packet| record.extend_from_slice(&packet.bytes)).unwrap();
    bytes.extend_from_slice(&record);
    let snapshot = draw(PAGE, SUBCODE_ANY, &bytes).unwrap();
    match &snapshot.image {
        SnapshotImage::Regis(picture) => assert!(picture.whole()),
        SnapshotImage::Frame { .. } => panic!("a ReGIS sub-page's snapshot is its picture"),
    }
    assert!(snapshot.source.ends_with(&record), "the record's packets follow");
    assert!(snapshot.source.len() >= teletext + record.len());
    assert!(matches!(draw(PAGE, SUBCODE_ANY, &snapshot.source).unwrap().image, SnapshotImage::Regis(_)), "drawn again from its source");
}

/// REQ-6885 item 4: a NAPLPS picture keeps its `NAPLPS_OBJECTS` record.
#[test]
fn a_naplps_picture_keeps_its_objects_record() {
    let mut bytes = packets(0, &[(1, row(b"Picture"))], &[]);
    let stream = RegisRecord { page: address(), subcode: SubCode::new(0), generation: 1, offset: 0, data: b"A" };
    neoteletext_web::records::regis::encode_naplps(stream, 0, 0, &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
    let mut objects = Vec::new();
    let object = NaplpsObject {
        number: 1,
        kind: ObjectKind::Macro,
        shown: true,
        detectable: true,
        layer: 2,
        order: 0,
        x: 100,
        y: 100,
        width: 60,
        height: 60,
        origin: (0, 420),
        cover: 0,
        looks: [0; 3],
        frames: &[0x20],
    };
    neoteletext_web::records::naplps_objects::encode((address(), SubCode::new(0), 1), &[object], 0, 0, &mut |packet| objects.extend_from_slice(&packet.bytes))
        .unwrap();
    bytes.extend_from_slice(&objects);
    let snapshot = draw(PAGE, SUBCODE_ANY, &bytes).unwrap();
    assert!(matches!(snapshot.image, SnapshotImage::Regis(_)));
    assert!(snapshot.source.ends_with(&objects), "the objects' record is kept with its picture");
}

/// REQ-6885 item 4: the source, drawn again, gives the same image.
#[test]
fn the_source_drawn_again_gives_the_same_image() {
    for bytes in [concealed_page(), packets(0, &[(1, row(b"Weather")), (2, row(&[MOSAIC_WHITE, FULL_BLOCK, FULL_BLOCK]))], &at(1, 3))] {
        let first = draw(PAGE, SUBCODE_ANY, &bytes).unwrap();
        let second = draw(PAGE, SUBCODE_ANY, &first.source).unwrap();
        assert_eq!(frame(&first.image), frame(&second.image));
        assert_eq!(first.source, second.source, "blanking is settled after once");
    }
}

/// REQ-6885 item 1: the snapshot mode takes its bytes, its place and
/// Close, and ends at anything else; it answers with the source's pieces
/// and then its image, and nothing else.
#[test]
fn snapshot_mode_accepts_only_whole_snapshot_and_close() {
    let mut input = Vec::new();
    protocol::decoder::write_to_decoder(&mut input, &ToDecoder::Whole { packets: true, pushed: false, bytes: concealed_page() }).unwrap();
    protocol::decoder::write_to_decoder(&mut input, &ToDecoder::Snapshot { page: PAGE, sub: SUBCODE_ANY }).unwrap();
    let mut output = Vec::new();
    serve(&mut input.as_slice(), &mut output).unwrap();
    let mut read = output.as_slice();
    let mut kinds = Vec::new();
    while let Some(message) = protocol::decoder::read_from_decoder(&mut read).unwrap() {
        kinds.push(match message {
            FromDecoder::SnapshotSource(_) => "source",
            FromDecoder::SnapshotDrawn { sub: 0, .. } => "drawn",
            other => panic!("{other:?}"),
        });
    }
    assert_eq!(kinds, ["source", "drawn"]);
    for refused in [ToDecoder::Showing(vec![PAGE]), ToDecoder::Data(vec![1]), ToDecoder::End, ToDecoder::Opened(None)] {
        let mut input = Vec::new();
        protocol::decoder::write_to_decoder(&mut input, &refused).unwrap();
        assert!(serve(&mut input.as_slice(), &mut Vec::new()).is_err(), "{refused:?}");
    }
    let mut input = Vec::new();
    protocol::decoder::write_to_decoder(&mut input, &ToDecoder::Close).unwrap();
    let mut output = Vec::new();
    serve(&mut input.as_slice(), &mut output).unwrap();
    assert!(output.is_empty(), "Close ends it with no answer");
}
