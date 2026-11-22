//! The NAPLPS operations at their bounds (script-api.md sections 7.1,
//! 7.2, 10.1, 10.2 and 10.5): each refusal is a `RangeError` whose
//! message begins with the bound's name, or a `TypeError`, and leaves the
//! host as it was; rates are counted before the work; and what the host
//! holds and stages is counted against the container's bounds
//! (TODO-7272).
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::naive_bytecount, reason = "tests")]

use protocol::script::{ContextFlags, Level};

use super::program::Paint;
use super::*;
use crate::host::EventKind;
use crate::random::SEED_BYTES;

const PAGE: i32 = 2;
const CHANNEL: i32 = 1;
const SO: u8 = 0x0E;
const RESET: u8 = 0x20;
const SET_COLOR: u8 = 0x3C;
const SELECT_COLOR: u8 = 0x3E;
const BLINK: u8 = 0x3F;

fn opening(context: ContextId, renderer: Renderer) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer,
        level: Level::TwoFive,
        page: 0x1A0,
        subcode: 0,
        flags: ContextFlags::default(),
        tick_divisor: 1,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/1A0.js".into(),
        script_url: "https://example.test/chan/1A0.js".into(),
        channel_title: "Example".into(),
        seed: [9; SEED_BYTES],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: 1,
    }
}

/// A host with both contexts open on a NAPLPS page, in a callback of the
/// page context at tick 0.
fn host() -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, Renderer::Naplps), 100);
    host.open(opening(ContextId::Page, Renderer::Naplps), 200);
    at(&mut host, 0);
    host
}

