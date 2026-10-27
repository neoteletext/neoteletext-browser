//! The link messages' tests: every message round trips on its three
//! pipes, and what is malformed, too long or not cleaned is refused.

use interface_core::canvas::PageKey;

use super::*;
use crate::ProtocolError;
use crate::decoder::{FromDecoder, read_from_decoder, write_to_core as decoder_writes};
use crate::fetcher::{FromFetcher, read_from_fetcher, write_to_core as fetcher_writes};
use crate::{ToCore, ToInterface, read_from_core, read_from_interface, write_to_core, write_to_interface};

const KEY: PageKey = PageKey { channel: 7, page: 0x123 };

fn run(row: u8, kind: LinkKind) -> LinkRun {
    LinkRun { row, column: 4, length: 3, double: row.is_multiple_of(2), light: row.is_multiple_of(3), kind }
}

fn page_links() -> PageLinks {
    PageLinks {
        version: 9,
        runs: vec![run(3, LinkKind::Page), run(4, LinkKind::Channel), run(6, LinkKind::Web)],
        controls: vec![Area { row: 10, column: 2, width: 9, height: 2 }],
        pointer_areas: Vec::new(),
        remapped: vec![0x302, 0x500],
    }
}

#[test]
fn every_link_message_round_trips() {
    for message in [
        Message::Page { key: KEY, links: page_links() },
        Message::Pointing(Some(Pointing { kind: LinkKind::Web, text: "https://www.example.invalid/ (opens in your web browser)".into() })),
        Message::Pointing(None),
        Message::Prompt(Some(Prompt { id: 3, kind: PromptKind::Leave, destination: "weather.invalid".into(), secure: true })),
        Message::Prompt(None),
        Message::Show(KEY),
        Message::Aliases { channel: 7, numbers: vec![0x500] },
    ] {
        let message = ToInterface::Link(message);
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &message).unwrap();
        assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
    let spot = Spot { key: KEY, version: 9, column: 39, row: 24 };
    for message in [
        ToCore::Point(Some(spot)),
        ToCore::Point(None),
        ToCore::Typed { number: 0x302, opened: Some((KEY, 9)) },
        ToCore::Typed { number: 0x500, opened: None },
        ToCore::Focused(false),
        ToCore::Confirm(3),
        ToCore::Cancel(3),
        ToCore::Lit(Some(Lit { key: KEY, area: Area { row: 20, column: 11, width: 9, height: 1 }, pressed: true })),
        ToCore::Lit(None),
    ] {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &message).unwrap();
        assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
    let found = FromDecoder::Links {
        page: 0x100,
        with_image: true,
        candidates: vec![
            Candidate { row: 5, column: 10, length: 3, double: true, light: false, target: "302".into() },
            Candidate { row: 7, column: 0, length: 12, double: false, light: true, target: "web:https://www.example.invalid/".into() },
        ],
        numbers: vec![NumberLink { number: 0x302, target: "sport:302".into() }],
        controls: vec![Area { row: 20, column: 1, width: 9, height: 1 }, Area { row: 5, column: 2, width: 30, height: 3 }],
        pointer_areas: Vec::new(),
        fastext: Fastext::default(),
    };
    let mut bytes = Vec::new();
    decoder_writes(&mut bytes, &found).unwrap();
    assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), found);
    let channel = FromFetcher::Links {
        aliases: vec![Alias { page: 0x500, target: "https://weather.invalid/".into() }],
        services: vec![Service { name: "sport".into(), address: "https://sport.invalid/".into() }],
    };
    let mut bytes = Vec::new();
    fetcher_writes(&mut bytes, &channel).unwrap();
    assert_eq!(read_from_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), channel);
}

/// The payload of a page's links with one run of these bytes.
fn one_run(place: [u8; 3], flags: u8, kind: u8) -> Vec<u8> {
    let mut payload = vec![0, 0, 0, 7, 0x01, 0x23, 0, 0, 0, 9, 1];
    payload.extend_from_slice(&place);
    // No controls, no pointer areas, no page numbers remapped.
    payload.extend_from_slice(&[flags, kind, 0, 0, 0]);
    let mut bytes = vec![KIND_PAGE_LINKS];
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

#[test]
fn malformed_links_are_refused() {
    let read = |bytes: Vec<u8>| read_from_core(&mut bytes.as_slice());
    assert!(read(one_run([3, 4, 3], 0, 1)).is_ok());
    for (place, flags, kind, why) in [
        ([25, 4, 3], 0, 1, "link outside the page"),
        ([3, 38, 3], 0, 1, "link outside the page"),
        ([3, 4, 0], 0, 1, "link outside the page"),
        ([3, 4, 3], 4, 1, "link flags"),
        ([3, 4, 3], 0, 0, "link kind"),
        ([3, 4, 3], 0, 4, "link kind"),
    ] {
        assert!(matches!(read(one_run(place, flags, kind)), Err(ProtocolError::BadValue(reason)) if reason == why), "{why}");
    }
    // More runs than a page may have.
    let mut payload = vec![0, 0, 0, 7, 0x01, 0x23, 0, 0, 0, 9, u8::try_from(MAX_RUNS + 1).unwrap()];
    payload.extend(std::iter::repeat_n([3, 4, 3, 0, 1], MAX_RUNS + 1).flatten());
    payload.push(0);
    let mut bytes = vec![KIND_PAGE_LINKS];
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&payload);
    assert!(matches!(read(bytes), Err(ProtocolError::BadValue("too many links"))));
    // A remapped number that is not a display page.
    let links = PageLinks { remapped: vec![0x1A0], ..page_links() };
    let mut bytes = Vec::new();
    assert!(write_to_interface(&mut bytes, &ToInterface::Link(Message::Page { key: KEY, links })).is_err());
}

