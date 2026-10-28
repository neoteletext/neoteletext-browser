//! The decoder pipe's tests: every kind round trips, and what a decoder
//! may not send, or sends malformed, is refused.

#[cfg(feature = "interaction")]
use super::kinds::KIND_INPUT_CONTROL;
use super::kinds::{KIND_ARRIVED, KIND_BOOKMARK_FOLDER, KIND_END, KIND_FRAME};
use super::*;
use crate::frame::PAGE_DOTS;
use crate::{MAX_PAGE_TITLE, ProtocolError};
use neoteletext_core::limits::SUBCODE_ANY;

#[test]
fn a_long_title_is_cut_on_a_character_boundary() {
    // Two-byte 'é' straddles the limit: cutting at the byte would leave
    // half a character, which arrives as U+FFFD.
    let title = format!("{}é", "a".repeat(MAX_PAGE_TITLE - 1));
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &FromDecoder::Arrived { page: 0x100, title }).unwrap();
    let arrived = read_from_decoder(&mut bytes.as_slice()).unwrap();
    assert_eq!(arrived, Some(FromDecoder::Arrived { page: 0x100, title: "a".repeat(MAX_PAGE_TITLE - 1) }));
}

#[test]
fn every_kind_round_trips() {
    for message in [
        ToDecoder::Data(vec![1, 2, 3]),
        ToDecoder::Whole { packets: true, pushed: true, bytes: vec![0; 42] },
        ToDecoder::Whole { packets: false, pushed: false, bytes: b"PN,10000\n".to_vec() },
        ToDecoder::End,
        ToDecoder::Showing(vec![0x100, 0x899]),
        ToDecoder::Close,
        ToDecoder::View(View { page: 0x302, reveal: true, hold: false, mix: true, subpage: Some(2) }),
        ToDecoder::View(View { page: 0x100, ..View::default() }),
        ToDecoder::SubPageRows { page: 0x210, range: SubPageRange { first: 8_126, count: 64 } },
    ] {
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, &message).unwrap();
        assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
    let mut indices = vec![0; PAGE_DOTS];
    indices[7] = 1;
    for message in [
        FromDecoder::Arrived { page: 0x100, title: "Headlines".into() },
        FromDecoder::Updated { page: 0x302 },
        FromDecoder::Confined,
        FromDecoder::StoreFull,
        FromDecoder::Frame { page: 0x123, palette: vec![[0, 0, 0], [255, 255, 255]], screen: 1, indices },
        FromDecoder::SubPages { page: 0x302, step: SubPageStep { count: 3, shown: 2, place: Some(1), previous: Some(1), next: Some(3) } },
        FromDecoder::SubPageCount { page: 0x210, count: 8_190 },
        FromDecoder::SubPageCount { page: 0x210, count: 0 },
        FromDecoder::SubPageRows { page: 0x210, rows: SubPageRows { count: 3, first: 1, rows: vec![(2, "Story 2/3".into()), (3, "Comments".into())] } },
        FromDecoder::SubPages { page: 0x100, step: SubPageStep { count: 1, shown: SUBCODE_ANY, place: Some(0), previous: None, next: None } },
        FromDecoder::Bookmark(BookmarkEntry::Folder("Bookmarks bar".into())),
        FromDecoder::Bookmark(BookmarkEntry::FolderEnd),
        FromDecoder::Bookmark(BookmarkEntry::Link { name: "Teefax".into(), address: "https://teastop.co.uk/teefax/".into() }),
        FromDecoder::BookmarksRead { skipped: 3 },
        FromDecoder::ChannelLog { level: interface_core::console::Level::Warning, page: Some(0x1A0 - 0xA0), text: "Weather feed late".into() },
        FromDecoder::ChannelLog { level: interface_core::console::Level::Info, page: None, text: "Started".into() },
    ] {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &message).unwrap();
        assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
}

/// Decision 71: a bookmark's text longer than its bound is refused,
/// and the Netscape file is known by its first line alone.
#[test]
fn a_bookmark_entry_is_held_to_its_bounds() {
    let mut bytes = vec![KIND_BOOKMARK_FOLDER];
    let length = MAX_BOOKMARK_NAME + 1;
    bytes.extend_from_slice(&u32::try_from(length + 2).unwrap().to_be_bytes());
    bytes.extend_from_slice(&u16::try_from(length).unwrap().to_be_bytes());
    bytes.extend(std::iter::repeat_n(b'a', length));
    assert!(read_from_decoder(&mut bytes.as_slice()).is_err());
    let link = FromDecoder::Bookmark(BookmarkEntry::Link { name: "n".into(), address: "h".repeat(crate::MAX_ADDRESS + 9) });
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &link).unwrap();
    let Some(FromDecoder::Bookmark(BookmarkEntry::Link { address, .. })) = read_from_decoder(&mut bytes.as_slice()).unwrap() else { panic!("a link") };
    assert_eq!(address.len(), crate::MAX_ADDRESS, "cut to its bound as it is sent");
    assert!(is_netscape_bookmarks(b"\xef\xbb\xbf \n<!doctype netscape-bookmark-file-1>\n<DL>"));
    assert!(!is_netscape_bookmarks(b"<html><!DOCTYPE NETSCAPE-Bookmark-file-1>"));
    assert!(!is_netscape_bookmarks(b"<!DOCTYPE NETSCAPE"));
}

