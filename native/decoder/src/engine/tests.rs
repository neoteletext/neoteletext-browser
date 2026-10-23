//! The engine's tests, a child of `engine` so that they reach its views,
//! its digests and its grid.

use super::feed::MAX_LOGS_HELD;
use super::*;
use crate::{Flashing, sample};
use neoteletext_core::charset::to_unicode;
#[cfg(feature = "links")]
use neoteletext_core::consts::ROWS;
use neoteletext_core::consts::{FLASH_CYCLE_US, FLASH_UNITS};
use neoteletext_core::level1::Glyph;
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::Item;
use neoteletext_formats::item::Reader;
use protocol::audio::Sound;
use std::fmt::Write as _;

/// Two pages in TTI, the engine's page file format: page 100 with a
/// coloured title, and page 101.
const TTI: &[u8] = b"PN,10000\nSC,0000\nPS,8000\nOL,0,        HEADER\nOL,1,\x1b\x41RED HEADLINES\nOL,3,\x1b\x47News of the day\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,Sport results\n";

fn engine_with(tti: &[u8]) -> Engine {
    let mut engine = Engine::new();
    engine.feed(tti);
    engine.finish();
    engine
}

/// The engine's REQ-INP-026 and decision 70: a page's `LG` lines reach
/// the trusted core with their level and page, and no more than are
/// held at once.
#[test]
fn a_channels_log_lines_are_taken_with_their_page() {
    let mut engine = Engine::new();
    let mut file = String::from("LG,I,Channel started\nPN,10000\nSC,0000\nPS,8000\nLG,W,Weather feed late\nOL,1,Headlines\n");
    for line in 0..MAX_LOGS_HELD {
        let _ = write!(file, "PN,2{:02}00\nLG,E,Line {line}\nOL,1,Page\n", line % 90);
    }
    engine.feed(file.as_bytes());
    engine.finish();
    let logs = engine.take_logs();
    assert_eq!(logs.len(), MAX_LOGS_HELD, "held to the bound");
    assert_eq!(logs[0], ChannelLog { level: interface_core::console::Level::Info, page: None, text: "Channel started".into() });
    assert_eq!(logs[1], ChannelLog { level: interface_core::console::Level::Warning, page: Some(0x100), text: "Weather feed late".into() });
    assert!(engine.take_logs().is_empty(), "taken once");
}

/// REQ-NAV-011 and the engine's E.108: a page file whose `LN` lines
/// give its keys, and no row 24 of its own, shows the receiver's row 24
/// with each key's label, so its keys act (REQ-SEC-024); a link to
/// another service is null and its key does nothing.
#[cfg(feature = "links")]
#[test]
fn link_lines_draw_the_receivers_row_24() {
    let engine = engine_with(b"PN,10000\nSC,0000\nPS,8000\nOL,1,INDEX\nLN,R,200,Headlines\nLN,G,sport:300,Sport\nLN,I,100,\n");
    let (grid, _) = engine.grid(0x100).unwrap();
    let row: String = grid.cells[ROWS - 1]
        .iter()
        .map(|cell| match cell.glyph {
            Glyph::Char(character) => to_unicode(character).base,
            _ => ' ',
        })
        .collect();
    // The broadcast row, which the page file's encoding carries, shows
    // another service's label in white; the receiver's blanks it.
    assert!(row.contains("Headlines") && !row.contains("Sport"), "the receiver's row 24: {row:?}");
    let rendered = engine.render_at(0x100, 0).unwrap();
    assert!(rendered.links.fastext.row24, "the keys act");
    assert!(rendered.links.fastext.keys[0].is_some() && rendered.links.fastext.keys[1].is_none(), "red leads to 200; green's other service is null");
}

