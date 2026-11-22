//! What the fuzz targets of `native/fuzz` reach of the web client: each
//! reader of bytes from outside, behind a function that holds it to what
//! it promises.

use url::Url;

use super::coding::{Decoder, gzip_header_length};
use super::cookies::{Context, Jar};
use super::limits::{MAX_COOKIES_PER_JAR, MAX_GZIP_HEADER};
use super::policy::ChannelPolicy;
use super::sse::EventStream;

/// Whether the address `url` is judged inside the channel whose identity
/// is `channel`, and the address's path as it is sent; `None` when either
/// is no address.
#[must_use]
pub fn channel_holds(channel: &str, url: &str) -> Option<(bool, String)> {
    let url = Url::parse(url).ok()?;
    let policy = ChannelPolicy::new(channel)?;
    Some((policy.is_same_origin(&url), url.path().to_owned()))
}

/// Decodes `coded` as the content coding `coding`, `at_a_time` bytes a
/// step into `room` bytes of room, to at most `most` bytes; what it
/// decoded to, or `None` for bytes that are not the coding's or a coding
/// not decoded.
///
/// # Panics
/// When a step takes more than it was given or writes more than its room.
#[must_use]
pub fn decode(coding: &[u8], coded: &[u8], at_a_time: usize, room: usize, most: usize) -> Option<Vec<u8>> {
    let mut decoder = Decoder::of(coding)?;
    let mut buffer = vec![0_u8; room.max(1)];
    let mut out = Vec::new();
    for piece in coded.chunks(at_a_time.max(1)) {
        let mut rest = piece;
        loop {
            let step = decoder.step(rest, &mut buffer).ok()?;
            assert!(step.taken <= rest.len() && step.written <= buffer.len(), "a step kept to its input and its room");
            out.extend_from_slice(buffer.get(..step.written).unwrap_or_default());
            rest = rest.get(step.taken..).unwrap_or_default();
            if out.len() > most || (step.taken == 0 && step.written == 0) {
                break;
            }
        }
        if out.len() > most {
            break;
        }
    }
    Some(out)
}

/// Reads `bytes` as the start of a gzip member.
///
/// # Panics
/// When a header longer than the bound is taken as one.
pub fn gzip_header(bytes: &[u8]) {
    if let Ok(Some(length)) = gzip_header_length(bytes) {
        assert!(length <= MAX_GZIP_HEADER && length <= bytes.len(), "a header is within its bound and its bytes");
    }
}

/// The events of the event stream `bytes`, given `at_a_time` bytes a
/// call, as type, data and last event ID; `None` when a bound is passed.
#[must_use]
pub fn events(bytes: &[u8], at_a_time: usize) -> Option<Vec<(String, String, String)>> {
    let mut stream = EventStream::default();
    let mut found = Vec::new();
    for piece in bytes.chunks(at_a_time.max(1)) {
        stream.push(piece, &mut |event| found.push((event.kind, event.data, event.last_id))).ok()?;
    }
    Some(found)
}

/// Keeps in one jar the cookies each of `headers` sets as a `Set-Cookie`
/// value of an answer from `url`, then reads back `kept` as a jar the
/// trusted core kept, and returns the `Cookie` header for `url` with the
/// jar written out.
///
/// # Panics
/// When the jar holds more than its bound, or a jar written out does not
/// read back to the same cookies.
#[must_use]
pub fn jar(url: &str, headers: &[&[u8]], kept: &[u8]) -> Option<(Option<String>, Vec<u8>)> {
    const SAME: Context = Context { same_site: true, navigation: true, safe_method: true };
    let url = Url::parse(url).ok().filter(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())?;
    let mut jar = Jar::new(None);
    for header in headers {
        jar.store(&url, header, SAME);
    }
    jar.read_written(kept);
    let (lasting, session) = (jar.write(false), jar.write(true));
    assert!(jar.count() <= MAX_COOKIES_PER_JAR, "a jar keeps to its bound");
    let mut again = Jar::new(None);
    again.read_written(&lasting);
    again.read_written(&session);
    assert_eq!(again.header(&url, SAME), jar.header(&url, SAME), "a jar written out reads back");
    Some((jar.header(&url, SAME), [lasting, session].concat()))
}
