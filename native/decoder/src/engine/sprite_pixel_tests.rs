//! Teletext sprites drawn by the decoder (neoteletext-common's
//! `docs/animation.md` ANIM-7, ANIM-8, ANIM-17; REQ-6536; TODO-6609): a
//! sprite whose frame is a passive object defined in the page's own
//! packets X/26, as the demonstration site's pages 151 to 153 send theirs,
//! and one whose frame is an object of a Public Object Page linked from
//! the page, as a sprite sheet sent by the server's `object_page` is. Each
//! is checked by the dots the decoder renders, not by its state alone.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "a test builds its packets from small numbers it chose and indexes what it built"
)]

use super::Engine;
use neoteletext_core::consts::{
    COMPOSITIONAL_LINKS, DATA_BYTES, LINK_FUNCTION_GPOP, LINK_FUNCTION_POP, OBJECT_BODY_TRIPLETS, OBJECT_POINTER_UNUSED, PACKET_ENHANCEMENT,
    TRIPLETS_PER_PACKET,
};
use neoteletext_core::ecc::with_odd_parity;
use neoteletext_core::packet::{ColourData, CompositionalLink, ObjectKind, ObjectNumber, ObjectUsage, Packet, Triplet, TripletFunction, encode};
use neoteletext_core::types::{ControlBits, Magazine, PageAddress, PageNumber, SubCode};
use neoteletext_web::records::sprites::Sprite;

const PAGE: u16 = 0x100;
const POP: u16 = 0x1A0;
/// Table 28's object sources, as a sprite's frame reference carries them
/// (ANIM-7): local, and a Public Object Page.
const SOURCE_LOCAL: u16 = 1;
const SOURCE_PUBLIC: u16 = 2;
/// Table 28's passive object type.
const PASSIVE: u16 = 3;
/// Where a frame reference holds its source and its type.
const SOURCE_SHIFT: u16 = 11;
const KIND_SHIFT: u16 = 9;
/// Level 1 colours (Table 30): red and yellow, in the first CLUT.
const RED: u8 = 1;
const YELLOW: u8 = 3;
/// The page's sprite: number 1, shown, at column 10, row 5, two cells.
const COLUMN: i8 = 10;
const ROW: i8 = 5;

fn magazine() -> Magazine {
    Magazine::new(1).unwrap()
}

fn header(mpp: u16) -> Packet {
    let address = PageAddress::from_mpp(mpp).unwrap();
    encode::header(address.magazine, address.page, SubCode::new(0), ControlBits(0), &[with_odd_parity(b' '); 32]).unwrap()
}

fn row(number: u8, text: &[u8]) -> Packet {
    let mut bytes = [with_odd_parity(b' '); DATA_BYTES];
    for (slot, &code) in bytes.iter_mut().zip(text) {
        *slot = with_odd_parity(code);
    }
    encode::row(magazine(), number, &bytes)
}

/// A triplet that does nothing: a reserved row mode (Table 28, 00010).
fn filler() -> Triplet {
    TripletFunction::ReservedRow { mode: 0b00010, address: 41, data: 0 }.to_triplet()
}

fn triplets(number: u8, byte_6: u8, functions: &[TripletFunction]) -> Packet {
    let mut all = [filler(); TRIPLETS_PER_PACKET];
    for (slot, function) in all.iter_mut().zip(functions) {
        *slot = function.to_triplet();
    }
    encode::triplet_packet(magazine(), number, byte_6, &all)
}

/// Two yellow letters on red, as a sprite's frame draws them.
fn letters() -> Vec<TripletFunction> {
    vec![
        TripletFunction::ForegroundColour { column: 0, colour: ColourData { index: YELLOW, high: 0 } },
        TripletFunction::BackgroundColour { column: 0, colour: ColourData { index: RED, high: 0 } },
        TripletFunction::G0Character { column: 0, code: b'S' },
        TripletFunction::G0Character { column: 1, code: b'P' },
    ]
}

fn define(number: ObjectNumber) -> TripletFunction {
    TripletFunction::ObjectDefinition { usage: ObjectUsage(0b11), kind: ObjectKind::Passive, number }
}

/// The page's text, with the given packets X/26 and X/27 before its rows.
fn page_packets(others: Vec<Packet>) -> Vec<u8> {
    let mut packets = vec![header(PAGE)];
    packets.extend(others);
    for number in 1..=23 {
        packets.push(row(number, b" a page of plain text under its sprites"));
    }
    packets.iter().flat_map(|packet| packet.bytes).collect()
}

/// The sprite record of the page: sprite 1 whose one frame is `reference`.
fn sprites(reference: u16) -> Vec<u8> {
    let frames = reference.to_be_bytes();
    let sprite = [Sprite::new(1, true, true, 1, 0, (COLUMN, ROW), (2, 1), &frames)];
    let mut packets = Vec::new();
    neoteletext_web::records::sprites::encode(PageAddress::from_mpp(PAGE).unwrap(), SubCode::new(0), &sprite, 0, 0, &mut |packet| {
        packets.extend_from_slice(&packet.bytes);
    })
    .unwrap();
    packets
}