#[test]
fn a_target_too_long_is_refused_never_cut() {
    // Cut short, an address can name another place.
    let target = format!("web:https://example.invalid/{}", "a".repeat(MAX_TARGET));
    let found = FromDecoder::Links {
        page: 0x100,
        with_image: false,
        candidates: vec![Candidate { row: 5, column: 0, length: 3, double: false, light: false, target }],
        numbers: Vec::new(),
        controls: Vec::new(),
        pointer_areas: Vec::new(),
        fastext: Fastext::default(),
    };
    assert!(matches!(decoder_writes(&mut Vec::new(), &found), Err(ProtocolError::BadValue("link target too long"))));
    // Written by hand, a reader refuses it too.
    let mut payload = vec![0x01, 0x00, 0, 1, 5, 0, 3, 0];
    payload.extend_from_slice(&u16::try_from(MAX_TARGET + 1).unwrap().to_be_bytes());
    payload.extend(std::iter::repeat_n(b'a', MAX_TARGET + 1));
    payload.push(0);
    let mut bytes = vec![68];
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&payload);
    assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("text too long"))));
}

#[test]
fn the_strips_text_must_be_cleaned_and_its_flags_known() {
    let hostile = Pointing { kind: LinkKind::Page, text: "Page\u{202E}203".into() };
    assert!(write_to_interface(&mut Vec::new(), &ToInterface::Link(Message::Pointing(Some(hostile)))).is_err());
    let mut bytes = vec![KIND_PROMPT, 0, 0, 0, 8, 0, 0, 0, 1, 1, 2, 0, 0];
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("prompt flags"))));
    bytes[9] = 3;
    bytes[10] = 0;
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("prompt kind"))));
}

#[test]
fn a_typed_number_must_be_a_display_page() {
    assert!(write_to_core(&mut Vec::new(), &ToCore::Typed { number: 0x1A0, opened: None }).is_err());
    let bytes = [25, 0, 0, 0, 2, 0x09, 0x00];
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
}

/// INPUT.md section 7.3: an area is inside the page, at least a cell
/// each way, on every pipe that carries one; and a page's controls are
/// bounded by the engine's `MAX_CONTROLS`.
#[test]
fn an_area_outside_the_page_or_too_many_controls_are_refused() {
    use crate::Fields;
    for (bytes, inside) in
        [([24, 0, 40, 1], true), ([1, 31, 9, 2], true), ([1, 32, 9, 1], false), ([24, 0, 1, 2], false), ([5, 0, 0, 1], false), ([5, 0, 1, 0], false)]
    {
        assert_eq!(read_area(&mut Fields::new(&bytes)).is_ok(), inside, "{bytes:?}");
    }
    let found = FromDecoder::Links {
        page: 0x100,
        with_image: false,
        candidates: Vec::new(),
        numbers: Vec::new(),
        controls: vec![Area { row: 1, column: 0, width: 1, height: 1 }; MAX_CONTROLS + 1],
        pointer_areas: Vec::new(),
        fastext: Fastext::default(),
    };
    assert!(matches!(decoder_writes(&mut Vec::new(), &found), Err(ProtocolError::BadValue("too many controls"))));
}

/// INPUT.md section 9.7: the decoder is told the look of one area of a
/// page, or that there is none.
#[test]
fn the_look_round_trips_to_the_decoder() {
    use crate::decoder::{ToDecoder, read_from_core as decoder_reads, write_to_decoder};
    for message in
        [ToDecoder::Look { page: 0x128, look: Some((Area { row: 20, column: 1, width: 9, height: 1 }, false)) }, ToDecoder::Look { page: 0x128, look: None }]
    {
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, &message).unwrap();
        assert_eq!(decoder_reads(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
}
