//! A script's batch applied to the open page (script-api.md section 11.2;
//! TODO-7293's checks): a child of `engine` so that the tests reach its
//! store and its grid.

use super::script::{NotApplied, ScriptPage};
use super::*;
use neoteletext_core::charset::to_unicode;
use neoteletext_core::consts::COLUMNS;
use neoteletext_core::level1::Glyph;
use neoteletext_core::types::{PageAddress, SubCode};
use protocol::script::batch::{NaplpsAppend, TeletextRow, TeletextSettings, Triplet};
use protocol::script::{Batch, Renderer};

/// Pages 100 and 101, each with a header, rows and a row 24.
const TTI: &[u8] = b"PN,10000\nSC,0000\nPS,8000\nOL,0,        HEADER\nOL,1,FILE ROW ONE\nOL,3,FILE ROW THREE\nOL,24,FILE KEYS\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,Sport results\nOL,5,Other page\n";

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine.feed(TTI);
    engine.finish();
    engine
}

fn page(renderer: Renderer) -> ScriptPage {
    ScriptPage { page: 0x100, sub: 0, renderer, designation: neoteletext_core::charset::language::language_value("en", true) }
}

/// A row of 40 codes that begins with `text`.
fn row(number: u8, text: &str) -> TeletextRow {
    let mut codes = [b' '; COLUMNS];
    for (slot, byte) in codes.iter_mut().zip(text.bytes()) {
        *slot = byte;
    }
    TeletextRow { row: number, codes }
}

fn shown(engine: &Engine, page: u16, number: usize) -> String {
    let (grid, _) = engine.grid(page).unwrap();
    let text: String = grid.cells[number]
        .iter()
        .map(|cell| match cell.glyph {
            Glyph::Char(character) => to_unicode(character).base,
            _ => ' ',
        })
        .collect();
    text.trim().to_owned()
}

/// The stored cells of every row of a page, raw.
fn cells(engine: &Engine, page: u16) -> Vec<[u8; COLUMNS]> {
    engine.decoder.store().get(PageAddress::from_mpp(page).unwrap(), SubCode::new(0)).unwrap().cells.to_vec()
}

/// Section 11.2 and TODO-7293's check: a batch's rows replace those rows
/// of the open page and no other; rows 0 and 24 are untouched, to the
/// byte; no other page of the store changes; and the page file is what
/// shows where the script wrote nothing (section 12.3).
#[test]
fn a_batch_writes_rows_of_the_open_page_only_and_never_row_0_or_24() {
    let mut engine = engine();
    let (before, other) = (cells(&engine, 0x100), cells(&engine, 0x101));
    let batch = Batch { teletext_rows: Some(vec![row(1, "SCRIPT ONE"), row(5, "SCRIPT FIVE"), row(23, "SCRIPT LAST")]), ..Batch::new(0) };
    let arrived = engine.apply_script(page(Renderer::Teletext), &batch, true).unwrap();
    assert!(arrived.iter().all(|(page, _)| *page == 0x100), "{arrived:?}");
    assert_eq!(
        [1, 3, 5, 23].map(|number| shown(&engine, 0x100, number)),
        ["SCRIPT ONE".to_owned(), "FILE ROW THREE".to_owned(), "SCRIPT FIVE".to_owned(), "SCRIPT LAST".to_owned()]
    );
    let after = cells(&engine, 0x100);
    assert_eq!((after[0], after[24]), (before[0], before[24]), "rows 0 and 24 are the page file's");
    assert_eq!(cells(&engine, 0x101), other, "another page of the store");
    assert_eq!(shown(&engine, 0x101, 5), "Other page");
}

/// Section 11.2 (the security review's finding S-08, TODO-7274): a batch
/// holds typed sections only, and one of another renderer than the open
/// page's is refused whole: nothing of it is applied.
#[test]
fn a_section_of_another_renderer_refuses_the_batch_whole() {
    let mut engine = engine();
    let before = cells(&engine, 0x100);
    let mixed = Batch { teletext_rows: Some(vec![row(1, "DRAWN")]), regis: Some(b"S(E)".to_vec()), ..Batch::new(0) };
    assert!(matches!(engine.apply_script(page(Renderer::Teletext), &mixed, true), Err(NotApplied(_))));
    let naplps = Batch { naplps: Some(NaplpsAppend { new_generation: true, bytes: vec![0x20] }), ..Batch::new(0) };
    assert!(engine.apply_script(page(Renderer::Teletext), &naplps, true).is_err());
    let rows = Batch { teletext_rows: Some(vec![row(1, "DRAWN")]), ..Batch::new(0) };
    assert!(engine.apply_script(page(Renderer::Regis), &rows, false).is_err());
    assert!(engine.apply_script(page(Renderer::Naplps), &Batch { regis: Some(b"S(E)".to_vec()), ..Batch::new(0) }, false).is_err());
    assert_eq!(cells(&engine, 0x100), before);
    assert_eq!(engine.script_renderer(0x100, 0), Some(Renderer::Teletext), "no stream was started on a teletext page");
}

