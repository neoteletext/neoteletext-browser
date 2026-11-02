//! The fetcher pipe's tests: every kind round trips, cookie messages keep
//! their bounds, and malformed contents and exchanges are refused.

use super::*;
use crate::ProtocolError;

/// A cookie with no attributes.
fn plain_cookie() -> SetCookie {
    SetCookie {
        name: b"x".to_vec(),
        value: b"2".to_vec(),
        max_age: None,
        expires: None,
        domain: false,
        path: None,
        secure: false,
        http_only: false,
        partitioned: false,
        same_site: CookieSameSite::Default,
    }
}

/// A cookie with every attribute.
fn full_cookie() -> SetCookie {
    SetCookie {
        max_age: Some(-5),
        expires: Some(1_791_590_400),
        domain: true,
        path: Some(b"/tt/".to_vec()),
        secure: true,
        http_only: true,
        partitioned: true,
        same_site: CookieSameSite::Strict,
        ..plain_cookie()
    }
}

#[test]
fn every_kind_round_trips() {
    for message in [
        FromFetcher::Contents(vec![Entry::Section("News".into()), Entry::Page { page: 0x101, title: "Headlines".into() }]),
        FromFetcher::Page { origin: Origin::PushedPackets, bytes: vec![0; 42] },
        FromFetcher::Page { origin: Origin::Fetched, bytes: vec![1, 2, 3] },
        FromFetcher::Page { origin: Origin::FetchedPage, bytes: b"PN,10000\\n".to_vec() },
        FromFetcher::Page { origin: Origin::FetchedPackets, bytes: vec![0; 84] },
        FromFetcher::Notice("Opened".into()),
        FromFetcher::Denied { status: 404, place: "302/0002".into() },
        FromFetcher::Denied { status: 503, place: String::new() },
        FromFetcher::Redirected("https://b.example/teletext/".into()),
        FromFetcher::PageRedirect { page: 0x302, sub: 2, query: None },
        FromFetcher::PageRedirect { page: 0x302, sub: 2, query: Some("a=1".into()) },
        FromFetcher::PageRedirect { page: 0x302, sub: neoteletext_core::limits::SUBCODE_ANY, query: None },
        FromFetcher::ProgramRedirect("https://b.example/teletext/302".into()),
        FromFetcher::Pushing(true),
        FromFetcher::Exchange { method: "GET".into(), url: "https://a.example/100.tti".into(), status: 200, bytes: 512, elapsed_us: 1_234 },
        FromFetcher::Confined,
        FromFetcher::ProxyFailed,
        FromFetcher::CertificateRefused(CertificateProblem::Expired),
        FromFetcher::CertificateRefused(CertificateProblem::Handshake),
        FromFetcher::CookieQuestion { id: 7, host: "a.example".into(), path: "/tt/100.tti".into(), secure: true, site: CookieSite::SameService },
        FromFetcher::CookieQuestion { id: 8, host: "b.example".into(), path: "/".into(), secure: false, site: CookieSite::CrossServiceNavigation },
        FromFetcher::ReachedQuestion { id: 9, host: "a.example".into() },
        FromFetcher::CookieSet { host: "a.example".into(), path: "/tt/".into(), scope: Some("/tt/".into()), secure: true, cookie: full_cookie() },
        FromFetcher::CookieSet { host: "a.example".into(), path: "/p".into(), scope: None, secure: false, cookie: plain_cookie() },
        FromFetcher::ClearSiteData("a.example".into()),
        FromFetcher::SubPages {
            page: 0x210,
            version: "4be81f7a09c2d635".into(),
            part: SubPageRows { count: 8_190, first: 8_128, rows: vec![(0x0001, String::new()), (0x3F7E, "The last".into())] },
        },
        FromFetcher::SubPagesMissing { page: 0x210, part: 0 },
        FromFetcher::SubPagesMissing { page: 0x210, part: 127 },
        FromFetcher::SubPagesFailed { page: 0x210, part: 3 },
        FromFetcher::NoContents,
        FromFetcher::Refresh(5),
        FromFetcher::Refresh(86_400),
        FromFetcher::Refused { file: RefusedFile::Channel, status: Some(500) },
        FromFetcher::Refused { file: RefusedFile::Bundle, status: None },
        FromFetcher::Refused { file: RefusedFile::Contents, status: Some(403) },
        FromFetcher::Refused { file: RefusedFile::Page(0x210), status: Some(200) },
        #[cfg(feature = "interaction")]
        FromFetcher::RoundTrip { microseconds: 190 },
    ] {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &message).unwrap();
        assert_eq!(read_from_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
    for message in [
        ToFetcher::CookieAnswer { id: 7, header: b"id=1".to_vec() },
        ToFetcher::CookieAnswer { id: 8, header: Vec::new() },
        ToFetcher::ReachedAnswer { id: 9, reached: true },
        ToFetcher::FetchPage { page: 0x210 },
        ToFetcher::FetchSubPages { page: 0x210, part: 0 },
        ToFetcher::FetchSubPages { page: 0x210, part: 127 },
        ToFetcher::FetchSubPage { page: 0x210, subcode: 0x3F7E },
        #[cfg(feature = "interaction")]
        ToFetcher::Send { bytes: b"a\xFF\x01\x80\x04\x00\x04\x02".to_vec(), move_of: None, more: true },
        #[cfg(feature = "interaction")]
        ToFetcher::Send { bytes: vec![0xFF, 0x02, 0x80, 0x04, 0x00, 0x06, 0x00, 0x0A, 0x05, 0x79, 0x64], move_of: Some(0), more: false },
        #[cfg(feature = "interaction")]
        ToFetcher::Send { bytes: vec![0xFF, 0x44, 0x0A, 0x80, 0x04, 0x00, 0x06, 0x01, 0x0B, 0x05, 0x85, 0x01, 0x64], move_of: Some(1), more: false },
        #[cfg(feature = "interaction")]
        ToFetcher::MeasureRoundTrip(true),
        #[cfg(feature = "interaction")]
        ToFetcher::MeasureRoundTrip(false),
    ] {
        let mut bytes = Vec::new();
        write_to_fetcher(&mut bytes, &message).unwrap();
        assert_eq!(read_to_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
}

// REQ-INT-140: a page the descriptor opens at is a page, like any other
// number a fetcher sends.
#[test]
fn an_initial_page_is_a_page() {
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &FromFetcher::InitialPage(0x302)).unwrap();
    assert_eq!(read_from_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), FromFetcher::InitialPage(0x302));
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &FromFetcher::InitialPage(0x0FF)).unwrap();
    assert!(read_from_fetcher(&mut bytes.as_slice()).is_err());
}

