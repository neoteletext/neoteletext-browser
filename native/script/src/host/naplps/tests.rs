//! The NAPLPS operations at their bounds (script-api.md sections 7.1,
//! 7.2, 10.1, 10.2 and 10.5): each refusal is a `RangeError` whose
//! message begins with the bound's name, or a `TypeError`, and leaves the
//! host as it was; rates are counted before the work; and what the host
//! holds and stages is counted against the container's bounds
//! (TODO-7272).
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::naive_bytecount, reason = "tests")]

use neoteletext_rules::animate::{Operation, OperationKind};
use neoteletext_rules::table::Target as AnimateTarget;
use protocol::script::batch::NaplpsObjectKind;
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

// Definitions and the cast (work package N2) ------------------------------------

const END_DEFINITION: [u8; 2] = [0x1B, 0x45];

/// A definition's program whose contents are `22 + 6 x steps` bytes: SO,
/// DOMAIN (8), SELECT COLOR (2), TEXTURE (2), POINT SET to the pen's
/// place (7), LINE relative and an operand a step, and the PDI's end.
fn stepped(steps: i32) -> Vec<i32> {
    let mut program = vec![Paint::POLYLINE_BY as i32, steps];
    program.extend((0..steps).flat_map(|index| [if index % 2 == 0 { 1 } else { -1 }, 0]));
    program
}

fn symbol_in(host: &mut Host, context: i32, existing: i32, size: (i32, i32), program: &[i32]) -> Reply {
    host.call(context, Op::NP_SYMBOL.number(), &[existing, size.0, size.1], Payload::Ints(&words(program)))
}

fn symbol(host: &mut Host, program: &[i32]) -> i32 {
    match symbol_in(host, PAGE, 0, (8, 8), program) {
        Reply::Int(handle) => handle,
        other => panic!("no symbol: {other:?}"),
    }
}

/// `NP_ACTOR`'s whole numbers for an actor at (`x`, `y`) in `layer`.
fn actor_ints(existing: i32, x: i32, y: i32, layer: i32, flags: i32) -> [i32; 15] {
    [existing, 1, x, y, 0, 0, 0, 0, layer, 0, flags, 0, 0, 0, 0]
}

fn actor_in(host: &mut Host, context: i32, ints: [i32; 15], frames: &[i32]) -> Reply {
    host.call(context, Op::NP_ACTOR.number(), &ints, Payload::Ints(&words(frames)))
}

fn actor(host: &mut Host, x: i32, y: i32, frames: &[i32]) -> i32 {
    match actor_in(host, PAGE, actor_ints(0, x, y, 2, 1), frames) {
        Reply::Int(handle) => handle,
        other => panic!("no actor: {other:?}"),
    }
}