/// The engine's E.107: a page drawn over its last frame, only its
/// changed rows again, is the page drawn whole, through every flash
/// phase and a new version of the page.
#[test]
fn a_frame_drawn_over_the_last_is_the_frame_drawn_whole() {
    let mut engine = engine_with(b"PN,10000\nSC,0000\nPS,8000\nOL,1,HEADLINES\nOL,5,\x1bHFLASHING\nOL,9,\x1bA\x1bHRED FLASH\nOL,12,STEADY\n");
    let mut last = engine.render_at(0x100, 0).unwrap().indices;
    let step = FLASH_CYCLE_US / u64::from(FLASH_UNITS);
    let mut redrawn = 0_usize;
    for unit in 1..=u64::from(FLASH_UNITS) * 2 {
        let now = unit * step;
        let over = engine.render_over(0x100, now, Some(&last), Vec::new()).unwrap();
        let whole = engine.render_into(0x100, now, Vec::new()).unwrap();
        assert!(over.indices == whole.indices, "phase at {now} µs");
        redrawn += usize::from(over.indices != last);
        // `render_into` moved the tracker on to the same frame.
        last = whole.indices;
    }
    assert!(redrawn > 0, "the page flashed");
    engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,HEADLINES\nOL,5,\x1bHFLASHING\nOL,12,CHANGED\n");
    engine.finish();
    let over = engine.render_over(0x100, 0, Some(&last), Vec::new()).unwrap();
    assert!(over.indices == engine.render_into(0x100, 0, Vec::new()).unwrap().indices, "a new version");
}

/// Decision 67, REQ-L1-010: concealed text is drawn only while
/// revealed; mix changes the page's backgrounds.
#[test]
fn reveal_and_mix_change_the_page_drawn() {
    // Row 1: a blue background (alpha blue 0x04, then new background
    // 0x1D, which takes the foreground's colour), white text (0x07),
    // concealed (0x18). Each code is written ESC and the code plus 40 in
    // TTI.
    let mut engine = engine_with(b"PN,10000\nSC,0000\nPS,8000\nOL,1,\x1bD\x1b]\x1bG\x1bXSECRET ANSWER\n");
    let hidden = engine.render_at(0x100, 0).unwrap();
    engine.set_view(View { page: 0x100, reveal: true, ..View::default() });
    let shown = engine.render_at(0x100, 0).unwrap();
    assert_ne!(hidden.indices, shown.indices, "revealed text is drawn");
    engine.set_view(View { page: 0x100, mix: true, ..View::default() });
    assert_ne!(engine.render_at(0x100, 0).unwrap().indices, hidden.indices, "mix clears the blue background");
    engine.set_view(View { page: 0x100, ..View::default() });
    assert_eq!(engine.render_at(0x100, 0).unwrap().indices, hidden.indices, "as before, the view forgotten");
    assert!(engine.views.is_empty());
}

/// REQ-NAV-005: every sub-page is kept and listed; the latest is drawn
/// unless one is chosen.
#[test]
fn sub_pages_are_listed_and_one_can_be_chosen() {
    let mut engine = engine_with(b"PN,10001\nSC,0001\nPS,8000\nOL,1,FIRST\n\nPN,10002\nSC,0002\nPS,8000\nOL,1,SECOND\n");
    let latest = engine.render_at(0x100, 0).unwrap();
    assert_eq!(latest.subpages, (vec![1, 2], 2), "the latest received is drawn");
    engine.set_view(View { page: 0x100, subpage: Some(1), ..View::default() });
    let first = engine.render_at(0x100, 0).unwrap();
    assert_eq!(first.subpages, (vec![1, 2], 1));
    assert_ne!(first.indices, latest.indices);
    engine.set_view(View { page: 0x100, subpage: Some(9), ..View::default() });
    assert_eq!(engine.render_at(0x100, 0).unwrap().subpages.1, 2, "one not stored: the latest");
}