/// Begins a step at `tick` and a callback of the page context in it.
fn at(host: &mut Host, tick: u64) {
    host.begin_step(tick, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
}

fn words(words: &[i32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn paint(host: &mut Host, op: Op, program: &[i32]) -> Reply {
    host.call(PAGE, op.number(), &[], Payload::Ints(&words(program)))
}

fn define(host: &mut Host, name: &str, rgb: i32) -> Reply {
    host.call(PAGE, Op::NP_COLOUR_DEFINE.number(), &[rgb], Payload::Text(name.as_bytes()))
}

fn call(host: &mut Host, op: Op, ints: &[i32]) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::None)
}

/// What the step staged of NAPLPS: whether it begins a generation, and the bytes.
fn staged(host: &mut Host) -> Option<(bool, Vec<u8>)> {
    host.take_batch(0, &[]).naplps.map(|append| (append.new_generation, append.bytes))
}

#[track_caller]
fn assert_range(reply: &Reply, bound: &str) {
    assert!(matches!(reply, Reply::Range(message) if message.starts_with(&format!("{bound}: "))), "{reply:?} is not a RangeError of {bound}");
}

#[track_caller]
fn assert_type(reply: &Reply) {
    assert!(matches!(reply, Reply::Type(_)), "{reply:?} is not a TypeError");
}

/// A program that fills a rectangle.
const RECT: [i32; 5] = [Paint::FILL_RECT as i32, 0, 0, 10, 10];

/// A program of `calls` polylines of the most points: about 6,150 bytes each.
fn long(calls: usize) -> Vec<i32> {
    let mut program = Vec::new();
    for _ in 0..calls {
        program.extend_from_slice(&[Paint::POLYLINE as i32, 1_024]);
        program.extend((0..1_024).flat_map(|index| [index % 400, index % 300]));
    }
    program
}

/// Section 7.2: the first painting of a page begins a generation (the
/// reset, the domain and teletext's eight colours go before it); later
/// ones are appended to it; and the batch carries which.
#[test]
fn the_first_paint_begins_a_generation_and_later_ones_append() {
    let mut host = host();
    assert_eq!(staged(&mut host), None);
    assert_eq!(paint(&mut host, Op::NP_PAINT, &RECT), Reply::Undefined);
    let (new, bytes) = staged(&mut host).unwrap();
    assert!(new);
    assert_eq!(bytes[..4], [SO, RESET, 0x7F, 0x7F]);
    assert_eq!(bytes.iter().filter(|byte| **byte == SET_COLOR).count(), 8, "teletext's eight colours");
    assert_eq!(staged(&mut host), None, "a batch takes what was staged");
    at(&mut host, 1);
    assert_eq!(paint(&mut host, Op::NP_PAINT, &RECT), Reply::Undefined);
    let (new, bytes) = staged(&mut host).unwrap();
    assert!(!new);
    assert!(!bytes.contains(&RESET) && !bytes.contains(&SET_COLOR), "{bytes:02X?}");
    // Two paintings of one step leave as one append, in order.
    at(&mut host, 2);
    paint(&mut host, Op::NP_PAINT, &RECT);
    let one = host.naplps.staged.len();
    paint(&mut host, Op::NP_PAINT, &[Paint::FILL_RECT as i32, 20, 20, 5, 5]);
    assert!(host.naplps.staged.len() > one);
    assert!(matches!(staged(&mut host), Some((false, _))));
}

/// Section 10.2 and rule 3 of 10.1: a program with a fault changes
/// nothing: nothing is staged, no generation begins, and the pen is where
/// it was.
#[test]
fn a_refused_painting_changes_nothing() {
    let mut host = host();
    assert_type(&paint(&mut host, Op::NP_PAINT, &[Paint::FILL_RECT as i32, 0, 0, 10, 10, Paint::MOVE_TO as i32, 1]));
    assert_range(&paint(&mut host, Op::NP_PAINT, &[Paint::FILL_RECT as i32, 0, 0, 10, 10, Paint::INK as i32, 16]), "NAPLPS_COLOURS");
    assert!(!host.naplps.started);
    assert_eq!(staged(&mut host), None);
    // A payload that is no painter program, and whole numbers where none belong.
    assert_type(&host.call(PAGE, Op::NP_PAINT.number(), &[], Payload::Text(b"x")));
    assert_type(&host.call(PAGE, Op::NP_PAINT.number(), &[], Payload::None));
    assert_type(&host.call(PAGE, Op::NP_PAINT.number(), &[1], Payload::Ints(&words(&RECT))));
    assert_range(&host.call(PAGE, Op::NP_PAINT.number(), &[], Payload::Ints(&vec![0; (MAX_PAINTER_WORDS + 1) * 4])), "MAX_PAINTER_WORDS");
    // After a good painting, a bad one leaves the pen and the stream as they were.
    paint(&mut host, Op::NP_PAINT, &[Paint::MOVE_TO as i32, 50, 50, Paint::INK as i32, 3]);
    let before = format!("{:?}", host.naplps);
    assert_type(&paint(&mut host, Op::NP_PAINT, &[Paint::MOVE_TO as i32, 1, 1, Paint::FILL_RECT as i32, 0, 0, 10, 10, Paint::STROKE as i32, 9]));
    assert_eq!(format!("{:?}", host.naplps), before);
}

/// Section 3.1 ("renderer state belongs to the page") and section 7: the
/// backdrop and the colour map are the page context's, on a NAPLPS page.
#[test]
fn only_the_page_context_of_a_naplps_page_paints() {
    let mut host = host();
    assert_type(&host.call(CHANNEL, Op::NP_PAINT.number(), &[], Payload::Ints(&words(&RECT))));
    assert_type(&host.call(CHANNEL, Op::NP_COLOUR_DEFINE.number(), &[0], Payload::Text(b"sky")));
    for renderer in [Renderer::Teletext, Renderer::Regis] {
        let mut host = Host::new();
        host.open(opening(ContextId::Page, renderer), 200);
        at(&mut host, 0);
        assert_type(&paint(&mut host, Op::NP_PAINT, &RECT));
        assert_type(&define(&mut host, "sky", 0));
        assert_type(&call(&mut host, Op::LISTEN, &[BACKDROP_HANDLE, EventKind::PAINTED.number(), 1]));
    }
    // The page's NAPLPS goes with its context: the next page begins empty.
    define(&mut host, "sky", 0x11_33_66);
    paint(&mut host, Op::NP_PAINT, &RECT);
    host.close(ContextId::Page);
    host.open(opening(ContextId::Page, Renderer::Naplps), 200);
    at(&mut host, 5);
    assert_eq!(staged(&mut host), None);
    assert_eq!(define(&mut host, "sea", 0), Reply::Int(8));
    assert!(matches!(staged(&mut host), Some((true, _))));
}

/// Section 7.1, `define`: the next free entry of 8 to 15, set by SELECT
/// COLOR and SET COLOR with the six-byte colour of RAS-6; a name defined
/// before keeps its entry; teletext's names are not defined again.
#[test]
fn colours_are_defined_in_entries_eight_to_fifteen() {
    let mut host = host();
    assert_eq!(define(&mut host, "sky", 0x11_33_66), Reply::Int(8));
    let (new, bytes) = staged(&mut host).unwrap();
    assert!(new, "the first colour begins the generation");
    // SELECT COLOR entry 8 (4/0 + 4 x 8 = 6/0), SET COLOR, six bytes.
    let at_colour = bytes.windows(3).rposition(|window| window == [SELECT_COLOR, 0x60, SET_COLOR]).unwrap();
    let mut colour = Vec::new();
    for pair in (0..6).rev() {
        let bit = |value: u32, at: u32| u8::from((value << 4) >> at & 1 == 1);
        let (low, high) = (pair * 2, pair * 2 + 1);
        colour.push(0x40 | bit(0x33, high) << 5 | bit(0x11, high) << 4 | bit(0x66, high) << 3 | bit(0x33, low) << 2 | bit(0x11, low) << 1 | bit(0x66, low));
    }
    assert_eq!(bytes[at_colour + 3..at_colour + 9], colour[..]);
    assert_eq!(define(&mut host, "sky", 0x00_00_FF), Reply::Int(8), "the same name is the same entry");
    for (index, name) in ["a", "b", "c", "d", "e", "f", "g"].into_iter().enumerate() {
        assert_eq!(define(&mut host, name, 0), Reply::Int(9 + i32::try_from(index).unwrap()));
    }
    assert_range(&define(&mut host, "ninth", 0), "NAPLPS_COLOURS");
    assert_type(&define(&mut host, "white", 0));
    assert_type(&define(&mut host, "a\u{7}", 0));
    assert_type(&define(&mut host, "z", 0x0100_0000));
    assert_type(&define(&mut host, "z", -1));
    assert_range(&define(&mut host, "", 0), "MAX_COLOUR_NAME_BYTES");
    assert_range(&define(&mut host, &"n".repeat(33), 0), "MAX_COLOUR_NAME_BYTES");
    assert_type(&host.call(PAGE, Op::NP_COLOUR_DEFINE.number(), &[0], Payload::None));
    // `set` takes a defined entry of 8 to 15.
    let mut fresh = self::host();
    define(&mut fresh, "sky", 0);
    assert_eq!(call(&mut fresh, Op::NP_COLOUR_SET, &[8, 0xFF_EE_88]), Reply::Undefined);
    assert_type(&call(&mut fresh, Op::NP_COLOUR_SET, &[9, 0]));
    assert_range(&call(&mut fresh, Op::NP_COLOUR_SET, &[7, 0]), "NAPLPS_COLOURS");
    assert_range(&call(&mut fresh, Op::NP_COLOUR_SET, &[16, 0]), "NAPLPS_COLOURS");
    assert_type(&call(&mut fresh, Op::NP_COLOUR_SET, &[8, 0x0100_0000]));
    assert_type(&call(&mut fresh, Op::NP_COLOUR_SET, &[8]));
}

/// Section 10.5, `MAX_COLOUR_SETS_PER_SECOND`: an entry is redefined at
/// most six times in a second; past that nothing throws, and the latest
/// value is applied at the next tick the rate allows.
#[test]
fn a_colour_changed_too_often_takes_its_latest_value_when_the_rate_allows() {
    let mut host = host();
    define(&mut host, "lamp", 0);
    staged(&mut host);
    let sets = |bytes: &[u8]| bytes.iter().filter(|byte| **byte == SET_COLOR).count();
    for value in 1..=6 {
        at(&mut host, u64::try_from(value).unwrap());
        assert_eq!(call(&mut host, Op::NP_COLOUR_SET, &[8, value]), Reply::Undefined);
        assert_eq!(sets(&staged(&mut host).unwrap().1), 1, "change {value}");
    }
    // The seventh and eighth within the second are held back.
    at(&mut host, 7);
    assert_eq!(call(&mut host, Op::NP_COLOUR_SET, &[8, 0x00_00_70]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::NP_COLOUR_SET, &[8, 0x00_00_80]), Reply::Undefined);
    assert_eq!(staged(&mut host), None);
    at(&mut host, 59);
    assert_eq!(staged(&mut host), None);
    // The first change was at tick 1: at tick 61 a second has passed since it.
    at(&mut host, 61);
    let (new, bytes) = staged(&mut host).unwrap();
    assert!(!new);
    assert_eq!(sets(&bytes), 1);
    // Blue 0x80: its top bit alone, bit 3 of the first of six bytes.
    let colour = bytes.iter().position(|byte| *byte == SET_COLOR).unwrap();
    assert_eq!(bytes[colour + 1..colour + 7], [0x48, 0x40, 0x40, 0x40, 0x40, 0x40]);
    at(&mut host, 62);
    assert_eq!(staged(&mut host), None, "it is applied once");
}

/// Section 7.1, `blink` and `steady`: BLINK with the other entry and the
/// three intervals; at most `MAX_BLINKS` processes; a blink between the
/// same two entries replaces the one before; `steady` is BLINK alone.
#[test]
fn blinks_are_bounded_and_replace_one_another() {
    let mut host = host();
    for name in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        define(&mut host, name, 0);
    }
    staged(&mut host);
    assert_eq!(call(&mut host, Op::NP_BLINK, &[8, 1, 5, 6, 7]), Reply::Undefined);
    let (_, bytes) = staged(&mut host).unwrap();
    // SELECT COLOR entry 8; BLINK to entry 1 (4/4), on 5, off 6, delay 7.
    assert!(bytes.windows(7).any(|window| window == [SELECT_COLOR, 0x60, BLINK, 0x44, 0x45, 0x46, 0x47]), "{bytes:02X?}");
    assert_eq!(host.naplps.blinks.len(), 1);
    // The same pair again replaces it; an interval of nothing ends it.
    call(&mut host, Op::NP_BLINK, &[8, 1, 9, 9, 0]);
    assert_eq!(host.naplps.blinks, [Blink { from: 8, to: 1, times: [9, 9, 0] }]);
    call(&mut host, Op::NP_BLINK, &[8, 1, 0, 9, 0]);
    assert_eq!(host.naplps.blinks, []);
    // Thirty-two processes, four an entry, and no thirty-third.
    at(&mut host, 100);
    for entry in 8..16 {
        for to in 0..4 {
            assert_eq!(call(&mut host, Op::NP_BLINK, &[entry, to, 5, 5, 0]), Reply::Undefined, "{entry} to {to}");
        }
    }
    assert_eq!(host.naplps.blinks.len(), MAX_BLINKS);
    assert_range(&call(&mut host, Op::NP_BLINK, &[8, 5, 5, 5, 0]), "MAX_BLINKS");
    // Steady: BLINK with no operands after the entry is selected, and the entry's processes gone.
    staged(&mut host);
    assert_eq!(call(&mut host, Op::NP_BLINK, &[9, -1, 0, 0, 0]), Reply::Undefined);
    let (_, bytes) = staged(&mut host).unwrap();
    assert_eq!(bytes, [SELECT_COLOR, 0x64, BLINK, 0x1A]);
    assert_eq!(host.naplps.blinks.len(), MAX_BLINKS - 4);
    assert!(host.naplps.blinks.iter().all(|blink| blink.from != 9));
    // Refusals.
    assert_range(&call(&mut host, Op::NP_BLINK, &[7, 1, 5, 5, 0]), "NAPLPS_COLOURS");
    assert_range(&call(&mut host, Op::NP_BLINK, &[10, 16, 5, 5, 0]), "NAPLPS_COLOURS");
    assert_range(&call(&mut host, Op::NP_BLINK, &[10, 1, 64, 5, 0]), "MAX_BLINK_TENTHS");
    assert_range(&call(&mut host, Op::NP_BLINK, &[10, 1, 5, 5, -1]), "MAX_BLINK_TENTHS");
    // The entry's changes are held to the colour rate, by a RangeError: a blink cannot wait.
    at(&mut host, 200);
    for to in 0..6 {
        assert_eq!(call(&mut host, Op::NP_BLINK, &[15, to, 0, 0, 0]), Reply::Undefined);
    }
    assert_range(&call(&mut host, Op::NP_BLINK, &[15, 0, 0, 0, 0]), "MAX_COLOUR_SETS_PER_SECOND");
}