/// Section 7.4 and 10.6, `DEFINABLE_CODES`, `MAX_MACRO_BYTES`,
/// `MAX_ALL_MACRO_BYTES` and `CHANNEL_SHARE_SYMBOLS`: each at its bound
/// and one past. A symbol is DEF MACRO with its contents and END.
#[test]
fn symbols_are_held_to_their_codes_and_their_bytes() {
    let mut host = host();
    // The longest contents: 22 + 6 x 679 = 4,096 bytes, and six more are too many.
    let first = symbol(&mut host, &stepped(679));
    let (_, bytes) = staged(&mut host).unwrap();
    let at_definition = bytes.windows(3).position(|window| window == [0x1B, 0x40, 0x20]).expect("DEF MACRO of code 2/0");
    assert_eq!(bytes[at_definition + 3..].len(), 4_096 + 2);
    assert_eq!(bytes[bytes.len() - 2..], END_DEFINITION);
    assert_range(&symbol_in(&mut host, PAGE, 0, (8, 8), &stepped(680)), "MAX_MACRO_BYTES");
    assert_eq!(staged(&mut host), None, "a symbol refused appends nothing");
    // Sixteen of the longest are exactly the 65,536 bytes; a seventeenth is past it.
    for _ in 1..16 {
        symbol(&mut host, &stepped(679));
    }
    assert_range(&symbol_in(&mut host, PAGE, 0, (8, 8), &stepped(679)), "MAX_ALL_MACRO_BYTES");
    // Redefined shorter, the first leaves room for one that fits what it gave up.
    assert_eq!(symbol_in(&mut host, PAGE, first, (8, 8), &stepped(1)), Reply::Int(first));
    symbol(&mut host, &stepped(600));
    // Eighty symbols are the page's, and no eighty-first.
    let mut host = self::host();
    let handles: Vec<i32> = (0..80).map(|_| symbol(&mut host, &RECT)).collect();
    assert_range(&symbol_in(&mut host, PAGE, 0, (8, 8), &RECT), "DEFINABLE_CODES");
    // The channel context has sixteen of its own, whatever the page has.
    for _ in 0..16 {
        assert!(matches!(symbol_in(&mut host, CHANNEL, 0, (8, 8), &RECT), Reply::Int(_)));
    }
    assert_range(&symbol_in(&mut host, CHANNEL, 0, (8, 8), &RECT), "CHANNEL_SHARE_SYMBOLS");
    // A symbol removed gives its code back, and its handle names nothing after.
    assert_eq!(call(&mut host, Op::NP_REMOVE, &[handles[3]]), Reply::Undefined);
    let again = symbol(&mut host, &RECT);
    assert_ne!(again, handles[3]);
    assert_type(&symbol_in(&mut host, PAGE, handles[3], (8, 8), &RECT));
    assert_type(&call(&mut host, Op::NP_REMOVE, &[handles[3]]));
    // One context's symbol is not the other's.
    assert_type(&symbol_in(&mut host, CHANNEL, handles[0], (8, 8), &RECT));
    // Refusals: a size past the canvas, a WAIT, a program with a fault.
    assert_range(&symbol_in(&mut host, PAGE, handles[0], (481, 8), &RECT), "CANVAS_WIDTH");
    assert_range(&symbol_in(&mut host, PAGE, handles[0], (8, 0), &RECT), "CANVAS_HEIGHT");
    assert_type(&symbol_in(&mut host, PAGE, handles[0], (8, 8), &[Paint::WAIT as i32, 1]));
    assert_type(&symbol_in(&mut host, PAGE, handles[0], (8, 8), &[Paint::STROKE as i32, 9]));
}