/// REQ-INT-111: a control record's body goes to the trusted core as
/// sent, within one fragment's body.
#[cfg(feature = "interaction")]
#[test]
fn an_input_control_body_round_trips_within_its_bound() {
    let message = FromDecoder::InputControl { body: vec![0xFF, 0x01, 2, 0] };
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &message).unwrap();
    assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), message);
    let long = FromDecoder::InputControl { body: vec![0; neoteletext_input::control::MAX_BODY + 1] };
    assert!(write_to_core(&mut Vec::new(), &long).is_err());
    let mut bytes = vec![KIND_INPUT_CONTROL];
    bytes.extend_from_slice(&u32::try_from(neoteletext_input::control::MAX_BODY + 1).unwrap().to_be_bytes());
    bytes.extend(std::iter::repeat_n(0, neoteletext_input::control::MAX_BODY + 1));
    assert!(read_from_decoder(&mut bytes.as_slice()).is_err());
}

/// Decision 67: a view names a real sub-page, never "any"; a sub-page
/// list is bounded; Fastext's targets round-trip and their bounds hold.
#[test]
fn page_controls_keep_their_bounds() {
    let any = View { page: 0x100, subpage: Some(SUBCODE_ANY), ..View::default() };
    assert!(write_to_decoder(&mut Vec::new(), &ToDecoder::View(any)).is_err());
    let mut bytes = Vec::new();
    write_to_decoder(&mut bytes, &ToDecoder::View(View { page: 0x100, ..View::default() })).unwrap();
    bytes[5] = 0x10;
    assert!(read_from_core(&mut bytes.as_slice()).is_err(), "an unknown view flag");
    // Decision 112: the standard's 8,190, never more, and rows within the
    // budget a message carries.
    let most = u16::try_from(neoteletext_core::limits::MAX_SUBPAGES).unwrap();
    assert!(write_to_core(&mut Vec::new(), &FromDecoder::SubPageCount { page: 0x100, count: most }).is_ok());
    assert!(write_to_core(&mut Vec::new(), &FromDecoder::SubPageCount { page: 0x100, count: most + 1 }).is_err());
    let many = FromDecoder::SubPages { page: 0x100, step: SubPageStep { count: most + 1, shown: 1, ..SubPageStep::default() } };
    assert!(write_to_core(&mut Vec::new(), &many).is_err());
    let rows = SubPageRows { count: most, first: 0, rows: vec![(1, String::new()); SUBPAGE_ROWS + 1] };
    assert!(write_to_core(&mut Vec::new(), &FromDecoder::SubPageRows { page: 0x100, rows }).is_err());
    #[cfg(feature = "links")]
    {
        use crate::links::{Fastext, FastextTarget};
        let target = |page, subcode, service| Some(FastextTarget { page, subcode, service });
        let fastext = Fastext {
            keys: [target(0x200, None, None), None, target(0x302, Some(4), None), target(0x101, None, Some(1)), target(0x100, None, None)],
            row24: true,
        };
        let message = FromDecoder::Links {
            page: 0x100,
            with_image: true,
            candidates: Vec::new(),
            numbers: Vec::new(),
            controls: Vec::new(),
            pointer_areas: Vec::new(),
            fastext,
        };
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &message).unwrap();
        assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), message);
        let wrong = Fastext { keys: [target(0x0FF, None, None), None, None, None, None], row24: false };
        let wrong = FromDecoder::Links {
            page: 0x100,
            with_image: true,
            candidates: Vec::new(),
            numbers: Vec::new(),
            controls: Vec::new(),
            pointer_areas: Vec::new(),
            fastext: wrong,
        };
        assert!(write_to_core(&mut Vec::new(), &wrong).is_err(), "not a display page");
    }
}

#[test]
fn a_decoder_cannot_send_what_it_may_not() {
    // A decoder sending the trusted core's own kind.
    let mut bytes = Vec::new();
    write_to_decoder(&mut bytes, &ToDecoder::End).unwrap();
    assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_END))));
    // A screen colour outside its palette, then an index.
    let mut payload = vec![0x01, 0x00, 1, 0, 0, 0, 1];
    payload.extend(std::iter::repeat_n(0, PAGE_DOTS));
    let mut bytes = vec![KIND_FRAME];
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&payload);
    assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("screen colour outside the palette"))));
    payload[6] = 0;
    payload[7] = 1;
    let mut bytes = vec![KIND_FRAME];
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&payload);
    assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("index outside the palette"))));
    // A hexadecimal page number.
    let mut bytes = vec![KIND_ARRIVED, 0, 0, 0, 4, 0x1A, 0x00, 0, 0];
    assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
    // A title longer than a row.
    bytes = vec![KIND_ARRIVED, 0, 0, 0, 4, 0x01, 0x00, 0xFF, 0xFF];
    assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("title too long"))));
}

/// Finding F-29: the trusted core never writes a decoder a page it would
/// refuse, nor more pages than a channel has.
#[test]
fn showing_holds_display_pages_and_no_more_than_a_channel_has() {
    let write = |pages: Vec<u16>| write_to_decoder(&mut Vec::new(), &ToDecoder::Showing(pages));
    assert!(write(vec![0x100, 0x899]).is_ok());
    assert!(write(vec![0x100, 0x1A0]).is_err(), "a hexadecimal page");
    assert!(write(vec![0x100; PAGES + 1]).is_err(), "too many");
}