/// A hostile fetcher may write any byte: a region is four bits, and one.
#[test]
fn a_region_is_four_bits() {
    let read = |payload: &[u8]| {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &FromFetcher::Region(None)).unwrap();
        let kind_and_length = bytes.len();
        bytes[kind_and_length - 4..].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(payload);
        read_from_fetcher(&mut bytes.as_slice())
    };
    assert_eq!(read(&[]).unwrap().unwrap(), FromFetcher::Region(None));
    assert_eq!(read(&[6]).unwrap().unwrap(), FromFetcher::Region(Some(6)));
    assert!(read(&[16]).is_err());
    assert!(read(&[6, 6]).is_err());
}

/// Decision 62: cookie messages within their bounds only.
#[test]
fn cookie_messages_are_bounded() {
    let set = |cookie: SetCookie| FromFetcher::CookieSet { host: "a.example".into(), path: "/".into(), scope: None, secure: true, cookie };
    assert!(write_to_core(&mut Vec::new(), &set(SetCookie { value: vec![b'x'; MAX_COOKIE_NAME_VALUE], ..plain_cookie() })).is_err());
    assert!(write_to_core(&mut Vec::new(), &set(SetCookie { path: Some(vec![b'/'; MAX_COOKIE_ATTRIBUTE + 1]), ..plain_cookie() })).is_err());
    // What a hostile fetcher could write itself: the lengths of a name and
    // value over the bound, a path over it, a `SameSite` that is none, and
    // bytes after the cookie.
    let message = |lengths: [u16; 3], same_site: u8, more: &[u8]| {
        let mut payload = vec![0, 1, b'a', 0, 1, b'/', 0, same_site];
        payload.extend(lengths.iter().flat_map(|length| length.to_be_bytes()));
        payload.extend_from_slice(more);
        let mut bytes = vec![KIND_COOKIE_SET];
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend(payload);
        bytes
    };
    let refused =
        |bytes: Vec<u8>, reason: &str| assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::BadValue(said)) if said == reason));
    let long = u16::try_from(MAX_COOKIE_NAME_VALUE + 1).unwrap();
    refused(message([long, 0, u16::MAX], 0, &[]), "cookie too long");
    refused(message([1, 0, u16::try_from(MAX_COOKIE_ATTRIBUTE + 1).unwrap()], 0, b"a"), "cookie path too long");
    refused(message([1, 0, u16::MAX], 4, b"a"), "cookie SameSite");
    refused(message([1, 0, u16::MAX], 0, b"ab"), "cookie has more");
    let bad_site = [KIND_COOKIE_QUESTION, 0, 0, 0, 12, 0, 0, 0, 1, 0, 1, b'a', 0, 1, b'/', 0, 3];
    assert!(matches!(read_from_fetcher(&mut bad_site.as_slice()), Err(ProtocolError::BadValue("site"))));
    let answer = ToFetcher::CookieAnswer { id: 1, header: vec![b'x'; MAX_COOKIE_HEADER + 1] };
    assert!(write_to_fetcher(&mut Vec::new(), &answer).is_err());
    // A fetcher may not send the trusted core's answers.
    let mut bytes = Vec::new();
    write_to_fetcher(&mut bytes, &ToFetcher::ReachedAnswer { id: 1, reached: false }).unwrap();
    assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_REACHED_ANSWER))));
}