/// Section 7.4, `drcs` and `texture`: DEF DRCS and DEF TEXTURE, 96
/// characters and four patterns (`DEFINABLE_CODES`, `TEXTURE_MASKS`),
/// their contents bounded each and together.
#[test]
fn drcs_characters_and_textures_are_held_to_their_tables() {
    let mut host = host();
    let drcs = |host: &mut Host, program: &[i32]| host.call(PAGE, Op::NP_DRCS.number(), &[], Payload::Ints(&words(program)));
    let texture =
        |host: &mut Host, size: (i32, i32), program: &[i32]| host.call(PAGE, Op::NP_TEXTURE.number(), &[size.0, size.1], Payload::Ints(&words(program)));
    assert_eq!(drcs(&mut host, &RECT), Reply::Int(KIND_DRCS << HANDLE_KIND_SHIFT));
    let (_, bytes) = staged(&mut host).unwrap();
    // TEXT with the cell's field goes before DEF DRCS (ESC 4/3) of code 2/0.
    let define = bytes.windows(3).position(|window| window == [0x1B, 0x43, 0x20]).expect("DEF DRCS");
    assert!(bytes[..define].contains(&0x22));
    assert_eq!(bytes[bytes.len() - 2..], END_DEFINITION);
    for _ in 1..96 {
        assert!(matches!(drcs(&mut host, &RECT), Reply::Int(_)));
    }
    assert_range(&drcs(&mut host, &RECT), "DEFINABLE_CODES");
    for pattern in 0..4 {
        assert_eq!(texture(&mut host, (12, 6), &RECT), Reply::Int((KIND_TEXTURE << HANDLE_KIND_SHIFT) | pattern));
    }
    assert_range(&texture(&mut host, (12, 6), &RECT), "TEXTURE_MASKS");
    assert_type(&host.call(CHANNEL, Op::NP_DRCS.number(), &[], Payload::Ints(&words(&RECT))));
    // The mask's side at its bound and one past.
    let mut host = self::host();
    assert!(matches!(texture(&mut host, (128, 128), &RECT), Reply::Int(_)));
    assert_range(&texture(&mut host, (129, 1), &RECT), "MASK_SIDE");
    assert_range(&texture(&mut host, (1, 0), &RECT), "MASK_SIDE");
    // DEF TEXTURE (ESC 4/4) of pattern A (4/1), after TEXTURE with the mask's size.
    let (_, bytes) = staged(&mut host).unwrap();
    let define = bytes.windows(3).position(|window| window == [0x1B, 0x44, 0x41]).expect("DEF TEXTURE");
    assert!(bytes[..define].contains(&0x23));
    // Each definition's contents and all of them together: eight of the longest, and no more.
    assert_range(&drcs(&mut host, &stepped(681)), "MAX_MACRO_BYTES");
    for _ in 0..7 {
        assert!(matches!(drcs(&mut host, &stepped(679)), Reply::Int(_)));
    }
    assert_range(&drcs(&mut host, &stepped(679)), "NAPLPS_MAX_PATTERN_BYTES");
}

