//! The fetcher pipe's tests: every kind round trips, cookie messages keep
//! their bounds, and malformed contents and exchanges are refused.

use super::*;
use crate::ProtocolError;

#[test]
fn every_kind_round_trips() {
    for message in [
        FromFetcher::Contents(vec![Entry::Section("News".into()), Entry::Page { page: 0x101, title: "Headlines".into() }]),
        FromFetcher::Page { origin: Origin::PushedPage, bytes: b"PN,10100\\nOL,1,Hello\\n".to_vec() },
        FromFetcher::Page { origin: Origin::PushedPackets, bytes: vec![0; 42] },
        FromFetcher::Page { origin: Origin::Fetched, bytes: vec![1, 2, 3] },
        FromFetcher::Page { origin: Origin::FetchedPage, bytes: b"PN,10000\\n".to_vec() },
        FromFetcher::Page { origin: Origin::FetchedPackets, bytes: vec![0; 84] },
        FromFetcher::Notice("Opened".into()),
        FromFetcher::Pushing(true),
        FromFetcher::Exchange { method: "GET".into(), url: "https://a.example/100.tti".into(), status: 200, bytes: 512, elapsed_us: 1_234 },
        FromFetcher::Confined,
        FromFetcher::ProxyFailed,
        FromFetcher::CertificateRefused(CertificateProblem::Expired),
        FromFetcher::CertificateRefused(CertificateProblem::Handshake),
        FromFetcher::CookieQuestion { id: 7, host: "a.example".into(), path: "/tt/100.tti".into(), secure: true, site: CookieSite::SameService },
        FromFetcher::CookieQuestion { id: 8, host: "b.example".into(), path: "/".into(), secure: false, site: CookieSite::CrossOriginForm },
        FromFetcher::ReachedQuestion { id: 9, host: "a.example".into() },
        FromFetcher::CookieSet { host: "a.example".into(), path: "/tt/".into(), scope: Some("/tt/".into()), secure: true, line: b"id=1; Secure".to_vec() },
        FromFetcher::CookieSet { host: "a.example".into(), path: "/p".into(), scope: None, secure: false, line: b"x=2".to_vec() },
        FromFetcher::ClearSiteData("a.example".into()),
    ] {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &message).unwrap();
        assert_eq!(read_from_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
    for message in [
        ToFetcher::CookieAnswer { id: 7, header: b"id=1".to_vec() },
        ToFetcher::CookieAnswer { id: 8, header: Vec::new() },
        ToFetcher::ReachedAnswer { id: 9, reached: true },
    ] {
        let mut bytes = Vec::new();
        write_to_fetcher(&mut bytes, &message).unwrap();
        assert_eq!(read_to_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
}

/// Decision 62: cookie messages within their bounds only.
#[test]
fn cookie_messages_are_bounded() {
    let long = FromFetcher::CookieSet { host: "a.example".into(), path: "/".into(), scope: None, secure: true, line: vec![b'x'; MAX_SET_COOKIE + 1] };
    assert!(write_to_core(&mut Vec::new(), &long).is_err());
    let mut bytes = vec![KIND_COOKIE_SET];
    let mut payload = vec![0, 1, b'a', 0, 1, b'/', 0];
    payload.extend(std::iter::repeat_n(b'x', MAX_SET_COOKIE + 1));
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend(payload);
    assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::BadValue("Set-Cookie too long"))));
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