#[test]
fn malformed_contents_are_refused() {
    // A hexadecimal page number.
    let bytes = [KIND_CONTENTS, 0, 0, 0, 7, 0, 1, ENTRY_PAGE, 0x1A, 0x00, 0, 0];
    assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
    // An unknown entry kind.
    let bytes = [KIND_CONTENTS, 0, 0, 0, 3, 0, 1, 9];
    assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::BadValue("contents entry"))));
    // An exchange's status out of range, and a method that is not one.
    let exchange = |method: &[u8], status: u16| {
        let mut payload = vec![0, u8::try_from(method.len()).unwrap()];
        payload.extend_from_slice(method);
        payload.extend_from_slice(&[0, 1, b'/']);
        payload.extend_from_slice(&status.to_be_bytes());
        payload.extend_from_slice(&[0; 16]);
        let mut bytes = vec![KIND_EXCHANGE, 0, 0, 0, u8::try_from(payload.len()).unwrap()];
        bytes.extend(payload);
        read_from_fetcher(&mut bytes.as_slice())
    };
    assert!(exchange(b"GET", 200).is_ok());
    assert!(matches!(exchange(b"GET", 99), Err(ProtocolError::BadValue("status"))));
    assert!(matches!(exchange(b"GET", 600), Err(ProtocolError::BadValue("status"))));
    assert!(matches!(exchange(b"get\n", 200), Err(ProtocolError::BadValue("method"))));
    assert!(matches!(exchange(b"", 200), Err(ProtocolError::BadValue("method"))));
    assert!(matches!(exchange(b"VERYLONGMETHOD", 200), Err(ProtocolError::BadValue("text too long"))));
    let bytes = [KIND_PROXY_FAILED, 0, 0, 0, 1, 0];
    assert!(read_from_fetcher(&mut bytes.as_slice()).is_err(), "nothing after it");
    // A fetcher sending a decoder's kind.
    let bytes = [65, 0, 0, 0, 0];
    assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(65))));
}