/// Section 7.5 and 10.6, `MAX_OBJECTS`, `CHANNEL_SHARE_ACTORS` and
/// `MAX_OBJECT_FRAMES`, each at its bound and one past; the objects leave
/// as the engine's record: one alone by its `ONE` flag, or all of them.
#[test]
fn the_cast_is_held_to_sixty_four_objects() {
    let mut host = host();
    let box_symbol = symbol(&mut host, &RECT);
    let tall = match symbol_in(&mut host, PAGE, 0, (8, 30), &RECT) {
        Reply::Int(handle) => handle,
        other => panic!("{other:?}"),
    };
    staged(&mut host);
    // One actor made in a step is sent alone.
    let first = actor(&mut host, 100, 50, &[box_symbol]);
    assert_eq!(first, (KIND_ACTOR << HANDLE_KIND_SHIFT) | 1);
    let record = host.take_batch(0, &[]).naplps_objects.expect("the object");
    assert!(record.one && record.title.is_empty());
    let object = &record.objects[0];
    // Canvas line 50 is raster line 70; a symbol 8 lines tall, run from
    // the unit screen's origin, has its top left on raster line 472.
    assert_eq!(
        (object.number, object.x, object.y, object.width, object.height, object.origin, object.layer, object.shown),
        (1, 100, 70, 8, 8, (0, 472), 2, true)
    );
    assert_eq!((object.frames.clone(), object.looks, object.kind), (vec![0x20], [0; 3], NaplpsObjectKind::Macro));
    // Sixteen frames, and no seventeenth; frames of one height.
    assert!(matches!(actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 1), &[box_symbol; 16]), Reply::Int(_)));
    assert_range(&actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 1), &[box_symbol; 17]), "MAX_OBJECT_FRAMES");
    assert_type(&actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 1), &[box_symbol, tall]));
    assert_type(&actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 1), &[]));
    assert_type(&actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 1), &[box_symbol + 256]));
    // Fifty-six objects are the page's, and no fifty-seventh.
    for index in 2..56 {
        actor(&mut host, index, index, &[box_symbol]);
    }
    assert_range(&actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 1), &[box_symbol]), "MAX_OBJECTS");
    // The channel context has eight of its own, in the overlay only, drawn with its own symbols.
    let Reply::Int(channel_symbol) = symbol_in(&mut host, CHANNEL, 0, (8, 8), &RECT) else { panic!("the channel context's symbol") };
    assert_type(&actor_in(&mut host, CHANNEL, actor_ints(0, 0, 0, 2, 1), &[channel_symbol]));
    assert_type(&actor_in(&mut host, CHANNEL, actor_ints(0, 0, 0, 3, 1), &[box_symbol]));
    let mut channel_actor = 0;
    for _ in 0..8 {
        let Reply::Int(handle) = actor_in(&mut host, CHANNEL, actor_ints(0, 0, 0, 3, 1), &[channel_symbol]) else { panic!("the channel context's actor") };
        channel_actor = handle;
    }
    assert_range(&actor_in(&mut host, CHANNEL, actor_ints(0, 0, 0, 3, 1), &[channel_symbol]), "CHANNEL_SHARE_ACTORS");
    assert_type(&host.call(CHANNEL, Op::NP_ANIMATE.number(), &[channel_actor, 16, 2, 0, 0, 0], Payload::None));
    assert_type(&host.call(CHANNEL, Op::NP_ANIMATE.number(), &[first, 5, 0, 0, 0, 0], Payload::None));
    // Many made in one step: every object is sent, sixty-four of them, with the page's title.
    host.call(PAGE, Op::TITLE.number(), &[], Payload::Text(b"Cast"));
    let record = host.take_batch(0, &[]).naplps_objects.expect("the objects");
    assert!(!record.one);
    assert_eq!((record.objects.len(), record.title.as_str()), (64, "Cast"));
    // The channel context's go with it; the page's stay.
    host.close(ContextId::Channel);
    assert_eq!(host.take_batch(0, &[]).naplps_objects.map(|record| record.objects.len()), Some(56));
    // A region: a rectangle and a cover, no frames.
    let mut host = self::host();
    let region = |ints: [i32; 15]| ints;
    let made = host.call(PAGE, Op::NP_ACTOR.number(), &region([0, 2, 10, 10, 20, 20, 0, 0, 1, -3, 3, 3, 0, 0, 0]), Payload::None);
    assert_eq!(made, Reply::Int((KIND_ACTOR << HANDLE_KIND_SHIFT) | 1));
    let record = host.take_batch(0, &[]).naplps_objects.unwrap();
    let object = &record.objects[0];
    assert_eq!(
        (object.kind, object.cover, object.order, object.origin, object.detectable, object.frames.len()),
        (NaplpsObjectKind::Region, 3, -3, (0, 0), true, 0)
    );
    assert_range(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 0, 20, 0, 0, 1, 0, 3, 3, 0, 0, 0], Payload::None), "CANVAS_WIDTH");
    assert_range(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 20, 461, 0, 0, 1, 0, 3, 3, 0, 0, 0], Payload::None), "CANVAS_HEIGHT");
    assert_range(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 20, 20, 0, 0, 0, 0, 3, 3, 0, 0, 0], Payload::None), "LAYERS");
    assert_range(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 20, 20, 0, 0, 4, 0, 3, 3, 0, 0, 0], Payload::None), "LAYERS");
    assert_range(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 20, 20, 0, 0, 1, 128, 3, 3, 0, 0, 0], Payload::None), "MAX_ORDER");
    assert_range(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 20, 20, 0, 0, 1, 0, 3, 16, 0, 0, 0], Payload::None), "NAPLPS_COLOURS");
    assert_range(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 40_000, 10, 20, 20, 0, 0, 1, 0, 3, 3, 0, 0, 0], Payload::None), "MAX_X");
    assert_type(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 3, 10, 10, 20, 20, 0, 0, 1, 0, 3, 3, 0, 0, 0], Payload::None));
    assert_type(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 20, 20, 0, 0, 1, 0, 8, 3, 0, 0, 0], Payload::None));
    assert_type(&host.call(PAGE, Op::NP_ACTOR.number(), &[0, 2, 10, 10, 20, 20], Payload::None));
}