/// Section 7.2 and 10.5: a painting is at most
/// `NAPLPS_MAX_BYTES_PER_CALLBACK` a callback and
/// `NAPLPS_MAX_BYTES_PER_SECOND` a second, and a step's output at most
/// `MAX_BATCH_BYTES`; past any of them the call throws and stages nothing.
#[test]
fn paintings_are_held_to_their_bytes_a_callback_and_a_second() {
    let mut host = host();
    assert_range(&paint(&mut host, Op::NP_PAINT, &long(11)), "NAPLPS_MAX_BYTES_PER_CALLBACK");
    assert_eq!(staged(&mut host), None);
    // Two paintings of one callback share its allowance.
    assert_eq!(paint(&mut host, Op::NP_PAINT, &long(6)), Reply::Undefined);
    assert_range(&paint(&mut host, Op::NP_PAINT, &long(6)), "NAPLPS_MAX_BYTES_PER_CALLBACK");
    let first = staged(&mut host).unwrap().1.len();
    assert!(first > 36_000 && first <= NAPLPS_MAX_BYTES_PER_CALLBACK, "{first}");
    // Callback after callback, each within its own allowance, until the second's is spent.
    let mut total = first;
    let mut refused = None;
    for tick in 1..10 {
        at(&mut host, tick);
        let reply = paint(&mut host, Op::NP_PAINT, &long(10));
        if reply != Reply::Undefined {
            refused = Some(reply);
            break;
        }
        total += staged(&mut host).unwrap().1.len();
    }
    assert_range(&refused.unwrap(), "NAPLPS_MAX_BYTES_PER_SECOND");
    assert!(total <= NAPLPS_MAX_BYTES_PER_SECOND && total > NAPLPS_MAX_BYTES_PER_SECOND - NAPLPS_MAX_BYTES_PER_CALLBACK, "{total}");
    // A second later the allowance is whole again.
    at(&mut host, 70);
    assert_eq!(paint(&mut host, Op::NP_PAINT, &long(10)), Reply::Undefined);
}