/// REQ-NAV-006: a held page keeps the version it showed until let go.
#[test]
fn a_held_page_keeps_its_version() {
    let mut engine = engine_with(b"PN,10000\nSC,0000\nPS,8000\nOL,1,MORNING\n");
    let morning = engine.render_at(0x100, 0).unwrap();
    engine.set_view(View { page: 0x100, hold: true, ..View::default() });
    engine.feed_whole(false, true, b"PN,10000\nSC,0000\nPS,8000\nOL,1,EVENING\n");
    assert_eq!(engine.render_at(0x100, 0).unwrap().indices, morning.indices, "held");
    engine.set_view(View { page: 0x100, ..View::default() });
    assert_ne!(engine.render_at(0x100, 0).unwrap().indices, morning.indices, "let go, the new version");
}

/// REQ-NAV-003 and REQ-SEC-024: a TTI page's FL line becomes its
/// coloured keys and Index, which act only while row 24 is drawn.
#[cfg(feature = "links")]
#[test]
fn fastext_comes_from_the_link_line_and_needs_row_24() {
    let with_row = engine_with(b"PN,10000\nSC,0000\nPS,8000\nOL,1,INDEX\nOL,24,\x1bAnews \x1bBsport\nFL,200,300,8ff,400,8ff,101\n");
    let fastext = with_row.render_at(0x100, 0).unwrap().links.fastext;
    let page = |fastext: &protocol::links::Fastext, key: usize| fastext.keys[key].map(|target| target.page);
    assert_eq!([0, 1, 2, 3, 4].map(|key| page(&fastext, key)), [Some(0x200), Some(0x300), None, Some(0x400), Some(0x101)]);
    assert!(fastext.row24);
    let without_row = engine_with(b"PN,10000\nSC,0000\nPS,8000\nOL,1,INDEX\nFL,200,300,8ff,400,8ff,101\n");
    assert!(!without_row.render_at(0x100, 0).unwrap().links.fastext.row24, "no row 24: the keys do nothing");
}

#[test]
fn pages_arrive_with_titles_and_render_in_teletext_colours() {
    let mut engine = Engine::new();
    let mut arrived = engine.feed(TTI);
    arrived.extend(engine.finish());
    assert_eq!(arrived, [(0x100, "RED HEADLINES".to_owned()), (0x101, "Sport results".to_owned())]);
    let page = engine.render(0x100).unwrap();
    assert_eq!(page.indices.len(), PAGE_DOTS);
    assert!(page.palette.len() <= 32);
    assert!(page.indices.iter().all(|&index| usize::from(index) < page.palette.len()));
    // Some dot of row 1 is drawn in red: alpha-red is code 0x01 (Table 26).
    let row_one = &page.indices[usize::from(PAGE_WIDTH) * 20..usize::from(PAGE_WIDTH) * 40];
    assert!(row_one.iter().any(|&index| page.palette[usize::from(index)] == [0xFF, 0, 0]), "no red in row 1");
    assert!(engine.render(0x102).is_none(), "a page never received");
}