/// REQ-1888, DOC-1869: a part of a list is at most [`ROWS_PER_PART`] rows
/// from a part's first place, of a version of at most 16 characters, a
/// part's number below [`MAX_LIST_PARTS`], within the standard's 8,190
/// sub-pages a page; a sub-page fetched is one sub-page, never 3F7F, "any".
#[test]
fn lists_and_sub_pages_keep_their_bounds() {
    let title = "T".repeat(40);
    let version = "v".repeat(MAX_LIST_VERSION);
    let full = SubPageRows { count: 8_190, first: 0, rows: vec![(0x0101, title); ROWS_PER_PART] };
    let part = |part: SubPageRows| FromFetcher::SubPages { page: 0x210, version: version.clone(), part };
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &part(full.clone())).unwrap();
    assert!(bytes.len() <= MAX_PAYLOAD, "a whole part with every title at its longest is one message");
    let over = SubPageRows { rows: vec![(0x0101, String::new()); ROWS_PER_PART + 1], ..full.clone() };
    assert!(write_to_core(&mut Vec::new(), &part(over)).is_err(), "a 65th row");
    let beyond = SubPageRows { count: 8_191, ..full.clone() };
    assert!(write_to_core(&mut Vec::new(), &part(beyond)).is_err());
    let misplaced = SubPageRows { first: 1, rows: vec![(0x0101, String::new())], ..full.clone() };
    assert!(write_to_core(&mut Vec::new(), &part(misplaced)).is_err(), "not a part's first place");
    let long = FromFetcher::SubPages { page: 0x210, version: "v".repeat(MAX_LIST_VERSION + 1), part: full };
    assert!(write_to_core(&mut Vec::new(), &long).is_err(), "a version of 17 characters");
    // Page 210, version "v", 8,190 sub-pages, the first row at `place`, one
    // row: sub-code 0101 with no title.
    let forged = |place: u8| [KIND_SUBPAGES, 0, 0, 0, 15, 0x02, 0x10, 0, 1, b'v', 0x1F, 0xFE, 0, place, 0, 1, 0x01, 0x01, 0, 0];
    assert!(read_from_fetcher(&mut forged(64).as_slice()).is_ok(), "a part at place 64");
    assert!(read_from_fetcher(&mut forged(1).as_slice()).is_err(), "a forged part at place 1");
    assert!(write_to_fetcher(&mut Vec::new(), &ToFetcher::FetchSubPages { page: 0x210, part: 128 }).is_err(), "past the last part");
    assert!(write_to_core(&mut Vec::new(), &FromFetcher::SubPagesMissing { page: 0x210, part: 128 }).is_err());
    assert!(write_to_fetcher(&mut Vec::new(), &ToFetcher::FetchSubPage { page: 0x210, subcode: 0x3F7F }).is_err(), "3F7F names no sub-page");
    assert!(write_to_fetcher(&mut Vec::new(), &ToFetcher::FetchSubPage { page: 0x210, subcode: 0x0080 }).is_err(), "not a sub-code");
    let forged = [KIND_FETCH_SUBPAGE, 0, 0, 0, 4, 0x02, 0x10, 0x3F, 0x7F];
    assert!(read_to_fetcher(&mut forged.as_slice()).is_err());
    // Neither side takes the other's.
    let mut bytes = Vec::new();
    write_to_fetcher(&mut bytes, &ToFetcher::FetchSubPages { page: 0x210, part: 0 }).unwrap();
    assert!(read_from_fetcher(&mut bytes.as_slice()).is_err());
}