/// Section 10.1 rule 7 (TODO-7272, the security review's finding S-06):
/// what the host stages and holds for NAPLPS is counted against one
/// step's output and the container's host state, and a painting that
/// would pass either is refused whole.
#[test]
fn staged_naplps_is_counted_against_the_batch_and_the_host_state() {
    let mut host = host();
    paint(&mut host, Op::NP_PAINT, &RECT);
    let (held, staged_bytes) = (host.state_bytes(), host.batch_bytes());
    assert_eq!(staged_bytes, host.naplps.staged.len());
    assert!(held >= staged_bytes + host.naplps.state_bytes());
    // The batch's bound.
    host.set_limits_for_test(usize::MAX, staged_bytes + 1_000);
    assert_range(&paint(&mut host, Op::NP_PAINT, &long(1)), "MAX_BATCH_BYTES");
    assert_eq!(host.batch_bytes(), staged_bytes);
    // The host state's bound.
    host.set_limits_for_test(held + 1_000, usize::MAX);
    assert_range(&paint(&mut host, Op::NP_PAINT, &long(1)), "MAX_HOST_STATE_BYTES");
    host.set_limits_for_test(held, usize::MAX);
    assert_range(&define(&mut host, "sky", 0), "MAX_HOST_STATE_BYTES");
    assert_eq!((host.state_bytes(), host.batch_bytes()), (held, staged_bytes));
    // Under the real bounds a script that paints all it may, callback
    // after callback for ten seconds, never makes the host hold more
    // than one step's output of NAPLPS.
    let mut host = self::host();
    let mut most = 0;
    for tick in 0..600 {
        at(&mut host, tick);
        let _ = paint(&mut host, Op::NP_PAINT, &long(10));
        let _ = paint(&mut host, Op::NP_REPAINT, &long(10));
        most = most.max(host.state_bytes());
        assert!(host.batch_bytes() <= crate::limits::MAX_BATCH_BYTES);
        host.take_batch(0, &[]);
    }
    assert!(most <= crate::limits::MAX_BATCH_BYTES + 16_384, "{most} bytes held");
}