/// TODO-7362 item 2(b) and (c): the enhancement section is the page's
/// whole local enhancement data, ended by a Termination Marker, and an
/// empty one leaves the page none; the packet X/28/0 carries the
/// language's designation and the batch's settings.
#[test]
fn the_enhancement_data_is_applied_whole_and_the_page_data_carries_the_settings() {
    let mut engine = engine();
    let at = page(Renderer::Teletext);
    // Set Active Position row 2, then a character at column 3 (EN 300 706
    // Table 27: row address 40 + 2, mode 00100; column mode 01111, G2).
    let two = vec![Triplet { address: 42, mode: 0b0_0100, data: 0 }, Triplet { address: 3, mode: 0b0_1111, data: 0x2A }];
    let settings = TeletextSettings { screen_colour: 4, row_colour: 1, substitution: true, colours: vec![(16, 0xFA0), (31, 0x123)] };
    let batch = Batch { teletext_enhancement: Some(two), teletext_settings: Some(settings), ..Batch::new(0) };
    engine.apply_script(at, &batch, true).unwrap();
    let stored = *engine.decoder.store().get(PageAddress::from_mpp(0x100).unwrap(), SubCode::new(0)).unwrap();
    assert_eq!(stored.enhancements_present, 1, "one packet X/26");
    let packet = stored.enhancements[0];
    assert_eq!((packet[0].unwrap().address, packet[1].unwrap().data), (42, 0x2A));
    let end = neoteletext_core::packet::TripletFunction::Termination { kind: 0b111 }.to_triplet();
    assert!(packet[2..].iter().all(|triplet| *triplet == Some(end)), "the rest is the Termination Marker");
    let data = stored.page_data.unwrap().fields;
    assert_eq!(data.designations.unwrap().default, at.designation.unwrap());
    let colours = data.colours.unwrap();
    assert_eq!((colours.screen, colours.row, colours.black_background_substitution), (4, 1, true));
    let words = data.colour_words.unwrap();
    assert_eq!((words[0].0, words[15].0, words[1].0), (0xFA0, 0x123, neoteletext_core::consts::COLOUR_MAP_DEFAULTS[17]));

    // An empty list: the page has no enhancement data any more.
    engine.apply_script(at, &Batch { teletext_enhancement: Some(Vec::new()), ..Batch::new(1) }, false).unwrap();
    let stored = *engine.decoder.store().get(PageAddress::from_mpp(0x100).unwrap(), SubCode::new(0)).unwrap();
    assert_eq!(stored.enhancements[0][0], Some(end));
    assert!(stored.page_data.is_some(), "a batch without settings leaves the page's own");
}

/// Section 12.3 and 13 items 3 and 4: a JavaScript page opens blank in
/// its renderer; a ReGIS batch appends to the open sub-page's one stream,
/// and a NAPLPS batch that asks a new generation starts one.
#[test]
fn a_javascript_page_opens_blank_and_its_streams_are_the_open_sub_pages() {
    let mut engine = engine();
    let blank = ScriptPage { page: 0x1A0, sub: 2, renderer: Renderer::Teletext, designation: None };
    assert_eq!(engine.script_renderer(0x1A0, 2), None);
    engine.script_blank_page(blank);
    assert_eq!(engine.script_renderer(0x1A0, 2), Some(Renderer::Teletext));
    assert!(engine.script_blank_page(blank).is_empty(), "a page that is held is left as it is");

    let regis = ScriptPage { page: 0x200, sub: 0, renderer: Renderer::Regis, designation: None };
    engine.apply_script(regis, &Batch { regis: Some(b"S(E)P[10,10]".to_vec()), ..Batch::new(0) }, false).unwrap();
    assert_eq!(engine.regis.stream_end(0x200, 0), Some((1, 12)));
    engine.apply_script(regis, &Batch { regis: Some(b"V[20,20]".to_vec()), ..Batch::new(1) }, false).unwrap();
    assert_eq!(engine.regis.stream_end(0x200, 0), Some((1, 20)), "appended to the one stream");
    assert_eq!((engine.script_renderer(0x200, 0), engine.regis.stream_end(0x100, 0), engine.regis.stream_end(0x200, 1)), (Some(Renderer::Regis), None, None));

    let naplps = ScriptPage { page: 0x300, sub: 0, renderer: Renderer::Naplps, designation: None };
    let append = |new_generation: bool| Batch { naplps: Some(NaplpsAppend { new_generation, bytes: vec![0x20, 0x20] }), ..Batch::new(0) };
    engine.apply_script(naplps, &append(true), false).unwrap();
    engine.apply_script(naplps, &append(false), false).unwrap();
    assert_eq!(engine.naplps.stream_end(0x300, 0), Some((1, 4)));
    engine.apply_script(naplps, &append(true), false).unwrap();
    assert_eq!(engine.naplps.stream_end(0x300, 0), Some((2, 2)), "a new generation from its first byte");
}

/// Section 4.4: what the pointer hit is named by the script host's
/// handle; on a teletext page the pointer's place is its cell.
#[test]
fn a_pointer_is_a_cell_on_a_teletext_page() {
    use neoteletext_core::consts::{CELL_HEIGHT, CELL_WIDTH};
    let pixel = |cells: usize, size: usize| u16::try_from(cells * size).unwrap();
    assert_eq!(script::cell_at(0, 0), (0, 0));
    assert_eq!(script::cell_at(pixel(39, CELL_WIDTH) + 5, pixel(24, CELL_HEIGHT) + 1), (39, 24));
    assert_eq!(script::cell_at(u16::MAX, u16::MAX), (39, 24), "a pixel past the picture is its last cell");
    assert!(engine().script_pick(page(Renderer::Teletext), 10, 10).is_empty(), "a page with no control");
}