/// Decision 60: the Colour Map of EN 300 706 clause 12.4 reaches the
/// frame. Page 100's X/28/0 (Table 4) redefines CLUTs 2 and 3 and sets
/// Colour Table Re-mapping 5, so the Level 1 foreground colours come
/// from CLUT 2; page 101's X/28/4 (Table 10, Level 3.5) redefines CLUTs
/// 0 and 1, but never CLUT 1 entry 0, which stays transparent.
#[test]
fn a_page_redefines_its_colour_map() {
    use neoteletext_core::ecc::with_odd_parity;
    use neoteletext_core::packet::{PageFormat, encode};
    use neoteletext_core::types::{ControlBits, DefaultColourFields, Designations, PageAddress, PresentationFields, Rgb12, SubCode};

    const ALPHA_RED: u8 = 0x01;
    const REMAPPING_FOREGROUND_CLUT_2: u8 = 5;
    const CLUT_2_RED: usize = 2 * 8 + 1;
    const TRANSPARENT: usize = 8;
    let words = |green: u8| core::array::from_fn(|entry| Rgb12::new(u8::try_from(entry).unwrap(), green, 0xF));
    let fields = |remapping: u8, green: u8| PresentationFields {
        designations: Some(Designations { default: 0, second: 0x7F }),
        colours: Some(DefaultColourFields { screen: 0, row: 0, black_background_substitution: false, remapping }),
        colour_words: Some(words(green)),
        ..PresentationFields::default()
    };
    let page = |mpp: u16, designation: u8, fields: &PresentationFields| {
        let address = PageAddress::from_mpp(mpp).unwrap();
        let mut text = [with_odd_parity(b' '); 40];
        for (slot, &code) in text.iter_mut().zip([ALPHA_RED, b'R', b'E', b'D'].iter()) {
            *slot = with_odd_parity(code);
        }
        [
            encode::header(address.magazine, address.page, SubCode::new(0), ControlBits(0), &[with_odd_parity(b' '); 32]),
            encode::page_enhancement(address.magazine, designation, PageFormat { function: 0, coding: 0 }, fields),
            encode::row(address.magazine, 1, &text),
        ]
    };
    let packets: Vec<u8> =
        page(0x100, 0, &fields(REMAPPING_FOREGROUND_CLUT_2, 3)).into_iter().chain(page(0x101, 4, &fields(0, 9))).flat_map(|packet| packet.bytes).collect();
    let mut engine = Engine::new();
    engine.feed(&packets);
    engine.finish();
    let row_one = |page: &Rendered| page.indices[usize::from(PAGE_WIDTH) * 20..usize::from(PAGE_WIDTH) * 40].to_vec();
    let scaled = |colour: Rgb12| [colour.red() * 17, colour.green() * 17, colour.blue() * 17];

    let remapped = engine.render(0x100).unwrap();
    assert_eq!(remapped.palette.len(), 32);
    assert_eq!(remapped.palette[CLUT_2_RED], scaled(Rgb12::new(1, 3, 0xF)), "CLUT 2 entry 1 as X/28/0 sent it");
    assert!(row_one(&remapped).contains(&u8::try_from(CLUT_2_RED).unwrap()), "red drawn from CLUT 2");
    assert_eq!(remapped.palette[1], [0xFF, 0, 0], "CLUT 0 is fixed below Level 3.5's X/28/4");

    let redefined = engine.render(0x101).unwrap();
    assert_eq!(redefined.palette[1], scaled(Rgb12::new(1, 9, 0xF)), "CLUT 0 entry 1 as X/28/4 sent it");
    assert!(row_one(&redefined).contains(&1), "red is CLUT 0 entry 1, redefined");
    assert_eq!(redefined.palette[TRANSPARENT], [0, 0, 0], "CLUT 1 entry 0 stays transparent, shown as the picture behind");
}

#[test]
fn mosaics_are_not_taken_for_a_title() {
    // Row 1 is graphics (mosaic green, 0x12): its lower-case letters and
    // punctuation are mosaic blocks, not text. Capitals would still be
    // text there, by the blast-through rule of EN 300 706 Table 26.
    let mut engine = Engine::new();
    engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,\x1bRppp ppp ,,|,,|\nOL,2,Front page\n");
    assert_eq!(engine.finish(), [(0x100, "Front page".to_owned())]);
}

#[test]
fn a_picture_row_with_capitals_is_not_a_title() {
    // Row 1 is mostly mosaics with a few blast-through capitals.
    let mut engine = Engine::new();
    engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,\x1bR,,|,,|E,,|C,,|,,|\nOL,2,Front page\n");
    assert_eq!(engine.finish(), [(0x100, "Front page".to_owned())]);
}