/// Section 7.2, `repaint`: a new generation, at most one every
/// `NAPLPS_REPAINT_TICKS`; the host sends the colour map and the blinks
/// again, so the script sees nothing but the new backdrop; what the step
/// had staged for the old generation is not sent.
#[test]
fn a_repaint_begins_a_generation_with_the_colours_and_blinks_again() {
    let mut host = host();
    define(&mut host, "sky", 0x11_33_66);
    define(&mut host, "lamp", 0xFF_EE_88);
    call(&mut host, Op::NP_BLINK, &[9, 8, 5, 5, 0]);
    paint(&mut host, Op::NP_PAINT, &[Paint::WAIT as i32, 20]);
    staged(&mut host);
    // Too soon: the first generation began at tick 0.
    at(&mut host, 29);
    assert_range(&paint(&mut host, Op::NP_REPAINT, &RECT), "NAPLPS_REPAINT_TICKS");
    assert_eq!(staged(&mut host), None);
    at(&mut host, 30);
    paint(&mut host, Op::NP_PAINT, &[Paint::FILL_RECT as i32, 5, 5, 5, 5]);
    assert_eq!(paint(&mut host, Op::NP_REPAINT, &RECT), Reply::Undefined);
    let (new, bytes) = staged(&mut host).unwrap();
    assert!(new);
    assert_eq!(bytes[..4], [SO, RESET, 0x7F, 0x7F]);
    assert_eq!(bytes.iter().filter(|byte| **byte == RESET).count(), 1, "what was staged for the old generation is gone");
    assert_eq!(bytes.iter().filter(|byte| **byte == SET_COLOR).count(), 10, "teletext's eight and the script's two");
    // Entry 9's colour, then its blink to entry 8 with no SELECT COLOR
    // between: the entry is already the one selected.
    assert!(bytes.windows(12).any(|window| window[..3] == [SELECT_COLOR, 0x64, SET_COLOR] && window[9..] == [BLINK, 0x60, 0x45]), "{bytes:02X?}");
    assert_eq!(host.naplps.wait_tenths, 0, "a generation's waits start again");
    // The rate is counted before the work and never given back: a
    // repaint whose program is refused has still been counted.
    at(&mut host, 60);
    assert_type(&paint(&mut host, Op::NP_REPAINT, &[Paint::STROKE as i32, 9]));
    at(&mut host, 61);
    assert_range(&paint(&mut host, Op::NP_REPAINT, &RECT), "NAPLPS_REPAINT_TICKS");
    at(&mut host, 90);
    assert_eq!(paint(&mut host, Op::NP_REPAINT, &RECT), Reply::Undefined);
    // A repaint may be a page's first painting.
    let mut fresh = self::host();
    assert_eq!(paint(&mut fresh, Op::NP_REPAINT, &RECT), Reply::Undefined);
    assert!(matches!(staged(&mut fresh), Some((true, _))));
}