/// Section 7.5: an actor's moves, shows, order, frames, cycles and looks
/// are `ANIMATE` operations (animation.md ANIM-6), bounded a step
/// (`MAX_ANIMATE_OPERATIONS`) and in how many objects cycle
/// (`MAX_CYCLING_OBJECTS`); a redefinition sends the one object again
/// with its state, and a removal every object.
#[test]
#[expect(clippy::too_many_lines, reason = "every animation operation, each bound at its edge, and what the batches carry")]
fn actors_are_animated_by_operations_and_redefined_alone() {
    let mut host = host();
    let box_symbol = symbol(&mut host, &RECT);
    let handles: Vec<i32> = (0..17).map(|index| actor(&mut host, 10 * index, 100, &[box_symbol, box_symbol])).collect();
    host.take_batch(0, &[]);
    let animate = |host: &mut Host, handle: i32, ints: [i32; 5]| {
        host.call(PAGE, Op::NP_ANIMATE.number(), &[handle, ints[0], ints[1], ints[2], ints[3], ints[4]], Payload::None)
    };
    let first = handles[0];
    for ints in [
        [15, 0, 30, 40, 0],
        [15, 1, 5, -5, 0],
        [6, 0, 0, 0, 0],
        [5, 0, 0, 0, 0],
        [16, 3, 7, 0, 0],
        [16, 255, -7, 0, 0],
        [17, 1, 0, 0, 0],
        [7, 2, 0, 0, 0],
        [18, 0, 1, 15, 1],
        [19, 0, 0, 0, 0],
    ] {
        assert_eq!(animate(&mut host, first, ints), Reply::Undefined, "{ints:?}");
    }
    let target = AnimateTarget::object(1);
    let batch = host.take_batch(0, &[]);
    assert_eq!(batch.naplps_objects, None, "animation sends no record");
    assert_eq!(
        batch.animate,
        Some(
            [
                // A place is written in the record's lines: canvas line 40 is raster line 60.
                OperationKind::Move { relative: false, x: 30, y: 60 },
                OperationKind::Move { relative: true, x: 5, y: -5 },
                OperationKind::Hide,
                OperationKind::Show,
                OperationKind::Order { layer: 3, order: 7 },
                OperationKind::Order { layer: 255, order: -7 },
                OperationKind::Frame(1),
                OperationKind::Look(2),
                OperationKind::Cycle { first: 0, last: 1, period: 15, once: true },
                OperationKind::StopCycle,
            ]
            .into_iter()
            .map(|kind| Operation { target, kind })
            .collect()
        )
    );
    // Refusals: each bound by its name, and nothing staged for them.
    assert_range(&animate(&mut host, first, [17, 16, 0, 0, 0]), "MAX_FRAME");
    assert_range(&animate(&mut host, first, [18, 0, 1, 3_601, 0]), "MAX_CYCLE_PERIOD");
    assert_range(&animate(&mut host, first, [18, 0, 1, 0, 0]), "MAX_CYCLE_PERIOD");
    assert_range(&animate(&mut host, first, [16, 0, 0, 0, 0]), "LAYERS");
    assert_range(&animate(&mut host, first, [7, 4, 0, 0, 0]), "LOOKS");
    assert_range(&animate(&mut host, first, [15, 0, 40_000, 0, 0]), "MAX_X");
    assert_type(&animate(&mut host, first, [18, 1, 0, 15, 0]));
    assert_type(&animate(&mut host, first, [20, 0, 0, 0, 0]));
    assert_type(&animate(&mut host, first + 100, [5, 0, 0, 0, 0]));
    assert_eq!(host.take_batch(0, &[]).animate, None);
    // Sixteen objects cycle, and no seventeenth; one of them may start again.
    for handle in &handles[..16] {
        assert_eq!(animate(&mut host, *handle, [18, 0, 1, 30, 0]), Reply::Undefined);
    }
    assert_range(&animate(&mut host, handles[16], [18, 0, 1, 30, 0]), "MAX_CYCLING_OBJECTS");
    assert_eq!(animate(&mut host, handles[3], [18, 1, 1, 9, 0]), Reply::Undefined);
    // Sixty-four operations a step, and no sixty-fifth.
    for _ in 17..64 {
        assert_eq!(animate(&mut host, first, [15, 1, 1, 0, 0]), Reply::Undefined);
    }
    assert_range(&animate(&mut host, first, [15, 1, 1, 0, 0]), "MAX_ANIMATE_OPERATIONS");
    assert_eq!(host.take_batch(0, &[]).animate.map(|operations| operations.len()), Some(64));
    // Redefined, an actor is sent alone where it stands, its frame, cycle and look forgotten.
    assert_eq!(actor_in(&mut host, PAGE, actor_ints(first, 300, 200, 1, 0), &[box_symbol]), Reply::Int(first));
    let batch = host.take_batch(0, &[]);
    let record = batch.naplps_objects.expect("the one object");
    assert!(record.one);
    assert_eq!(
        (record.objects[0].number, record.objects[0].x, record.objects[0].y, record.objects[0].shown, record.objects[0].frames.len()),
        (1, 300, 220, false, 1)
    );
    assert_eq!(batch.animate, None);
    // A label is carried by the record.
    assert_eq!(host.call(PAGE, Op::NP_LABEL.number(), &[handles[1]], Payload::Text("Kite".as_bytes())), Reply::Undefined);
    assert_range(&host.call(PAGE, Op::NP_LABEL.number(), &[handles[1]], Payload::Text(&[b'x'; 65])), "MAX_LABEL_BYTES");
    assert_type(&host.call(PAGE, Op::NP_LABEL.number(), &[handles[1]], Payload::Text(b"a\x07")));
    let record = host.take_batch(0, &[]).naplps_objects.unwrap();
    assert_eq!((record.one, record.objects[0].label.as_str()), (true, "Kite"));
    // Removed, every object is sent, and those that cycle are told to again.
    assert_eq!(call(&mut host, Op::NP_REMOVE, &[handles[16]]), Reply::Undefined);
    assert_type(&call(&mut host, Op::NP_REMOVE, &[handles[16]]));
    let batch = host.take_batch(0, &[]);
    assert_eq!(batch.naplps_objects.map(|record| (record.one, record.objects.len())), Some((false, 16)));
    let cycles = batch.animate.unwrap();
    assert_eq!(cycles.len(), 15, "the fifteen that still cycle: the one redefined does not");
    assert!(cycles.contains(&Operation { target: AnimateTarget::object(4), kind: OperationKind::Cycle { first: 1, last: 1, period: 9, once: false } }));
    // A symbol an actor draws with is not removed, and keeps its height.
    assert_type(&call(&mut host, Op::NP_REMOVE, &[box_symbol]));
    assert_type(&symbol_in(&mut host, PAGE, box_symbol, (8, 9), &RECT));
    // The wheel: the things that take it are told to the decoder, sixteen at most.
    let mut host = self::host();
    let box_symbol = symbol(&mut host, &RECT);
    let wheels: Vec<i32> = (0..16)
        .map(|_| match actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 5), &[box_symbol]) {
            Reply::Int(handle) => handle,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_range(&actor_in(&mut host, PAGE, actor_ints(0, 0, 0, 2, 5), &[box_symbol]), "MAX_WHEEL_THINGS");
    assert_eq!(host.take_batch(0, &[]).watch.and_then(|watch| watch.wheel), Some(wheels.clone()));
    // An actor is a thing to listen to; removed, it is not, and its listeners are forgotten.
    assert_eq!(call(&mut host, Op::LISTEN, &[wheels[0], EventKind::POINTER.number(), 1]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::NP_REMOVE, &[wheels[0]]), Reply::Undefined);
    assert!(!host.listens_to(ContextId::Page, wheels[0], EventKind::POINTER));
    assert_type(&call(&mut host, Op::LISTEN, &[wheels[0], EventKind::POINTER.number(), 1]));
}