#[test]
fn a_new_version_of_a_page_is_reported_as_updated() {
    let mut engine = Engine::new();
    engine.feed(TTI);
    engine.finish();
    assert_eq!(engine.take_updated(), [] as [u16; 0]);
    // The same page again, unchanged: no update.
    engine.feed(b"PN,10100\nSC,0000\nPS,8000\nOL,1,Sport results\n\n");
    engine.finish();
    assert_eq!(engine.take_updated(), [] as [u16; 0]);
    // A changed version: an update, and not a new arrival.
    let arrived = engine.feed(b"PN,10100\nSC,0000\nPS,8000\nOL,1,Sport results: 2-1\n\n");
    assert!(arrived.is_empty() && engine.finish().is_empty());
    assert_eq!(engine.take_updated(), [0x101]);
}

/// The audio packets of a 2 000-sample PCM sound at 8 kHz that plays on
/// `page`, channel 1, written by the engine's encoder.
fn sound_stream(page: u16) -> Vec<u8> {
    use neoteletext_audio::encode::{Command, Sound, packets};
    let pcm: Vec<i16> = (0..2000).map(|n| i16::try_from(n % 64).unwrap().saturating_mul(100)).collect();
    let sound = Sound::new(5, 1, neoteletext_audio::Codec::Pcm16, 8_000, 1000, &pcm).unwrap();
    let mut records = vec![sound.object_record().unwrap()];
    records.extend(sound.data_records());
    let play = Command { channel: 1, sequence: 1, operation: 1, handle: 5, generation: 1, gain: 70, fade_ms: 20, flags: 0, duck: 0, page: Some(page) };
    records.push(play.record());
    packets(0, 0, &records).0.iter().flat_map(|packet| packet.bytes).collect()
}

#[test]
fn a_page_sound_is_sent_when_its_page_is_opened() {
    let mut engine = Engine::new();
    assert_eq!(engine.feed(&sound_stream(0x100)), [] as [(u16, String); 0], "audio packets are not pages");
    engine.finish();
    assert_eq!(engine.take_sounds(), [] as [Audio; 0], "nothing before the page is open");
    engine.open_page(Some(0x100));
    let sounds = engine.take_sounds();
    let sound = Sound { handle: 5, generation: 1 };
    assert_eq!(sounds.first(), Some(&Audio::Define { sound, rate_hz: 8_000, samples: 2000 }));
    let samples: usize = sounds.iter().map(|audio| if let Audio::Samples { pcm, .. } = audio { pcm.len() } else { 0 }).sum();
    assert_eq!(samples, 2000);
    assert!(matches!(sounds.last(), Some(Audio::Start { channel: 1, gain: 70, fade_ms: 20, .. })));
    // Leaving the page fades it out.
    engine.open_page(Some(0x101));
    assert_eq!(engine.take_sounds(), [Audio::Stop { channel: 1, fade_ms: 5 }]);
}

#[test]
fn a_live_sound_streams_and_fades_two_seconds_after_its_last_block() {
    use neoteletext_audio::encode::{Command, Sound as Writer, packets};
    let live = Writer::live(6, 1, neoteletext_audio::Codec::Pcm16, 8_000, 100).unwrap();
    let play = Command { channel: 0, sequence: 1, operation: 1, handle: 6, generation: 1, gain: 50, fade_ms: 0, flags: 0, duck: 0, page: None };
    let mut records = vec![live.object_record().unwrap(), play.record()];
    for index in 0..4 {
        records.extend(live.live_records(index, &[100; 50]));
    }
    let bytes: Vec<u8> = packets(0, 0, &records).0.iter().flat_map(|packet| packet.bytes).collect();
    let mut engine = Engine::new();
    engine.tick_audio(1_000);
    engine.feed(&bytes);
    engine.finish();
    let sounds = engine.take_sounds();
    let sound = Sound { handle: 6, generation: 1 };
    assert_eq!(sounds.first(), Some(&Audio::Stream { sound, rate_hz: 8_000 }), "{sounds:?}");
    assert!(sounds.iter().any(|audio| matches!(audio, Audio::Start { channel: 0, .. })));
    assert!(sounds.iter().any(|audio| matches!(audio, Audio::StreamSamples { pcm, .. } if pcm.len() == 50)));
    engine.tick_audio(2_999);
    assert_eq!(engine.take_sounds(), [] as [Audio; 0]);
    engine.tick_audio(3_000);
    assert_eq!(engine.take_sounds(), [Audio::Stop { channel: 0, fade_ms: neoteletext_audio::LIVE_STALL_FADE_MS }, Audio::Forget(sound)]);
}