/// Section 3.7: the budget report's NAPLPS entries, and section 7.3's
/// `wait`: a generation's waits are counted in ticks against
/// `WAIT_BUDGET_TICKS`.
#[test]
fn the_budget_reports_the_colour_map_the_bytes_and_the_waits() {
    let mut host = host();
    define(&mut host, "sky", 0);
    call(&mut host, Op::NP_BLINK, &[8, 1, 5, 5, 0]);
    paint(&mut host, Op::NP_PAINT, &[Paint::WAIT as i32, 5, Paint::WAIT as i32, 100]);
    let appended = host.naplps.staged.len();
    let Reply::Text(budget) = call(&mut host, Op::BUDGET, &[]) else { panic!("the budget is text") };
    for entry in [
        "\"blinks\":{\"used\":1,\"limit\":32}".to_owned(),
        format!("\"appendedBytesThisSecond\":{{\"used\":{appended},\"limit\":262144}}"),
        "\"waitTicks\":{\"used\":630,\"limit\":3600}".to_owned(),
        "\"colours\":{\"used\":1,\"limit\":8}".to_owned(),
    ] {
        assert!(budget.contains(&entry), "{entry} in {budget}");
    }
    // A page of another renderer reports none of them.
    let mut other = Host::new();
    other.open(opening(ContextId::Page, Renderer::Teletext), 200);
    at(&mut other, 0);
    let Reply::Text(budget) = call(&mut other, Op::BUDGET, &[]) else { panic!("the budget is text") };
    assert!(!budget.contains("waitTicks"));
}