/// Section 7.2: a new generation deletes the receiver's definitions, so
/// the host sends every symbol, DRCS character and texture again, then
/// the cast, each object where it stands with its cycle; the opening is
/// the host's own and is not charged to the callback's allowance. And
/// section 10.1 rule 7: what is kept for all this is counted.
#[test]
fn a_repaint_sends_the_definitions_and_the_cast_again() {
    let mut host = host();
    let box_symbol = symbol(&mut host, &stepped(679));
    host.call(PAGE, Op::NP_DRCS.number(), &[], Payload::Ints(&words(&RECT)));
    host.call(PAGE, Op::NP_TEXTURE.number(), &[12, 6], Payload::Ints(&words(&RECT)));
    let handle = actor(&mut host, 100, 50, &[box_symbol]);
    host.call(PAGE, Op::NP_ANIMATE.number(), &[handle, 15, 1, 7, 7, 0], Payload::None);
    host.call(PAGE, Op::NP_ANIMATE.number(), &[handle, 18, 0, 0, 20, 0], Payload::None);
    host.take_batch(0, &[]);
    let held = host.state_bytes();
    assert!(held > 4_093 && host.naplps.cast.state_bytes() > 4_093, "the symbol's contents are counted");
    at(&mut host, 30);
    // A painting that takes the whole of a callback's allowance still fits beside the opening.
    assert_eq!(paint(&mut host, Op::NP_REPAINT, &long(10)), Reply::Undefined);
    let batch = host.take_batch(0, &[]);
    let append = batch.naplps.expect("the new generation");
    assert!(append.new_generation && append.bytes.len() > 61_000 + 4_093);
    for definition in [[0x1B, 0x40, 0x20], [0x1B, 0x43, 0x20], [0x1B, 0x44, 0x41]] {
        assert_eq!(append.bytes.windows(3).filter(|window| *window == definition).count(), 1, "{definition:02X?}");
    }
    let record = batch.naplps_objects.expect("the cast again");
    assert!(!record.one);
    assert_eq!((record.objects[0].x, record.objects[0].y), (107, 77));
    assert_eq!(
        batch.animate,
        Some(vec![Operation { target: AnimateTarget::object(1), kind: OperationKind::Cycle { first: 0, last: 0, period: 20, once: false } }])
    );
    // The host's bound on what it holds is met by a definition too.
    host.set_limits_for_test(host.state_bytes() + 1_000, usize::MAX);
    assert_range(&symbol_in(&mut host, PAGE, 0, (8, 8), &stepped(679)), "MAX_HOST_STATE_BYTES");
    // A scope's actors and symbols go with it.
    let mut host = self::host();
    assert!(matches!(call(&mut host, Op::SCOPE_BEGIN, &[]), Reply::Int(1)));
    let scoped = symbol(&mut host, &RECT);
    let in_scope = actor(&mut host, 1, 1, &[scoped]);
    call(&mut host, Op::SCOPE_LEAVE, &[1]);
    let kept = symbol(&mut host, &RECT);
    host.take_batch(0, &[]);
    assert_eq!(call(&mut host, Op::SCOPE_END, &[1]), Reply::Undefined);
    assert_eq!(host.take_batch(0, &[]).naplps_objects.map(|record| record.objects.len()), Some(0));
    assert_type(&host.call(PAGE, Op::NP_ANIMATE.number(), &[in_scope, 5, 0, 0, 0, 0], Payload::None));
    assert_type(&symbol_in(&mut host, PAGE, scoped, (8, 8), &RECT));
    assert_eq!(symbol_in(&mut host, PAGE, kept, (8, 8), &RECT), Reply::Int(kept));
}