/// REQ-RCH-035: a message the trusted core marks as a contact's move is
/// that contact's move alone, on both sides of the pipe, so a fetcher
/// holds nothing else in a contact's slot; `more` is a flag.
#[cfg(feature = "interaction")]
#[test]
fn a_move_is_marked_only_as_its_own_contacts() {
    // DOC-1963: a move to the dot (121, 100); a key.
    let mouse_move = vec![0xFF, 0x02, 0x80, 0x04, 0x00, 0x06, 0x00, 0x0A, 0x05, 0x79, 0x64];
    let second_finger_move = vec![0xFF, 0x44, 0x0A, 0x80, 0x04, 0x00, 0x06, 0x01, 0x0B, 0x05, 0x85, 0x01, 0x64];
    let enter = vec![0xFF, 0x01, 0x80, 0x04, 0x00, 0x04, 0x02];
    let mut two_moves = mouse_move.clone();
    two_moves.extend_from_slice(&mouse_move);
    for (bytes, move_of) in [
        (mouse_move.clone(), Some(1)),
        (second_finger_move, Some(0)),
        (enter, Some(0)),
        (two_moves, Some(0)),
        (mouse_move.clone(), Some(u8::try_from(neoteletext_input::MAX_CONTACTS).unwrap())),
    ] {
        let message = ToFetcher::Send { bytes: bytes.clone(), move_of, more: false };
        assert!(write_to_fetcher(&mut Vec::new(), &message).is_err(), "{bytes:?} as {move_of:?}");
        let mut payload = vec![move_of.unwrap(), 0];
        payload.extend_from_slice(&bytes);
        let mut framed = vec![KIND_SEND];
        framed.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        framed.extend_from_slice(&payload);
        assert!(read_to_fetcher(&mut framed.as_slice()).is_err(), "{bytes:?} as {move_of:?}");
    }
    let mut payload = vec![0, 2];
    payload.extend_from_slice(&mouse_move);
    let mut framed = vec![KIND_SEND];
    framed.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    framed.extend_from_slice(&payload);
    assert!(read_to_fetcher(&mut framed.as_slice()).is_err(), "more is a flag");
    let bad_measure = [KIND_MEASURE_ROUND_TRIP, 0, 0, 0, 1, 2];
    assert!(read_to_fetcher(&mut bad_measure.as_slice()).is_err(), "measure is a flag");
    // A fetcher may not send the trusted core's messages, nor the core a fetcher's.
    let mut bytes = Vec::new();
    write_to_fetcher(&mut bytes, &ToFetcher::MeasureRoundTrip(true)).unwrap();
    assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_MEASURE_ROUND_TRIP))));
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &FromFetcher::RoundTrip { microseconds: 1 }).unwrap();
    assert!(matches!(read_to_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_ROUND_TRIP))));
}

/// REQ-708: a refusal carries a real status or none, and a file by its role.
#[test]
fn a_refusal_is_held_to_its_bounds() {
    let refused = |status| FromFetcher::Refused { file: RefusedFile::Page(0x100), status };
    assert!(write_to_core(&mut Vec::new(), &refused(Some(9_999))).is_err(), "no status has four digits");
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &refused(Some(500))).unwrap();
    // The status's high byte.
    let high = bytes.len() - 2;
    bytes[high] = 0xFF;
    assert!(read_from_fetcher(&mut bytes.as_slice()).is_err(), "a status out of bounds closes the fetcher");
    let mut page_of_the_channel = Vec::new();
    write_to_core(&mut page_of_the_channel, &FromFetcher::Refused { file: RefusedFile::Channel, status: None }).unwrap();
    let place = page_of_the_channel.len() - 3;
    page_of_the_channel[place] = 1;
    assert!(read_from_fetcher(&mut page_of_the_channel.as_slice()).is_err(), "a channel's file has no page");
}

/// REQ-704: a `refresh` outside what `channel.ttx` may declare is refused on
/// both sides, so a fetcher cannot have the core ask too often.
#[test]
fn a_refresh_is_held_to_its_bounds() {
    assert!(write_to_core(&mut Vec::new(), &FromFetcher::Refresh(MIN_REFRESH_SECONDS - 1)).is_err());
    assert!(write_to_core(&mut Vec::new(), &FromFetcher::Refresh(MAX_REFRESH_SECONDS + 1)).is_err());
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &FromFetcher::Refresh(MIN_REFRESH_SECONDS)).unwrap();
    let last = bytes.len() - 1;
    bytes[last] = 1;
    assert!(read_from_fetcher(&mut bytes.as_slice()).is_err());
}
