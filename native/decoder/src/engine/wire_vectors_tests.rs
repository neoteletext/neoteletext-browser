//! Renderer parity's rows of the shared wire vectors (neoteletext-common's
//! `wire-vectors.tsv`, `docs/renderer-parity.md` PAR-44; REQ-6770,
//! REQ-6771), read through the decoder as a channel's pushed messages:
//! WELCOME agreeing both picture languages, `DRAWING_LINKS` and
//! `REGIS_CURSOR` for a NAPLPS sub-page, and the ReGIS and NAPLPS bytes of
//! a Greek and an Arabic text part drawn in their provided sets.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "a test reports a failure by panicking, and indexes and counts what it made"
)]

use neoteletext_core::types::{PageAddress, SubCode};
#[cfg(feature = "interaction")]
use neoteletext_input::control::{Control, MAX_BODY, write_body};
use neoteletext_web::records::regis::{RegisRecord, encode, encode_naplps};
use neoteletext_web::wire_vectors::{bytes_of, vectors};
use protocol::regis::unpack;

use super::Engine;

/// The page and sub-code of every vector (the file's own statement).
const PAGE: u16 = 0x101;
/// Its sub-code.
const SUB: u16 = 1;
/// The NAPLPS object the vectors' links and cursor name.
const OBJECT: u16 = 301;

/// The value of the shared table's row of `kind` named `name`.
fn row(kind: &str, name: &str) -> &'static str {
    vectors().find(|row| row.kind == kind && row.name == name).unwrap_or_else(|| panic!("no {kind} row {name}")).value
}

/// `value`'s bytes: one message or packet, or the packets of a `records`
/// row one after the other.
pub(crate) fn vector_bytes(value: &str) -> Vec<u8> {
    let mut all = Vec::new();
    for part in value.split(' ') {
        let (bytes, length) = bytes_of(part).unwrap();
        all.extend_from_slice(&bytes[..length]);
    }
    all
}

/// The packets of the shared table's `records` row `name`.
pub(crate) fn records_row(name: &str) -> Vec<u8> {
    vector_bytes(row("records", name))
}

/// A stream record of `data` for `page`'s sub-page `sub`, generation 1, as
/// the packets of a pushed message: ReGIS, or NAPLPS.
pub(crate) fn picture_stream(page: u16, sub: u16, naplps: bool, data: &[u8]) -> Vec<u8> {
    let record = RegisRecord { page: PageAddress::from_mpp(page).unwrap(), subcode: SubCode::new(sub), generation: 1, offset: 0, data };
    let mut packets = Vec::new();
    let mut out = |packet: neoteletext_core::packet::Packet| packets.extend_from_slice(&packet.bytes);
    if naplps { encode_naplps(record, 0, 0, &mut out) } else { encode(record, 0, 0, &mut out) }.unwrap();
    packets
}

/// PAR-1: the shared WELCOME agreeing both picture languages reaches the
/// trusted core as its control body, with both bits. Only a build with
/// typing (`interaction`) passes controls on.
#[cfg(feature = "interaction")]
#[test]
fn the_shared_welcome_of_the_picture_languages_is_passed_on() {
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &vector_bytes(row("record", "welcome_pictures")));
    let controls = engine.take_input().controls;
    let caps =
        neoteletext_input::caps::CURSOR_CAP | neoteletext_input::caps::SELECT_CAP | neoteletext_input::caps::REGIS_CAP | neoteletext_input::caps::NAPLPS_CAP;
    let mut body = [0_u8; MAX_BODY];
    let length = write_body(&Control::Welcome { version: neoteletext_input::VERSION, caps, resume: None }, &mut body).unwrap();
    assert_eq!(controls, [body[..length].to_vec()]);
}

/// PAR-2: the shared `DRAWING_LINKS` and `REGIS_CURSOR` rows, sent for a
/// NAPLPS sub-page, make its object 301 the cursor and a link to 302.
#[test]
fn the_shared_links_and_cursor_reach_a_naplps_sub_page() {
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &picture_stream(PAGE, SUB, true, b"A"));
    engine.feed_whole(true, true, &records_row("drawing_links"));
    engine.feed_whole(true, true, &records_row("regis_cursor"));
    assert_eq!(engine.naplps().cursor(PAGE, SUB), OBJECT);
    #[cfg(feature = "links")]
    assert_eq!(engine.regis().take_links(), [(PAGE, SUB, vec![(OBJECT, "302".to_owned())])]);
}

/// The picture `data` draws on its own page, `page`, in the picture
/// language it is.
fn drawn(page: u16, naplps: bool, data: &[u8]) -> Vec<u8> {
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &picture_stream(page, 0, naplps, data));
    let (regis, naplps_drawn) = engine.take_pictures(page, None, 0);
    let drawn = if naplps { naplps_drawn } else { regis }.unwrap_or_else(|| panic!("page {page:x} drew nothing"));
    unpack(&drawn.picture.packed)
}

/// `bytes` with every occurrence of `part` taken out.
fn without(bytes: &[u8], part: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(part) {
            at += part.len();
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

/// PAR-4, PAR-5, PAR-12, PAR-14: each shared text part draws, and draws in
/// its provided set: the same codes with the set's selection taken out
/// (ReGIS's `T(A…)`, NAPLPS's designation into G0), drawn in the primary
/// set, give another picture.
#[test]
fn the_shared_text_parts_draw_in_their_provided_sets() {
    // The selection each row makes of its set (renderer parity PAR-4 and
    // PAR-5): ReGIS alphabet 100 plus the set (Greek 17, Arabic 51), and
    // the NAPLPS designation of REQ-6668's table.
    let parts: [(&str, bool, &[u8]); 4] = [
        ("regis_greek", false, b"T(A117)"),
        ("regis_arabic", false, b"T(A151)"),
        ("naplps_greek", true, b"\x1b\x28\x21\x31"),
        ("naplps_arabic", true, b"\x1b\x28\x23\x33"),
    ];
    for (page, (name, naplps, selection)) in (0x102_u16..).zip(parts) {
        let bytes = vector_bytes(row("picture_text", name));
        assert!(bytes.windows(selection.len()).any(|window| window == selection), "{name} selects its set");
        let in_its_set = drawn(page, naplps, &bytes);
        assert!(in_its_set.iter().any(|&pixel| pixel != 0), "{name} draws");
        assert_ne!(in_its_set, drawn(page, naplps, &without(&bytes, selection)), "{name} draws in its own set");
    }
}