/// Decision 42: in NeoTeletext only links are underlined, so the page's
/// own underline (a Level 2.5 Display Attribute) is not drawn.
#[test]
fn the_pages_own_underline_is_not_drawn() {
    use neoteletext_core::packet::DisplayAttributes;
    use neoteletext_formats::author::DisplayPage;
    let mut page = DisplayPage::new(PageAddress::from_mpp(0x100).unwrap(), SubCode::new(0));
    page.text(3, 4, b"UNDERLINE").unwrap();
    page.local().position(3, 0).unwrap().attributes(4, DisplayAttributes { underline: true, ..DisplayAttributes::default() });
    let mut bytes = Vec::new();
    neoteletext_formats::write::write_tti(&[&page.build().unwrap()], &mut bytes).unwrap();
    let mut engine = Engine::new();
    engine.feed(&bytes);
    engine.finish();
    let rendered = engine.render(0x100).unwrap();
    let (grid, _) = engine.grid(0x100).unwrap();
    assert!(grid.cell(3, 4).is_some_and(|cell| cell.attributes.underline()), "the page does underline the word");
    // The last line of row 3 under the word, which an underline fills.
    let width = usize::from(PAGE_WIDTH);
    let cell = usize::from(interface_core::canvas::CELL_WIDTH);
    let height = usize::from(interface_core::canvas::CELL_HEIGHT);
    let line = 3 * height + height - 1;
    let under = &rendered.indices[line * width + 4 * cell..line * width + 13 * cell];
    // Columns 0 to 3 of the row are not underlined: their last line is
    // the row's background.
    let background = rendered.indices[line * width];
    assert!(under.iter().all(|&index| index == background), "no line under the text");
}

/// `hyperlinks.md` section 6: the page numbers in a page's text and its
/// `LT` and `LK` lines are found, for the trusted core to decide.
#[cfg(feature = "links")]
#[test]
fn a_pages_links_are_found_in_its_text_and_its_link_lines() {
    use protocol::links::{Candidate, Found, NumberLink};
    let page = b"PN,10000\nSC,0000\nPS,8000\nOL,1,News index\nOL,3,Sport 101\nOL,5,Weather here\n\
LT,5,0,7,web:https://www.example.invalid/\nLK,101,https://sport.invalid/\n";
    let mut engine = Engine::new();
    engine.feed(page);
    engine.finish();
    let links = engine.render(0x100).unwrap().links;
    let at = |candidate: &Candidate| (candidate.row, candidate.column, candidate.length, candidate.target.clone());
    assert_eq!(
        links.candidates.iter().map(at).collect::<Vec<_>>(),
        [(5, 0, 7, Found::Declared("web:https://www.example.invalid/".into())), (3, 6, 3, Found::Number(0x101))]
    );
    assert_eq!(links.numbers, [NumberLink { number: 0x101, target: "https://sport.invalid/".into() }]);
    // A run over blank cells is trimmed to the text it covers.
    let mut engine = Engine::new();
    engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,5,   Weather\nLT,5,0,20,302\nLT,7,0,10,303\n");
    engine.finish();
    let links = engine.render(0x100).unwrap().links;
    assert_eq!(links.candidates.iter().map(at).collect::<Vec<_>>(), [(5, 3, 7, Found::Declared("302".into()))], "row 7 is blank");
}