/// Section 7.3, `measure`: the host computes a text's size from its cells
/// and the character field, with its spacing; proportional spacing is
/// measured as fixed spacing, its upper bound.
#[test]
fn text_is_measured_by_its_cells() {
    let mut host = host();
    let mut measure = |ints: &[i32], payload: &str| host.call(PAGE, Op::NP_MEASURE.number(), ints, Payload::Text(payload.as_bytes()));
    assert_eq!(measure(&[12, 20, 0, 0], "abc"), Reply::Text("36,20".into()));
    assert_eq!(measure(&[12, 20, 1, 0], "abc"), Reply::Text("45,20".into()));
    assert_eq!(measure(&[24, 40, 2, 0], "abc"), Reply::Text("108,40".into()));
    assert_eq!(measure(&[12, 20, 3, 0], "abc"), Reply::Text("36,20".into()));
    assert_eq!(measure(&[12, 20, 0, 0], ""), Reply::Text("0,20".into()));
    // A language of two bytes before the text; a letter with a mark is one cell.
    assert_eq!(measure(&[12, 20, 0, 2], "elαβγδ"), Reply::Text("48,20".into()));
    assert_eq!(measure(&[12, 20, 0, 0], "e\u{301}"), Reply::Text("12,20".into()));
    assert_type(&measure(&[12, 20, 0, 0], "漢"));
    assert_type(&measure(&[12, 20, 4, 0], "a"));
    assert_type(&measure(&[12, 20, 0, 1], "αβ"));
    assert_range(&measure(&[0, 20, 0, 0], "a"), "CANVAS_WIDTH");
    assert_range(&measure(&[12, 461, 0, 0], "a"), "CANVAS_HEIGHT");
    assert_range(&measure(&[12, 20, 0, 17], "aaaaaaaaaaaaaaaaaaaa"), "MAX_LANGUAGE_BYTES");
    assert_range(&measure(&[12, 20, 0, 0], &"a".repeat(1_001)), "MAX_TEXT_CHARS");
}

/// Section 4.2, `painted`, and operation LISTEN: the backdrop is a thing
/// the page context may listen to, by its handle, and the decoder is
/// asked to watch its painting.
#[test]
fn the_backdrop_is_a_thing_whose_painting_is_watched() {
    let mut host = host();
    assert_eq!(call(&mut host, Op::LISTEN, &[BACKDROP_HANDLE, EventKind::PAINTED.number(), 1]), Reply::Undefined);
    assert!(host.listens_to(ContextId::Page, BACKDROP_HANDLE, EventKind::PAINTED));
    let batch = host.take_batch(0, &[]);
    assert_eq!(batch.watch.and_then(|watch| watch.painted), Some(vec![BACKDROP_HANDLE]));
    assert_type(&host.call(CHANNEL, Op::LISTEN.number(), &[BACKDROP_HANDLE, EventKind::PAINTED.number(), 1], Payload::None));
    assert_type(&call(&mut host, Op::LISTEN, &[BACKDROP_HANDLE + 1, EventKind::PAINTED.number(), 1]));
}