/// The dots of the sprite's two cells, as the decoder renders the page.
fn sprite_dots(engine: &Engine) -> Vec<u8> {
    let rendered = engine.render(PAGE).unwrap();
    let width = usize::from(interface_core::canvas::PAGE_WIDTH);
    let (cell_width, cell_height) = (12, 20);
    let (left, top) = (usize::from(COLUMN.unsigned_abs()) * cell_width, usize::from(ROW.unsigned_abs()) * cell_height);
    (top..top + cell_height).flat_map(|y| (left..left + 2 * cell_width).map(move |x| y * width + x)).map(|dot| rendered.indices[dot]).collect()
}

/// Whether `dots` hold the sprite's red background: the page's own text is
/// white on black there.
fn shows_sprite(engine: &Engine, dots: &[u8]) -> bool {
    let rendered = engine.render(PAGE).unwrap();
    dots.iter().any(|&index| rendered.palette[usize::from(index)] == [0xFF, 0, 0])
}

/// TODO-6609: a sprite whose frame is a passive object of the page's own
/// packets X/26, which is what the demonstration site's pages 151 to 153
/// send, is drawn by the decoder: its cells take the object's colours.
#[test]
fn a_sprite_framed_by_the_pages_own_object_is_drawn() {
    let mut engine = Engine::new();
    // X/26/0 holds the page's own data, ended; the object starts X/26/1.
    let number = ObjectNumber(1 << 4);
    let mut object = vec![define(number)];
    object.extend(letters());
    object.push(TripletFunction::Termination { kind: 0b101 });
    let packets = vec![triplets(PACKET_ENHANCEMENT, 0, &[TripletFunction::Termination { kind: 0b111 }]), triplets(PACKET_ENHANCEMENT, 1, &object)];
    engine.feed_whole(true, true, &page_packets(packets));
    assert!(!shows_sprite(&engine, &sprite_dots(&engine)), "no sprite before its record");
    engine.feed_whole(true, true, &sprites((SOURCE_LOCAL << SOURCE_SHIFT) | (PASSIVE << KIND_SHIFT) | number.0));
    assert!(shows_sprite(&engine, &sprite_dots(&engine)), "the sprite's cells take its object's red");
}

/// TODO-6609 and ANIM-17: a sprite whose frame is an object of a Public
/// Object Page the page links to (packet X/27/4), sent before the page as
/// the server's `object_page` sends a teletext sprite sheet, is drawn too.
#[test]
fn a_sprite_framed_by_an_object_page_is_drawn() {
    let mut engine = Engine::new();
    // The object page: object 0 of the passive type, sub-page 0.
    let mut body = vec![TripletFunction::Termination { kind: 0b011 }.to_triplet(); usize::from(OBJECT_BODY_TRIPLETS)];
    let number = ObjectNumber(0);
    for (position, function) in core::iter::once(define(number)).chain(letters()).chain([TripletFunction::Termination { kind: 0b011 }]).enumerate() {
        body[position] = function.to_triplet();
    }
    let mut pointers = [filler(); TRIPLETS_PER_PACKET];
    // Packet 1 triplet 1 + 2 (the passive type of the first pair): object
    // 0 at position 0; every other pointer unused.
    for (index, slot) in pointers.iter_mut().enumerate().skip(1) {
        let even = if index == 3 { 0 } else { OBJECT_POINTER_UNUSED };
        *slot = encode::pointer_triplet(even, OBJECT_POINTER_UNUSED);
    }
    let mut unused = [filler(); TRIPLETS_PER_PACKET];
    for slot in unused.iter_mut().skip(1) {
        *slot = encode::pointer_triplet(OBJECT_POINTER_UNUSED, OBJECT_POINTER_UNUSED);
    }
    let mut object_page = vec![header(POP), encode::triplet_packet(magazine(), 1, 1, &pointers), encode::triplet_packet(magazine(), 2, 1, &unused)];
    object_page.push(encode::triplet_packet(magazine(), 3, 0, &body[..TRIPLETS_PER_PACKET].try_into().unwrap()));
    let object_page: Vec<u8> = object_page.iter().flat_map(|packet| packet.bytes).collect();
    engine.feed_whole(true, true, &object_page);
    // The page links to the object page as its POP, and to no GPOP.
    let address = PageAddress::from_mpp(POP).unwrap();
    let link = |function: u8, page: PageNumber| CompositionalLink {
        function,
        validity: 0b11,
        magazine: address.magazine,
        page,
        subpages: 0xFFFF,
        marker_first: true,
        marker_second: false,
    };
    let none = PageNumber(0xFF);
    let links: [CompositionalLink; COMPOSITIONAL_LINKS] =
        [link(LINK_FUNCTION_GPOP, none), link(LINK_FUNCTION_POP, address.page), link(0b10, none), link(0b11, none), link(0, none), link(0, none)];
    engine.feed_whole(true, true, &page_packets(vec![encode::compositional_links(magazine(), 4, &links)]));
    assert!(!shows_sprite(&engine, &sprite_dots(&engine)), "no sprite before its record");
    engine.feed_whole(true, true, &sprites((SOURCE_PUBLIC << SOURCE_SHIFT) | (PASSIVE << KIND_SHIFT) | number.0));
    assert!(shows_sprite(&engine, &sprite_dots(&engine)), "the sprite's cells take the object page's object's red");
}