/// `interaction.md` section 10: a pushed page shows when its message
/// arrives, not when the next page of its magazine begins; and a channel
/// may push packets after fetching page files.
#[test]
fn a_pushed_page_shows_as_it_arrives() {
    let text = |engine: &Engine| -> String {
        let (grid, _) = engine.grid(0x100).unwrap();
        (0..11)
            .map(|column| match grid.cell(1, column).unwrap().glyph {
                Glyph::Char(character) => to_unicode(character).base,
                _ => ' ',
            })
            .collect()
    };
    let mut engine = Engine::new();
    engine.feed(TTI);
    engine.finish();
    assert_eq!(engine.feed_whole(false, true, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Version one\n"), [] as [(u16, String); 0], "an update, not an arrival");
    assert_eq!(text(&engine), "Version one");
    assert_eq!(engine.take_updated(), [0x100]);
    engine.feed_whole(false, true, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Version two\n");
    assert_eq!(text(&engine), "Version two");
    // The same page pushed as packets: the TTI page encoded by the engine.
    let mut packets = Vec::new();
    let mut reader = AutoReader::new(AutoConfig { format: Some(Format::Tti), encode_pages: true, ..AutoConfig::default() });
    let mut take = |item: Item<'_>| {
        if let Item::Packet { packet, .. } = item {
            packets.extend_from_slice(&packet.bytes);
        }
    };
    reader.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,Packet push\n", &mut take);
    reader.finish(&mut take);
    engine.feed_whole(true, true, &packets);
    assert_eq!(text(&engine), "Packet push");
    // A fetched copy of a pushed page, older, does not replace it,
    // whether it comes whole or in pieces.
    engine.feed_whole(false, false, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Fetched old\n");
    assert_eq!(text(&engine), "Packet push");
    engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,Fetched old\n");
    engine.finish();
    assert_eq!(text(&engine), "Packet push");
    // A fetched file's last page arrives with the file, though no
    // other page of its magazine follows.
    let arrived = engine.feed_whole(false, false, b"PN,30000\nSC,0000\nPS,8000\nOL,1,Third magazine\n");
    assert_eq!(arrived, [(0x300, "Third magazine".to_owned())]);
}

/// REQ-INT-110, REQ-INT-111 and REQ-INT-114: a page's `CU` and `IM`
/// lines, and a pushed cursor or control record, reach the trusted
/// core as bodies; nothing else does.
#[cfg(feature = "interaction")]
#[test]
fn the_channels_input_records_are_passed_on() {
    use neoteletext_input::control::{Control, packet_of, read_body};
    use neoteletext_input::modes::MOUSE_BUTTONS;
    let mut engine = Engine::new();
    engine.feed_whole(false, true, b"PN,50000\r\nOL,1,Links\r\nCU,255,0,0,0,7\r\nIM,8\r\n");
    let input = engine.take_input();
    assert_eq!(input.cursors, [[0x05, 0x00, 0, 7, 255, 0, 0, 0]]);
    assert_eq!(input.controls.iter().map(|body| read_body(body).unwrap()).collect::<Vec<_>>(), [Control::PageModes { page: 0x500, modes: MOUSE_BUTTONS }]);
    let welcome = packet_of(&Control::Welcome { version: 2, caps: MOUSE_BUTTONS }, 0, 1).unwrap();
    engine.feed_whole(true, true, &welcome.bytes);
    let input = engine.take_input();
    assert_eq!(input.controls.iter().map(|body| read_body(body).unwrap()).collect::<Vec<_>>(), [Control::Welcome { version: 2, caps: MOUSE_BUTTONS }]);
    assert_eq!(input.cursors, [] as [[u8; neoteletext_input::cursor::BODY]; 0]);
    assert_eq!(engine.take_input(), InputRecords::default(), "once each");
    engine.feed_whole(false, true, b"PN,50100\r\nOL,1,None\r\n");
    assert_eq!(engine.take_input(), InputRecords::default(), "a page without the lines");
}

#[test]
fn a_message_reports_only_the_pages_it_touched() {
    let mut engine = Engine::new();
    engine.feed_whole(false, false, TTI);
    assert_eq!(engine.take_changed().pages, BTreeSet::from([0x100, 0x101]));
    engine.feed_whole(false, false, b"PN,10100\nSC,0000\nPS,8000\nOL,1,Sport results: 2-1\n\n");
    let changed = engine.take_changed();
    assert_eq!(changed.pages, BTreeSet::from([0x101]), "only the page in the message");
    assert!(!changed.everything);
    assert_eq!(engine.take_changed(), Changed::default(), "once each");
}

#[test]
fn arrivals_compare_only_the_pages_that_completed() {
    // A store of many pages: one page more must not compare them all,
    // as scanning the store on every message did (1.5 ms a message at
    // 4,096 pages, release build, 21 October 2026).
    const PAGES_STORED: u16 = 200;
    let mut engine = Engine::new();
    let numbers: Vec<u16> = (0..PAGES_STORED).map(|offset| 0x100 + offset / 10 * 0x10 + offset % 10).collect();
    engine.feed_whole(false, false, &sample::tti(&numbers, &[]));
    let before = engine.digests;
    engine.feed_whole(false, false, b"PN,10100\nSC,0000\nPS,8000\nOL,1,One page more\n\n");
    assert_eq!(engine.digests - before, 1, "only the page that completed");
    assert_eq!(engine.take_updated(), [0x101]);
}

#[test]
fn only_a_shown_page_that_flashes_wakes_the_decoder_and_only_when_its_phase_changes() {
    let mut engine = Engine::new();
    engine.feed_whole(false, false, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Steady\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,\x1b\x48Flashing\n\n");
    let mut flashing = Flashing::default();
    // Unit 0 of a cycle: slow flash on (decision E.48).
    let start = 3 * FLASH_CYCLE_US;
    flashing.drawn(0x100, &engine.render_at(0x100, start).unwrap());
    assert_eq!(flashing.next_us(start), None, "a steady page never wakes it");
    flashing.drawn(0x101, &engine.render_at(0x101, start).unwrap());
    let unit = FLASH_CYCLE_US.div_ceil(u64::from(FLASH_UNITS));
    // Slow flash, the only rate the page uses, stays on until unit 9:
    // the fast rates change phase before then, but change nothing on
    // this page, so the decoder sleeps until unit 9. Before this it
    // woke at every unit and drew the page at seven of the twelve.
    assert_eq!(flashing.next_us(start), Some(start + (9 * FLASH_CYCLE_US).div_ceil(u64::from(FLASH_UNITS))), "unit 9, where the slow flash turns off");
    assert_eq!(flashing.due(start + unit), [] as [u16; 0]);
    assert_eq!(flashing.due(start + 2 * unit + 1), [] as [u16; 0], "the fast rates' change at unit 2 is not this page's");
    let off = start + 9 * FLASH_CYCLE_US / u64::from(FLASH_UNITS) + 1;
    assert_eq!(flashing.due(off), [0x101]);
    flashing.retain(&[0x100]);
    assert_eq!(flashing.next_us(off), None, "no longer shown");
}

#[test]
fn a_store_that_fills_is_reported_once() {
    const ROOM: usize = 4;
    let mut engine = Engine::with_store(ROOM);
    engine.feed_whole(false, false, &sample::tti(&[0x100, 0x101, 0x102], &[]));
    engine.take_changed();
    assert!(!engine.take_store_full(), "room for one more");
    engine.feed_whole(false, false, &sample::tti(&[0x103, 0x104], &[]));
    engine.take_changed();
    assert!(engine.take_store_full(), "as many pages as the store has room for");
    assert!(!engine.take_store_full(), "once");
}

#[test]
fn arrivals_are_reported_once() {
    let mut engine = Engine::new();
    engine.feed(TTI);
    engine.finish();
    assert_eq!(engine.feed(b""), [] as [(u16, String); 0]);
    assert_eq!(engine.finish(), [] as [(u16, String); 0]);
}
