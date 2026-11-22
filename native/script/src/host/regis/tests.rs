//! The ReGIS operations at their bounds (script-api.md sections 6.1, 6.2,
//! 10.1, 10.2 and 10.6; work package R1): what each stages, that each
//! refusal names its bound and changes nothing, that every commit error
//! is caught here and nothing is sent, and that what the host counts as
//! held is never less than what is held (TODO-7272).
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::expect_used, reason = "tests")]

use protocol::script::{ContextFlags, ContextOpen, Event, Level, StepFlags};
use regis::{ForcedLook, Picture, Screen, Terminal};

use super::*;
use crate::limits::regis::{REGIS_MAX_CONTENTS_BYTES, REGIS_MAX_DRAWINGS};
use crate::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES};
use crate::ops::RegisPaint;
use crate::random::SEED_BYTES;

const PAGE: i32 = 2;
const CHANNEL: i32 = 1;

fn opening(context: ContextId, renderer: Renderer) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer,
        level: Level::TwoFive,
        page: 0x1A0,
        subcode: 0,
        flags: ContextFlags { pointer_press: true, ..ContextFlags::default() },
        tick_divisor: 1,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/1A0.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [9; SEED_BYTES],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: 1,
    }
}

/// A host with both contexts open on a ReGIS page, in a step.
fn host() -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, Renderer::Regis), 100);
    host.open(opening(ContextId::Page, Renderer::Regis), 200);
    host.begin_step(0, true);
    host
}

fn call(host: &mut Host, op: Op, ints: &[i32]) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::None)
}

fn words(numbers: &[i32]) -> Vec<u8> {
    numbers.iter().flat_map(|number| number.to_le_bytes()).collect()
}

fn call_ints(host: &mut Host, op: Op, ints: &[i32], payload: &[i32]) -> Reply {
    let bytes = words(payload);
    host.call(PAGE, op.number(), ints, Payload::Ints(&bytes))
}

/// A drawing with default attributes; its handle.
fn drawing(host: &mut Host) -> i32 {
    made(call(host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 1]))
}

fn made(reply: Reply) -> i32 {
    match reply {
        Reply::Int(handle) => handle,
        other => panic!("no drawing was made: {other:?}"),
    }
}

fn set(host: &mut Host, handle: i32, property: Property, value: i32) -> Reply {
    call(host, Op::RG_SET, &[handle, property.number(), value])
}

fn get(host: &mut Host, handle: i32, field: i32) -> Reply {
    call(host, Op::RG_GET, &[handle, field])
}

/// What a painter program writes as a drawing's contents, in English,
/// with no macrograph to call.
fn encode_plain(program: &[i32]) -> Result<Vec<u8>, Reply> {
    painter::encode(program, &Setting { language: "en", letter: &|_| Err(type_error("not a macrograph")), macrograph: false })
}

/// A filled rectangle as a painter program.
fn filled(x: i32, y: i32, width: i32, height: i32, ink: i32) -> Vec<i32> {
    vec![RegisPaint::INK.number(), ink, RegisPaint::FILL.number(), 5, RegisPaint::RECT.number(), x, y, width, height]
}

/// The step's ReGIS as its batch gives it, as text; empty for none.
fn sent(host: &mut Host) -> String {
    let batch = host.take_batch(1, &[]);
    String::from_utf8_lossy(&batch.regis.unwrap_or_default()).into_owned()
}

/// Everything a script could tell of the ReGIS state, and what is held.
fn fingerprint(host: &mut Host) -> String {
    let page = host.budget(ContextId::Page);
    let channel = host.budget(ContextId::Channel);
    let regis = &host.regis;
    format!(
        "{page}{channel}{}:{}:{:?}:{:?}:{:?}:{:?}:{:?}",
        host.state_bytes(),
        host.batch_bytes(),
        regis.drawings,
        regis.out,
        regis.animate,
        regis.colours,
        regis.bounds
    )
}

#[track_caller]
fn assert_range(reply: &Reply, bound: &str) {
    match reply {
        Reply::Range(message) => assert!(message.starts_with(&format!("{bound}: ")), "the RangeError does not begin with {bound}: {message}"),
        other => panic!("expected a RangeError naming {bound}, got {other:?}"),
    }
}

/// Section 10.1 rule 3: a call that would pass a bound throws a
/// `RangeError` naming it and changes nothing.
#[track_caller]
fn refuses(host: &mut Host, bound: &str, refused: impl FnOnce(&mut Host) -> Reply) {
    let before = fingerprint(host);
    let reply = refused(host);
    assert_range(&reply, bound);
    assert_eq!(fingerprint(host), before, "a refused call changed the host");
}

#[track_caller]
fn mistypes(host: &mut Host, refused: impl FnOnce(&mut Host) -> Reply) {
    let before = fingerprint(host);
    let reply = refused(host);
    assert!(matches!(reply, Reply::Type(_)), "expected a TypeError, got {reply:?}");
    assert_eq!(fingerprint(host), before, "a refused call changed the host");
}

/// A receiver fed what the host sent: every batch commits there.
struct Receiver {
    terminal: Terminal,
    screen: Screen,
}

impl Receiver {
    fn new() -> Self {
        Self { terminal: Terminal::new(), screen: Screen::new() }
    }

    /// Feeds a batch's ReGIS and applies its animation operations;
    /// whether a commit was applied.
    fn take(&mut self, batch: &Batch) -> bool {
        let mut committed = false;
        if let Some(bytes) = &batch.regis {
            let events = self.terminal.feed_regis(&mut self.screen, bytes);
            assert!(!events.iter().any(|event| matches!(event, RegisEvent::CommitFailed { .. })), "the receiver refused what the host sent: {events:?}");
            committed = events.iter().any(|event| matches!(event, RegisEvent::Committed(_)));
        }
        for operation in batch.animate.iter().flatten() {
            let mut state = self.terminal.animation(operation.target.number).unwrap_or_default();
            match operation.kind {
                OperationKind::Frame(frame) | OperationKind::Cycle { first: frame, .. } => state.frame = frame,
                OperationKind::Look(look) => {
                    state.look = [ForcedLook::None, ForcedLook::Hover, ForcedLook::Pressed, ForcedLook::Alternative][usize::from(look)];
                }
                _ => {}
            }
            assert!(self.terminal.set_animation(operation.target.number, state));
        }
        committed
    }

    fn picture(&mut self) -> Picture {
        let mut picture = Picture::new();
        self.terminal.compose(&self.screen, &mut picture);
        picture
    }

    fn pixel(&mut self, x: u32, y: u32) -> u8 {
        self.picture().canvas().pixel(x, y).unwrap()
    }
}

/// Section 6.2: every option of `drawing` stages the format's own
/// attribute in one `D(S n(…))`, only what differs from a new drawing's
/// defaults; each later assignment stages its own; the step ends with one
/// `D(C)`; and a receiver commits it.
#[test]
fn a_drawing_stages_its_fourteen_attributes_and_one_commit() {
    let mut host = host();
    let plain = drawing(&mut host);
    assert_eq!(plain, 1, "the page context's first drawing is number 1 of generation 0");
    // A hidden master, to be drawn in others' places.
    let master = made(call(&mut host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 0]));
    let second = made(call(&mut host, Op::RG_DRAWING, &[plain, master, 2, -5, 7, -3, 20, FLAG_VISIBLE | FLAG_DETECTABLE]));
    assert_eq!((master, second), (2, 3));
    assert_eq!(set(&mut host, second, Property::HOVER, master), Reply::Undefined);
    assert_eq!(set(&mut host, second, Property::PRESSED, master), Reply::Undefined);
    assert_eq!(set(&mut host, second, Property::ALTERNATIVE, master), Reply::Undefined);
    assert_eq!(call(&mut host, Op::RG_CLIP, &[second, 1, 1, 2, 30, 40]), Reply::Undefined);
    let fourth = made(call(&mut host, Op::RG_DRAWING, &[0, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(call_ints(&mut host, Op::RG_FRAMES, &[second], &[master, fourth]), Reply::Undefined);
    assert_eq!(host.call(PAGE, Op::RG_LABEL.number(), &[second], Payload::Text("It's caf\u{e9}".as_bytes())), Reply::Undefined);
    // What was staged reads back, each as the property's own type.
    for (field, want) in [
        (Property::X.number(), Reply::Int(-5)),
        (Property::Y.number(), Reply::Int(7)),
        (Property::DEPTH.number(), Reply::Int(-3)),
        (Property::LAYER.number(), Reply::Int(2)),
        (Property::VISIBLE.number(), Reply::Int(1)),
        (Property::DETECTABLE.number(), Reply::Int(1)),
        (Property::PARENT.number(), Reply::Int(plain)),
        (Property::SOURCE.number(), Reply::Int(master)),
        (Property::HOVER.number(), Reply::Int(master)),
        (Property::PRESSED.number(), Reply::Int(master)),
        (Property::ALTERNATIVE.number(), Reply::Int(master)),
        (Property::PAINT_RATE.number(), Reply::Int(20)),
        (Property::WHEEL.number(), Reply::Int(0)),
        (Property::TRACK.number(), Reply::Int(0)),
        (RegisField::CLIP.number(), Reply::Text("1,2,30,40".into())),
        (RegisField::FRAMES.number(), Reply::Text("2,4".into())),
        (RegisField::LABEL.number(), Reply::Text("It's caf\u{e9}".into())),
    ] {
        assert_eq!(get(&mut host, second, field), want, "field {field}");
    }
    assert_eq!(get(&mut host, plain, Property::PAINT_RATE.number()), Reply::Int(PAINT_RATE_UNSET), "a new drawing's rate is unset (DEC-7031 item 2)");
    assert_eq!(get(&mut host, fourth, Property::PAINT_RATE.number()), Reply::Int(0), "a rate of 0, at once, is not unset");
    // Later assignments: a value the drawing already has stages nothing.
    assert_eq!(set(&mut host, plain, Property::X, 0), Reply::Undefined);
    assert_eq!(set(&mut host, plain, Property::X, 240), Reply::Undefined);
    assert_eq!(set(&mut host, plain, Property::Y, 9), Reply::Undefined);
    assert_eq!(set(&mut host, plain, Property::DETECTABLE, 1), Reply::Undefined);
    let batch = host.take_batch(1, &[]);
    let bytes = batch.regis.clone().unwrap();
    // The label's e-acute is one byte of ISO 8859-1, its quote doubled.
    let mut want = b"D(S1(X0))D(S2(V0))D(S3(P1,R2,G2,X-5,Y7,L-3,T20,I1,H2,A2,N2,C(X1,Y2,W30,H40)))D(S4(G0,T0,V0))D(S3(F(2,4),U'It''s caf".to_vec();
    want.push(0xE9);
    want.extend_from_slice(b"'))D(S1(X240,Y9,I1))D(C)");
    assert_eq!(String::from_utf8_lossy(&bytes), String::from_utf8_lossy(&want));
    let mut receiver = Receiver::new();
    assert!(receiver.take(&batch), "the receiver commits the step");
    assert!(receiver.terminal.committed_drawing(3).is_some());
    assert_eq!(receiver.terminal.label(3).as_deref(), Some("It's caf\u{e9}"));
    assert_eq!(receiver.terminal.parent(3), Some(1));
    assert_eq!(receiver.terminal.drawing_position(1), Some((240, 9)));
    // A step that staged nothing sends nothing.
    host.begin_step(1, true);
    assert_eq!(sent(&mut host), "");
}

/// Section 6.2's ranges and section 10.6's bounds: each refusal names its
/// bound, or is a `TypeError`, and changes nothing.
#[test]
fn each_attribute_is_held_to_its_range() {
    let mut host = host();
    let one = drawing(&mut host);
    for property in [Property::X, Property::Y, Property::DEPTH] {
        refuses(&mut host, "MAX_COORDINATE", |host| set(host, one, property, 32_768));
        refuses(&mut host, "MAX_COORDINATE", |host| set(host, one, property, -32_769));
        assert_eq!(set(&mut host, one, property, -32_768), Reply::Undefined);
    }
    refuses(&mut host, "LAYERS", |host| set(host, one, Property::LAYER, 4));
    refuses(&mut host, "LAYERS", |host| call(host, Op::RG_DRAWING, &[0, 0, -1, 0, 0, 0, -1, 1]));
    refuses(&mut host, "MAX_PAINT_RATE", |host| set(host, one, Property::PAINT_RATE, 32_768));
    refuses(&mut host, "MAX_PAINT_RATE", |host| call(host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -2, 1]));
    refuses(&mut host, "MAX_COORDINATE", |host| call(host, Op::RG_DRAWING, &[0, 0, 1, 40_000, 0, 0, -1, 1]));
    refuses(&mut host, "MAX_COORDINATE", |host| call(host, Op::RG_CLIP, &[one, 1, 0, 0, 40_000, 1]));
    mistypes(&mut host, |host| call(host, Op::RG_CLIP, &[one, 1, 0, 0, -1, 1]));
    mistypes(&mut host, |host| set(host, one, Property::VISIBLE, 2));
    mistypes(&mut host, |host| call(host, Op::RG_SET, &[one, 99, 0]));
    mistypes(&mut host, |host| call(host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 16]));
    mistypes(&mut host, |host| call(host, Op::RG_DRAWING, &[0, 0, 1]));
    // A paint rate once given cannot be unset: ReGIS has no operation
    // for it (regis-v2.md, "Paint rate").
    assert_eq!(set(&mut host, one, Property::PAINT_RATE, -1), Reply::Undefined);
    assert_eq!(set(&mut host, one, Property::PAINT_RATE, 0), Reply::Undefined);
    mistypes(&mut host, |host| set(host, one, Property::PAINT_RATE, -1));
    // A label: at most 64 of ISO 8859-1's printable characters.
    let long = "x".repeat(MAX_LABEL_BYTES + 1);
    refuses(&mut host, "MAX_LABEL_BYTES", |host| host.call(PAGE, Op::RG_LABEL.number(), &[one], Payload::Text(long.as_bytes())));
    mistypes(&mut host, |host| host.call(PAGE, Op::RG_LABEL.number(), &[one], Payload::Text("tab\there".as_bytes())));
    mistypes(&mut host, |host| host.call(PAGE, Op::RG_LABEL.number(), &[one], Payload::Text("\u{142}".as_bytes())));
    assert_eq!(host.call(PAGE, Op::RG_LABEL.number(), &[one], Payload::Text("x".repeat(MAX_LABEL_BYTES).as_bytes())), Reply::Undefined);
    assert_eq!(host.call(PAGE, Op::RG_LABEL.number(), &[one], Payload::None), Reply::Undefined);
    assert_eq!(get(&mut host, one, RegisField::LABEL.number()), Reply::Undefined);
    // A frame list: 1 to 16 drawings.
    let frames: Vec<i32> = (0..=MAX_OBJECT_FRAMES).map(|_| one).collect();
    refuses(&mut host, "MAX_OBJECT_FRAMES", |host| call_ints(host, Op::RG_FRAMES, &[one], &frames));
    mistypes(&mut host, |host| call_ints(host, Op::RG_FRAMES, &[one], &[one, 0]));
    assert_eq!(call_ints(&mut host, Op::RG_FRAMES, &[one], &frames[..MAX_OBJECT_FRAMES]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::RG_FRAMES, &[one]), Reply::Undefined);
    assert!(sent(&mut host).ends_with(",U'',F(1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1),F0))D(C)"));
}

/// Section 10.1 rule 4: a handle is a checked index of its context's own
/// table; a stale, foreign or forged one is a `TypeError`.
#[test]
fn a_stale_foreign_or_forged_handle_is_a_type_error() {
    let mut host = host();
    let one = drawing(&mut host);
    let child = made(call(&mut host, Op::RG_DRAWING, &[one, 0, 1, 0, 0, 0, -1, 1]));
    let theirs = made(host.call(CHANNEL, Op::RG_DRAWING.number(), &[0, 0, 3, 0, 0, 0, -1, 1], Payload::None));
    assert_eq!(theirs, 32_768, "the channel context's numbers begin above the page context's");
    for forged in [0, -1, 77, 65_536, i32::MAX, i32::MIN, theirs] {
        mistypes(&mut host, |host| set(host, forged, Property::X, 1));
        mistypes(&mut host, |host| call(host, Op::RG_REMOVE, &[forged]));
    }
    mistypes(&mut host, |host| set(host, one, Property::SOURCE, theirs));
    mistypes(&mut host, |host| call(host, Op::RG_DRAWING, &[theirs, 0, 1, 0, 0, 0, -1, 1]));
    mistypes(&mut host, |host| host.call(CHANNEL, Op::RG_SET.number(), &[one, Property::X.number(), 1], Payload::None));
    // Removing a drawing removes its children; both handles are stale.
    assert_eq!(call(&mut host, Op::RG_REMOVE, &[one]), Reply::Text("0:1,2".into()));
    for stale in [one, child] {
        mistypes(&mut host, |host| set(host, stale, Property::X, 1));
        mistypes(&mut host, |host| get(host, stale, Property::X.number()));
    }
    // A number freed in a step names nothing else until the step is over.
    assert_eq!(drawing(&mut host), 3);
    assert_eq!(sent(&mut host), "D(S1(X0))D(S2(P1))D(S32768(G3))D(K1)D(S3(X0))D(C)");
    // A page left takes its handles with it, whatever their numbers.
    host.close(ContextId::Page);
    host.open(opening(ContextId::Page, Renderer::Regis), 200);
    host.begin_step(1, true);
    let again = drawing(&mut host);
    assert_eq!(again, HANDLE_NUMBERS + 1, "a new page's handles are a new generation's");
    mistypes(&mut host, |host| set(host, 3, Property::X, 1));
    mistypes(&mut host, |host| host.call(CHANNEL, Op::RG_SET.number(), &[theirs, Property::X.number(), 1], Payload::None));
}

/// Section 3.1: the ReGIS API is the page's when its renderer is ReGIS,
/// and the channel context draws only in the overlay layer, within its
/// share, and sets no colour.
#[test]
fn the_channel_context_draws_only_in_the_overlay_within_its_share() {
    let mut host = Host::new();
    host.open(opening(ContextId::Page, Renderer::Teletext), 200);
    host.begin_step(0, true);
    assert!(matches!(call(&mut host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 1]), Reply::Type(_)), "a teletext page has no ReGIS drawings");
    let mut host = self::host();
    let channel = |host: &mut Host, op: Op, ints: &[i32]| host.call(CHANNEL, op.number(), ints, Payload::None);
    assert!(matches!(channel(&mut host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 1]), Reply::Type(_)));
    let top = made(channel(&mut host, Op::RG_DRAWING, &[0, 0, 3, 0, 0, 0, -1, 1]));
    // A child is in its top-level ancestor's layer, whatever its own.
    let child = made(channel(&mut host, Op::RG_DRAWING, &[top, 0, 1, 0, 0, 0, -1, 1]));
    assert!(matches!(channel(&mut host, Op::RG_SET, &[top, Property::LAYER.number(), 2]), Reply::Type(_)));
    assert!(matches!(channel(&mut host, Op::RG_SET, &[child, Property::PARENT.number(), 0]), Reply::Type(_)));
    assert!(matches!(channel(&mut host, Op::RG_COLOUR_SET, &[1, 0, 50, 50]), Reply::Type(_)));
    for _ in 2..CHANNEL_SHARE_DRAWINGS {
        made(channel(&mut host, Op::RG_DRAWING, &[0, 0, 3, 0, 0, 0, -1, 1]));
    }
    assert_range(&channel(&mut host, Op::RG_DRAWING, &[0, 0, 3, 0, 0, 0, -1, 1]), "CHANNEL_SHARE_DRAWINGS");
    // The channel's share is not the page's to use, nor the page's the channel's.
    assert_eq!(drawing(&mut host), 1);
    // The page's `clear` leaves the channel's drawings: it deletes its own by number.
    assert_eq!(call(&mut host, Op::RG_CLEAR, &[]), Reply::Text("0:1".into()));
    assert!(sent(&mut host).ends_with("D(S1(X0))D(K1)D(C)"));
    // With no other context's drawings, `clear` is `D(Z)`.
    host.begin_step(1, true);
    host.close(ContextId::Channel);
    host.begin_step(2, true);
    let (one, two) = (drawing(&mut host), drawing(&mut host));
    assert_eq!(call(&mut host, Op::RG_CLEAR, &[]), Reply::Text(format!("0:{},{}", one % HANDLE_NUMBERS, two % HANDLE_NUMBERS)));
    assert!(sent(&mut host).ends_with("D(Z)D(C)"));
}

/// Sections 6.2 and 10.6: at most 2,048 drawings for a page, of which the
/// channel context's share is kept for it; a chain of parents at most 16;
/// tracked, wheel, animated and cycling drawings each to their bound.
#[test]
fn the_tables_are_held_to_their_bounds() {
    let mut host = host();
    let mut handles = Vec::new();
    for at in 0..REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS {
        handles.push(drawing(&mut host));
        // Commit as the table grows, so that one step's output stays small.
        if at % 256 == 255 {
            assert!(host.take_batch(1, &[]).regis.is_some());
            host.begin_step(u64::try_from(at).unwrap(), true);
        }
    }
    refuses(&mut host, "REGIS_MAX_DRAWINGS", |host| call(host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 1]));
    assert!(host.budget(ContextId::Page).contains("\"drawings\":{\"used\":1984,\"limit\":1984}"));
    // A chain of sixteen, and no seventeenth.
    for pair in handles[..MAX_DRAWING_DEPTH].windows(2) {
        assert_eq!(set(&mut host, pair[1], Property::PARENT, pair[0]), Reply::Undefined);
    }
    refuses(&mut host, "MAX_DRAWING_DEPTH", |host| set(host, handles[MAX_DRAWING_DEPTH], Property::PARENT, handles[MAX_DRAWING_DEPTH - 1]));
    mistypes(&mut host, |host| set(host, handles[0], Property::PARENT, handles[3]));
    mistypes(&mut host, |host| set(host, handles[0], Property::PARENT, handles[0]));
    for (property, most, bound) in [(Property::TRACK, MAX_TRACKED_DRAWINGS, "MAX_TRACKED_DRAWINGS"), (Property::WHEEL, MAX_WHEEL_THINGS, "MAX_WHEEL_THINGS")] {
        for handle in &handles[..most] {
            assert_eq!(set(&mut host, *handle, property, 1), Reply::Undefined);
        }
        refuses(&mut host, bound, |host| set(host, handles[most], property, 1));
    }
    let batch = host.take_batch(2, &[]);
    let watch = batch.watch.unwrap();
    assert_eq!(watch.tracked.unwrap(), (1..=64).collect::<Vec<i32>>());
    assert_eq!(watch.wheel.unwrap(), (1..=16).collect::<Vec<i32>>());
    // Animation state: 64 drawings hold it, 16 cycle.
    host.begin_step(3000, true);
    for handle in &handles[..MAX_CYCLING_OBJECTS] {
        assert_eq!(call(&mut host, Op::RG_ANIMATE, &[*handle, i32::from(code::CYCLE), 0, 3, 10, 0]), Reply::Undefined);
    }
    refuses(&mut host, "MAX_CYCLING_OBJECTS", |host| call(host, Op::RG_ANIMATE, &[handles[MAX_CYCLING_OBJECTS], i32::from(code::CYCLE), 0, 3, 10, 0]));
    for handle in &handles[MAX_CYCLING_OBJECTS..MAX_ANIMATED_DRAWINGS] {
        assert_eq!(call(&mut host, Op::RG_ANIMATE, &[*handle, i32::from(code::FRAME), 2, 0, 0, 0]), Reply::Undefined);
    }
    // One step's batch carries 64 animation operations.
    refuses(&mut host, "MAX_ANIMATE_OPERATIONS", |host| call(host, Op::RG_ANIMATE, &[handles[0], i32::from(code::STOP_CYCLE), 0, 0, 0, 0]));
    assert_eq!(host.take_batch(3, &[]).animate.unwrap().len(), MAX_ANIMATE_OPERATIONS);
    host.begin_step(3001, true);
    refuses(&mut host, "MAX_ANIMATED_DRAWINGS", |host| call(host, Op::RG_ANIMATE, &[handles[MAX_ANIMATED_DRAWINGS], i32::from(code::FRAME), 1, 0, 0, 0]));
    assert!(host.budget(ContextId::Page).contains("\"animated\":{\"used\":64,\"limit\":64},\"cycling\":{\"used\":16,\"limit\":16}"));
}

/// Section 6.2: `frame`, `cycle`, `stopCycle` and `look` are `ANIMATE`
/// operations 17, 18, 19 and 7 on the receiver's animation state, each
/// held to its ranges; a `once` cycle the decoder reports ended no longer
/// counts as cycling.
#[test]
fn animation_operations_are_the_receivers_and_held_to_their_ranges() {
    let mut host = host();
    let one = drawing(&mut host);
    let animate = |host: &mut Host, operation: u8, operands: [i32; 4]| {
        call(host, Op::RG_ANIMATE, &[one, i32::from(operation), operands[0], operands[1], operands[2], operands[3]])
    };
    refuses(&mut host, "MAX_OBJECT_FRAMES", |host| animate(host, code::FRAME, [16, 0, 0, 0]));
    refuses(&mut host, "MAX_OBJECT_FRAMES", |host| animate(host, code::CYCLE, [0, 16, 1, 0]));
    refuses(&mut host, "MAX_CYCLE_PERIOD", |host| animate(host, code::CYCLE, [0, 3, 0, 0]));
    refuses(&mut host, "MAX_CYCLE_PERIOD", |host| animate(host, code::CYCLE, [0, 3, 3601, 0]));
    mistypes(&mut host, |host| animate(host, code::CYCLE, [3, 2, 10, 0]));
    mistypes(&mut host, |host| animate(host, code::CYCLE, [0, 3, 10, 2]));
    mistypes(&mut host, |host| animate(host, code::LOOK, [4, 0, 0, 0]));
    // Show, hide, move, order and reset are not a ReGIS drawing's: its
    // attributes are staged in the stream.
    for other in [code::SHOW, code::HIDE, code::MOVE, code::ORDER, code::RESET, 0, 99] {
        mistypes(&mut host, |host| animate(host, other, [0, 0, 0, 0]));
    }
    assert_eq!(animate(&mut host, code::FRAME, [15, 0, 0, 0]), Reply::Undefined);
    assert_eq!(animate(&mut host, code::LOOK, [3, 0, 0, 0]), Reply::Undefined);
    assert_eq!(animate(&mut host, code::CYCLE, [1, 4, 3600, 1]), Reply::Undefined);
    assert_eq!(get(&mut host, one, RegisField::FRAME.number()), Reply::Int(1));
    assert_eq!(get(&mut host, one, RegisField::LOOK.number()), Reply::Int(3));
    let target = Target { kind: TargetKind::Object, number: 1 };
    assert_eq!(
        host.take_batch(1, &[]).animate.unwrap(),
        vec![
            Operation { target, kind: OperationKind::Frame(15) },
            Operation { target, kind: OperationKind::Look(3) },
            Operation { target, kind: OperationKind::Cycle { first: 1, last: 4, period: 3600, once: true } },
        ]
    );
    assert!(host.budget(ContextId::Page).contains("\"cycling\":{\"used\":1,"));
    host.begin_step(1, true);
    host.regis_reports(&Step { cycle_ended: vec![1], ..Step::default() });
    assert!(host.budget(ContextId::Page).contains("\"cycling\":{\"used\":0,"));
    assert_eq!(animate(&mut host, code::STOP_CYCLE, [0, 0, 0, 0]), Reply::Undefined);
}

/// Section 6.2, "event painted" and `bounds`: a `painted` listener
/// watches the drawing, at most 32; the watch list goes whole when it
/// changes; a tracked drawing's rectangle is the step's report, and none
/// when the report leaves it out.
#[test]
fn painted_is_watched_and_bounds_are_the_steps_report() {
    let mut host = host();
    let handles: Vec<i32> = (0..=MAX_WATCHED_PAINTINGS).map(|_| drawing(&mut host)).collect();
    let listen = |host: &mut Host, handle: i32, on: i32| call(host, Op::LISTEN, &[handle, EventKind::PAINTED.number(), on]);
    for handle in &handles[..MAX_WATCHED_PAINTINGS] {
        assert_eq!(listen(&mut host, *handle, 1), Reply::Undefined);
    }
    refuses(&mut host, "MAX_WATCHED_PAINTINGS", |host| listen(host, handles[MAX_WATCHED_PAINTINGS], 1));
    mistypes(&mut host, |host| listen(host, 4000, 1));
    assert!(
        matches!(host.call(CHANNEL, Op::LISTEN.number(), &[1, EventKind::PAINTED.number(), 1], Payload::None), Reply::Type(_)),
        "a thing is its own context's"
    );
    assert_eq!(host.take_batch(1, &[]).watch.unwrap().painted.unwrap(), (1..=32).collect::<Vec<i32>>());
    assert!(host.listens_to(ContextId::Page, 5, EventKind::PAINTED), "the report is given to the context that listens");
    // A drawing removed is no longer watched.
    host.begin_step(1, true);
    assert_eq!(call(&mut host, Op::RG_REMOVE, &[handles[0]]), Reply::Text("1:1".into()));
    assert_eq!(listen(&mut host, handles[MAX_WATCHED_PAINTINGS], 1), Reply::Undefined);
    assert_eq!(host.take_batch(2, &[]).watch.unwrap().painted.unwrap(), (2..=33).collect::<Vec<i32>>());
    // Bounds: only a tracked drawing's, only as reported.
    host.begin_step(2, true);
    assert_eq!(set(&mut host, handles[1], Property::TRACK, 1), Reply::Undefined);
    let report = |handle: i32| Bounds { handle, x: 3, y: 4, width: 50, height: 60 };
    host.regis_reports(&Step { bounds: vec![report(2), report(3), report(9999)], ..Step::default() });
    assert_eq!(call(&mut host, Op::RG_BOUNDS, &[handles[1]]), Reply::Text("3,4,50,60".into()));
    assert_eq!(call(&mut host, Op::RG_BOUNDS, &[handles[2]]), Reply::Undefined, "a drawing not tracked has no bounds");
    host.regis_reports(&Step::default());
    assert_eq!(call(&mut host, Op::RG_BOUNDS, &[handles[1]]), Reply::Undefined, "a tracked drawing the report leaves out is hidden");
}

/// Section 6.1: `define` allocates entries 1 to 15 by name; a colour is
/// ReGIS's own hue, lightness and saturation, or red, green and blue
/// converted to the nearest whole values; the receiver shows the colour
/// asked for.
#[test]
fn colours_are_named_entries_in_hls() {
    let mut host = host();
    let define = |host: &mut Host, name: &str, ints: &[i32]| host.call(PAGE, Op::RG_COLOUR_DEFINE.number(), ints, Payload::Text(name.as_bytes()));
    assert_eq!(define(&mut host, "sky", &[0, 25, 60]), Reply::Int(1));
    assert_eq!(define(&mut host, "hot", &[0x00FF_8800]), Reply::Int(2));
    assert_eq!(define(&mut host, "sky", &[240, 50, 100]), Reply::Int(1), "a name names its entry again");
    mistypes(&mut host, |host| define(host, "bad", &[361, 50, 50]));
    mistypes(&mut host, |host| define(host, "bad", &[0, 101, 50]));
    mistypes(&mut host, |host| define(host, "bad", &[0x0100_0000]));
    mistypes(&mut host, |host| define(host, "bad", &[1, 2]));
    mistypes(&mut host, |host| define(host, "a\nb", &[0, 0, 0]));
    let long = "n".repeat(MAX_COLOUR_NAME_BYTES + 1);
    refuses(&mut host, "MAX_COLOUR_NAME_BYTES", |host| define(host, &long, &[0, 0, 0]));
    refuses(&mut host, "REGIS_COLOURS", |host| call(host, Op::RG_COLOUR_SET, &[16, 0, 0, 0]));
    assert_eq!(call(&mut host, Op::RG_COLOUR_SET, &[0, 0x0000_3366]), Reply::Undefined);
    let batch = host.take_batch(1, &[]);
    assert_eq!(String::from_utf8_lossy(batch.regis.as_ref().unwrap()), "S(M0(H330L20S100))S(M1(H240L50S100))S(M2(H152L50S100))");
    let mut receiver = Receiver::new();
    receiver.take(&batch);
    let shown = receiver.terminal.palette().colours();
    assert_eq!(shown[1], [0, 255, 0], "hue 240 is green");
    // #f80 and #036, to the nearest whole hue, lightness and saturation:
    // within two parts in 255 of the colour asked for.
    for (entry, asked) in [(2, [0xFF_i32, 0x88, 0x00]), (0, [0x00, 0x33, 0x66])] {
        for part in 0..3 {
            assert!((i32::from(shown[entry][part]) - asked[part]).abs() <= 2, "entry {entry}: {:?} for {asked:?}", shown[entry]);
        }
    }
    // Fifteen names, and no sixteenth.
    host.begin_step(100, true);
    for at in 3..REGIS_COLOURS {
        assert_eq!(define(&mut host, &format!("c{at}"), &[0, 0, 0]), Reply::Int(i32::try_from(at).unwrap()));
    }
    refuses(&mut host, "REGIS_COLOURS", |host| define(host, "one more", &[0, 0, 0]));
    assert!(host.budget(ContextId::Page).contains("\"colours\":{\"used\":15,\"limit\":15}"));
}

/// Every primary and secondary colour and the greys convert to the hue
/// ReGIS gives them (the manual's chapter 2: blue 0, magenta 60, red 120,
/// yellow 180, green 240, cyan 300), and the receiver's own conversion
/// gives the colour back.
#[test]
fn red_green_and_blue_convert_to_regiss_own_hues() {
    for (rgb, want) in [
        (0x0000_00FF_u32, (0, 50, 100)),
        (0x00FF_00FF, (60, 50, 100)),
        (0x00FF_0000, (120, 50, 100)),
        (0x00FF_FF00, (180, 50, 100)),
        (0x0000_FF00, (240, 50, 100)),
        (0x0000_FFFF, (300, 50, 100)),
        (0x0000_0000, (0, 0, 0)),
        (0x00FF_FFFF, (0, 100, 0)),
        (0x0080_8080, (0, 50, 0)),
    ] {
        assert_eq!(colour::hls_of_rgb(rgb), want, "{rgb:06X}");
    }
    // Any colour comes back from the receiver within three parts in 255:
    // what whole percentages of lightness and saturation can hold.
    let mut terminal = Terminal::new();
    let mut screen = Screen::new();
    for rgb in (0..0x0100_0000_u32).step_by(0x0003_0711) {
        let (hue, lightness, saturation) = colour::hls_of_rgb(rgb);
        assert!((0..360).contains(&hue) && (0..=100).contains(&lightness) && (0..=100).contains(&saturation));
        let _ = terminal.feed_regis(&mut screen, format!("S(M1(H{hue}L{lightness}S{saturation}))").as_bytes());
        let shown = terminal.palette().colour(1);
        let asked = rgb.to_be_bytes();
        for part in 0..3 {
            assert!((i32::from(shown[part]) - i32::from(asked[part + 1])).abs() <= 4, "{rgb:06X} as H{hue}L{lightness}S{saturation} shows {shown:?}");
        }
    }
}

/// Section 10.5: one colour entry takes effect at most six times a
/// second; a later value waits for the next tick the rate allows, the
/// latest winning, and nothing throws.
#[test]
fn a_colour_entry_changes_at_most_six_times_a_second() {
    let mut host = host();
    let mut written = Vec::new();
    for tick in 0..120_u64 {
        host.begin_step(tick, true);
        assert_eq!(call(&mut host, Op::RG_COLOUR_SET, &[3, 0, i32::try_from(tick % 100).unwrap(), 50]), Reply::Undefined);
        if let Some(bytes) = host.take_batch(1, &[]).regis {
            written.push((tick, String::from_utf8(bytes).unwrap()));
        }
    }
    let ticks: Vec<u64> = written.iter().map(|(tick, _)| *tick).collect();
    assert_eq!(ticks, vec![0, 1, 2, 3, 4, 5, 60, 61, 62, 63, 64, 65]);
    assert_eq!(written[6].1, "S(M3(H0L60S50))", "the latest value is the one written");
    // A value held back is written when the rate allows, with no further call.
    host.begin_step(120, true);
    assert_eq!(host.take_batch(1, &[]).regis.map(|bytes| String::from_utf8(bytes).unwrap()).as_deref(), Some("S(M3(H0L19S50))"));
    host.begin_step(121, true);
    assert_eq!(call(&mut host, Op::RG_COLOUR_SET, &[3, 7, 7, 7]), Reply::Undefined);
    for tick in 121..126 {
        host.begin_step(tick, true);
        let _ = host.take_batch(1, &[]);
        assert_eq!(call(&mut host, Op::RG_COLOUR_SET, &[3, 7, 7, 7]), Reply::Undefined);
    }
    let batch = host.take_batch(1, &[]);
    assert_eq!(batch.regis, None);
    let page = batch.state.unwrap().into_iter().find(|state| state.context == ContextId::Page).unwrap();
    assert_eq!(page.next_due, Some(126), "the decoder is asked to wake the script's host for the colour that waits");
}

/// Section 6.3, the calls work package R1 encodes: each as the ReGIS the
/// table gives; a place with a negative coordinate as a move from where
/// the position is; and a receiver paints what was asked.
#[test]
fn a_painter_program_becomes_one_well_formed_definition() {
    use RegisPaint as P;
    let encode = |program: &[i32]| String::from_utf8(encode_plain(program).unwrap()).unwrap();
    assert_eq!(
        encode(&[P::MOVE_TO.number(), 10, 20, P::LINE_TO.number(), 30, 40, P::LINE_BY.number(), -5, 6, P::MOVE_BY.number(), 1, -2, P::DOT.number()]),
        "P[10,20]V[30,40]V[-5,+6]P[+1,-2]V[]"
    );
    assert_eq!(encode(&[P::MOVE_TO.number(), -10, 20, P::LINE_TO.number(), 5, -5]), "P[-10,+20]V[+15,-25]");
    assert_eq!(encode(&[P::POLYLINE.number(), 3, 0, 0, 10, 0, 10, 10]), "P[0,0]V[10,0][10,10]");
    assert_eq!(encode(&[P::POLYLINE.number(), 1, 4, 5]), "P[4,5]V[]");
    assert_eq!(encode(&[P::POLYGON.number(), 3, 0, 0, 10, 0, 10, 10, P::LINE_BY.number(), 1, 1]), "P[0,0]V(B)[10,0][10,10](E)V[+1,+1]");
    assert_eq!(encode(&[P::RECT.number(), 2, 3, 10, 5]), "P[2,3]V[+9][,+4][-9][,-4]");
    assert_eq!(encode(&[P::INK.number(), 15, P::FILL.number(), 5, P::RECT.number(), 0, 0, 800, 480]), "W(I15)F(P[0,0]V[+799][,+479][-799][,-479])");
    let refused = |program: &[i32]| encode_plain(program).unwrap_err();
    assert!(matches!(refused(&[99]), Reply::Type(_)));
    assert!(matches!(refused(&[P::MOVE_TO.number(), 1]), Reply::Type(_)), "a program that ends inside a call");
    assert!(matches!(refused(&[P::FILL.number(), 9, P::DOT.number()]), Reply::Type(_)), "a fill longer than the program");
    assert!(matches!(refused(&[P::FILL.number(), 2, P::FILL.number(), 0]), Reply::Type(_)), "a fill inside a fill");
    assert!(matches!(refused(&[P::FILL.number(), 2, P::INK.number(), 1]), Reply::Type(_)));
    assert!(matches!(refused(&[P::RECT.number(), 0, 0, 0, 5]), Reply::Type(_)));
    assert!(matches!(refused(&[P::POLYLINE.number(), 0]), Reply::Type(_)));
    assert!(matches!(refused(&[P::TEXT.number(), 0]), Reply::Type(_)), "a text call that ends before its options");
    assert_range(&refused(&[P::MOVE_TO.number(), 32_768, 0]), "MAX_COORDINATE");
    assert_range(&refused(&[P::MOVE_TO.number(), 32_767, 0, P::LINE_BY.number(), 1, 0]), "MAX_COORDINATE");
    assert_range(&refused(&[P::RECT.number(), 32_000, 0, 1000, 5]), "MAX_COORDINATE");
    assert_range(&refused(&[P::INK.number(), 16]), "REGIS_COLOURS");
    let mut many = vec![P::POLYLINE.number(), 1025];
    many.extend(std::iter::repeat_n(0, 2050));
    assert_range(&refused(&many), "MAX_POINTS");
    // A fill traces at most 4,096 vertices: 1,024 rectangles and one more.
    let mut fill = vec![P::FILL.number(), 5 * 1025];
    for _ in 0..1025 {
        fill.extend_from_slice(&[P::RECT.number(), 0, 0, 2, 2]);
    }
    assert_range(&refused(&fill), "MAX_FILL_VERTICES");
    let mut long = Vec::new();
    for _ in 0..REGIS_MAX_CONTENTS_BYTES / 8 {
        long.extend_from_slice(&[P::MOVE_TO.number(), 100, 100]);
    }
    assert_range(&refused(&long), "REGIS_MAX_CONTENTS_BYTES");
    // On a receiver: a filled rectangle at a negative place in a drawing
    // set at 100,100 covers exactly its pixels, and a polygon's outline
    // closes.
    let mut host = host();
    let one = made(call(&mut host, Op::RG_DRAWING, &[0, 0, 1, 100, 100, 0, -1, 1]));
    let mut program = filled(-10, -10, 20, 20, 2);
    program.extend_from_slice(&[P::INK.number(), 3, P::POLYGON.number(), 3, 30, 0, 50, 0, 50, 20]);
    assert_eq!(call_ints(&mut host, Op::RG_DEFINE, &[one], &program), Reply::Undefined);
    let mut receiver = Receiver::new();
    assert!(receiver.take(&host.take_batch(1, &[])));
    for (x, y, want) in
        [(90, 90, 2), (109, 109, 2), (100, 100, 2), (89, 90, 0), (110, 109, 0), (90, 110, 0), (140, 100, 3), (150, 110, 3), (140, 110, 3), (145, 104, 0)]
    {
        assert_eq!(receiver.pixel(x, y), want, "the pixel at {x},{y}");
    }
}

/// Sections 6.2 and 10.6: a drawing's contents are at most 65,536 bytes
/// and a page's 1 MiB, of which the channel context's share is kept for
/// it; one step's output is at most `MAX_BATCH_BYTES`; redefining
/// replaces what was counted.
#[test]
fn contents_are_held_to_a_drawing_a_page_and_a_step() {
    let mut host = host();
    // About 26 KiB of contents: 1,000 rectangles.
    let mut big = Vec::new();
    for at in 0..1000 {
        big.extend_from_slice(&[RegisPaint::RECT.number(), at % 700, at % 400, 30_000, 30_000]);
    }
    let size = encode_plain(&big).unwrap().len();
    assert!(size > 20_000 && size <= REGIS_MAX_CONTENTS_BYTES, "{size}");
    let page_share = REGIS_MAX_TOTAL_CONTENTS_BYTES - CHANNEL_SHARE_CONTENTS_BYTES;
    let mut defined = 0;
    let mut step_full = 0;
    let mut tick = 0;
    let mut handles = Vec::new();
    loop {
        // Hidden, so that the commit's cap on rendering work, which
        // counts only what is shown, is not what this test meets.
        let handle = made(call(&mut host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 0]));
        match call_ints(&mut host, Op::RG_DEFINE, &[handle], &big) {
            Reply::Undefined => {
                defined += 1;
                handles.push(handle);
            }
            Reply::Range(message) if message.starts_with("MAX_BATCH_BYTES: ") => {
                // The step is full: nothing more is staged until the next.
                step_full += 1;
                refuses(&mut host, "MAX_BATCH_BYTES", |host| call_ints(host, Op::RG_DEFINE, &[handle], &big));
                assert!(host.batch_bytes() <= MAX_BATCH_BYTES);
                let batch = host.take_batch(1, &[]);
                assert!(batch.regis.as_ref().is_some_and(|bytes| bytes.len() <= MAX_BATCH_BYTES));
                tick += 1;
                host.begin_step(tick, true);
            }
            Reply::Range(message) => {
                assert!(message.starts_with("REGIS_MAX_TOTAL_CONTENTS_BYTES: "), "{message}");
                refuses(&mut host, "REGIS_MAX_TOTAL_CONTENTS_BYTES", |host| call_ints(host, Op::RG_DEFINE, &[handle], &big));
                break;
            }
            other => panic!("{other:?}"),
        }
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES);
    }
    assert_eq!(defined, page_share / size, "the page's share of the contents, in drawings of {size} bytes");
    assert!(step_full >= 3, "the step's output filled {step_full} times");
    assert!(host.budget(ContextId::Page).contains(&format!("\"contentBytes\":{{\"used\":{},\"limit\":{page_share}}}", defined * size)));
    // Redefining replaces: a small definition frees the room of a large one.
    let _ = host.take_batch(1, &[]);
    host.begin_step(tick + 1, true);
    assert_eq!(call_ints(&mut host, Op::RG_DEFINE, &[handles[0]], &[RegisPaint::DOT.number()]), Reply::Undefined);
    let last = *handles.last().unwrap();
    assert_eq!(call_ints(&mut host, Op::RG_DEFINE, &[last], &big), Reply::Undefined);
    assert!(host.budget(ContextId::Page).contains(&format!("\"contentBytes\":{{\"used\":{},", (defined - 1) * size + 3)));
    // Removing gives the room back.
    assert!(matches!(call(&mut host, Op::RG_REMOVE, &[handles[1]]), Reply::Text(_)));
    assert!(host.budget(ContextId::Page).contains(&format!("\"contentBytes\":{{\"used\":{},", (defined - 2) * size + 3)));
    // The channel context's share is its own.
    let theirs = made(host.call(CHANNEL, Op::RG_DRAWING.number(), &[0, 0, 3, 0, 0, 0, -1, 1], Payload::None));
    let program = words(&big);
    for _ in 0..CHANNEL_SHARE_CONTENTS_BYTES / size {
        assert_eq!(host.call(CHANNEL, Op::RG_DEFINE.number(), &[theirs], Payload::Ints(&program)), Reply::Undefined);
        let _ = host.take_batch(1, &[]);
        let again = made(host.call(CHANNEL, Op::RG_DRAWING.number(), &[0, 0, 3, 0, 0, 0, -1, 1], Payload::None));
        let _ = again;
    }
}

/// Stages `text` as the step's operations, as if operations had written
/// it: for the commit errors the host's own operations never cause.
fn stage_raw(host: &mut Host, text: &[u8]) {
    host.regis_begin_change();
    host.regis.stage(text);
}

/// Section 6.2, "Commit": each of the eight commit errors is caught at
/// the host. Nothing is sent, the tables are as they stood before the
/// step, each context that had drawings is told the code and the drawing,
/// and the next step commits as if the refused one had not been.
#[test]
fn each_commit_error_is_caught_at_the_host_and_nothing_is_sent() {
    // What a step does to earn each error. The first five are beyond
    // what the host's own operations can stage (their bounds are far
    // inside the renderer's, as the limits assert), so the step's text is
    // written as an operation would write it; the last three are a
    // script's to cause.
    type Cause = fn(&mut Host, &[i32]);
    let causes: [(CommitError, Option<u16>, Cause); 8] = [
        (CommitError::Contents, Some(900), |host, _| {
            host.set_limits_for_test(usize::MAX, usize::MAX);
            let mut text = b"D(B900)".to_vec();
            text.resize(text.len() + regis::limits::MAX_DRAWING_CONTENT_BYTES + 1, b' ');
            text.extend_from_slice(b"D(E)");
            stage_raw(host, &text);
        }),
        (CommitError::Staged, Some(0), |host, _| {
            host.set_limits_for_test(usize::MAX, usize::MAX);
            stage_raw(host, &b"D(S900(X1))".repeat(regis::limits::MAX_STAGED_BYTES / 11 + 1));
        }),
        (CommitError::Parent, Some(900), |host, _| stage_raw(host, b"D(S900(P901))")),
        (CommitError::Malformed, Some(900), |host, _| stage_raw(host, b"D(S900(G9))")),
        (CommitError::Drawings, None, |host, _| {
            host.set_limits_for_test(usize::MAX, usize::MAX);
            let mut text = Vec::new();
            for number in 1000..=u16::MAX {
                put(&mut text, format_args!("D(S{number}(X1))"));
            }
            stage_raw(host, &text);
        }),
        // Seventeen deep: a chain of nine set under a chain of eight.
        (CommitError::Depth, None, |host, handles| assert_eq!(set(host, handles[8], Property::PARENT, handles[7]), Reply::Undefined)),
        // A drawing drawn in its own place, through its child's source.
        (CommitError::Cycle, None, |host, handles| assert_eq!(set(host, handles[1], Property::SOURCE, handles[0]), Reply::Undefined)),
        // A filled screen drawn through 400 visible sources costs more
        // than one repaint may.
        (CommitError::Work, Some(0), |host, handles| {
            assert_eq!(call_ints(host, Op::RG_DEFINE, &[handles[20]], &filled(0, 0, 800, 480, 1)), Reply::Undefined);
            for _ in 0..400 {
                made(call(host, Op::RG_DRAWING, &[0, handles[20], 1, 0, 0, 0, -1, 1]));
            }
        }),
    ];
    for (error, concerned, cause) in causes {
        let mut host = host();
        // Before the step: a chain of eight, a chain of nine, and more.
        let handles: Vec<i32> = (0..24).map(|_| drawing(&mut host)).collect();
        for chain in [&handles[..8], &handles[8..17]] {
            for pair in chain.windows(2) {
                assert_eq!(set(&mut host, pair[1], Property::PARENT, pair[0]), Reply::Undefined);
            }
        }
        let theirs = made(host.call(CHANNEL, Op::RG_DRAWING.number(), &[0, 0, 3, 0, 0, 0, -1, 1], Payload::None));
        assert_eq!(call(&mut host, Op::LISTEN, &[i32::from(u16::try_from(handles[22]).unwrap()), EventKind::PAINTED.number(), 1]), Reply::Undefined);
        let mut receiver = Receiver::new();
        assert!(receiver.take(&host.take_batch(1, &[])));
        host.begin_step(1, true);
        let tables = format!("{:?}{:?}{:?}", host.regis.drawings, host.regis.colours, host.regis.contents);
        // The step: changes of every kind, then the cause.
        let new = drawing(&mut host);
        assert_eq!(set(&mut host, handles[21], Property::X, 55), Reply::Undefined);
        assert_eq!(call(&mut host, Op::RG_REMOVE, &[handles[22]]), Reply::Text("1:23".into()));
        assert_eq!(call(&mut host, Op::RG_ANIMATE, &[handles[21], i32::from(code::FRAME), 2, 0, 0, 0]), Reply::Undefined);
        assert_eq!(call(&mut host, Op::RG_COLOUR_SET, &[4, 0, 50, 50]), Reply::Undefined);
        assert_eq!(set(&mut host, handles[21], Property::TRACK, 1), Reply::Undefined);
        assert_eq!(host.call(CHANNEL, Op::RG_SET.number(), &[theirs, Property::Y.number(), 9], Payload::None), Reply::Undefined);
        cause(&mut host, &handles);
        let batch = host.take_batch(2, &[]);
        host.set_limits_for_test(MAX_HOST_STATE_BYTES, MAX_BATCH_BYTES);
        assert_eq!(batch.regis, None, "{error:?}: nothing is sent");
        assert_eq!(batch.animate, None, "{error:?}: no animation operation of the step is sent");
        assert!(batch.log.unwrap().iter().any(|line| line.text.contains(&format!("code {}", error.code()))), "{error:?}: the log says so");
        assert_eq!(format!("{:?}{:?}{:?}", host.regis.drawings, host.regis.colours, host.regis.contents), tables, "{error:?}: the tables are put back");
        // Each context is told: the code and the drawing; the drawings
        // made in the step, gone; those removed in it, back.
        let page = batch.state.unwrap().into_iter().find(|state| state.context == ContextId::Page).unwrap();
        assert_eq!(page.next_due, Some(2), "{error:?}: the host is woken to tell the script");
        let (numbers, payload) = host.regis_take_failure(ContextId::Page).unwrap();
        // The drawing concerned is the renderer's to name: the one being
        // defined, the one naming a missing parent, one of a chain or a
        // cycle, the one that would have been made, or none.
        assert_eq!(numbers[0], i32::from(error.code()), "{error:?}");
        match concerned {
            Some(concerned) => assert_eq!(numbers[1], i32::from(concerned), "{error:?}"),
            None => assert!(numbers[1] > 0, "{error:?} names a drawing"),
        }
        assert_eq!(numbers[2..], [0, 0]);
        let gone = usize::try_from(payload[0]).unwrap();
        assert_eq!(payload[1], new % HANDLE_NUMBERS, "{error:?}: the drawing made in the step is gone");
        assert_eq!(payload[1 + gone..], [23], "{error:?}: the drawing removed in the step is back");
        assert_eq!(host.regis_take_failure(ContextId::Channel).unwrap().1, vec![0]);
        assert_eq!(host.regis_take_failure(ContextId::Page), None, "told once");
        mistypes(&mut host, |host| set(host, new, Property::X, 1));
        assert_eq!(get(&mut host, handles[22], Property::X.number()), Reply::Int(0));
        assert_eq!(get(&mut host, handles[21], Property::X.number()), Reply::Int(0));
        assert!(host.listens_to(ContextId::Page, 23, EventKind::PAINTED), "{error:?}: what was listened for on a drawing that is back is listened for again");
        // The next step commits, here and at the receiver, which was sent
        // nothing of the refused one.
        host.begin_step(2, true);
        assert_eq!(set(&mut host, handles[21], Property::X, 7), Reply::Undefined);
        let next = host.take_batch(3, &[]);
        assert_eq!(next.regis.as_deref(), Some(b"D(S22(X7))D(C)".as_slice()), "{error:?}: {:?}", next.log);
        assert!(receiver.take(&next), "{error:?}");
        assert_eq!(receiver.terminal.drawing_position(22), Some((7, 0)));
        assert!(receiver.terminal.committed_drawing(23).is_some());
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES);
    }
}

/// Section 10.1 rule 7 and TODO-7272: what the host counts as held for
/// ReGIS is never less than what its tables and the renderer's validator
/// hold, through a page filled to every bound, and the whole stays within
/// `MAX_HOST_STATE_BYTES`. A page at its bounds is not refused for memory.
#[test]
fn what_is_counted_as_held_covers_what_is_held() {
    let mut host = host();
    let mut tick = 0_u64;
    let mut most = 0_usize;
    let mut check = |host: &mut Host, what: &str| {
        let counted = host.state_bytes();
        let regis = &host.regis;
        let table: usize = regis
            .drawings
            .iter()
            .map(|drawing| {
                size_of::<Drawing>() + drawing.label.as_ref().map_or(0, |label| label.len()) + drawing.frames.as_ref().map_or(0, |frames| frames.len() * 2)
            })
            .sum();
        let held = regis.validator_holds() + table * 2 + regis.out.len();
        assert!(counted >= held, "{what}: {counted} bytes are counted and {held} are held");
        assert!(counted <= MAX_HOST_STATE_BYTES, "{what}: {counted} bytes are counted");
        most = most.max(counted);
    };
    let label = "L".repeat(MAX_LABEL_BYTES);
    let mut big = Vec::new();
    for at in 0..1400 {
        big.extend_from_slice(&[RegisPaint::RECT.number(), at % 700, at % 400, 30_000, 30_000]);
    }
    assert!(encode_plain(&big).unwrap().len() > 60_000);
    let mut handles: Vec<i32> = Vec::new();
    let step = |host: &mut Host, tick: &mut u64| {
        let batch = host.take_batch(1, &[]);
        assert_eq!(batch.log, None, "no commit is refused");
        *tick += 1;
        host.begin_step(*tick, true);
    };
    // Every drawing the page may hold, each with a label and sixteen frames.
    for at in 0..REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS {
        let handle = drawing(&mut host);
        handles.push(handle);
        assert_eq!(host.call(PAGE, Op::RG_LABEL.number(), &[handle], Payload::Text(label.as_bytes())), Reply::Undefined, "drawing {at}");
        // The first is every other's sixteen frames; it cannot be its own.
        if at > 0 {
            assert_eq!(call_ints(&mut host, Op::RG_FRAMES, &[handle], &[handles[0]; MAX_OBJECT_FRAMES]), Reply::Undefined, "drawing {at}");
        }
        check(&mut host, "making");
        if at % 128 == 127 {
            step(&mut host, &mut tick);
            check(&mut host, "after a commit");
        }
    }
    step(&mut host, &mut tick);
    // Their contents, to the page's share, three definitions a step.
    let mut defined = 0;
    for (at, handle) in handles.iter().enumerate().skip(1) {
        match call_ints(&mut host, Op::RG_DEFINE, &[*handle], &big) {
            Reply::Undefined => defined += 1,
            Reply::Range(message) => {
                assert!(message.starts_with("REGIS_MAX_TOTAL_CONTENTS_BYTES: "), "{message}");
                break;
            }
            other => panic!("{other:?}"),
        }
        // A hidden drawing costs no rendering work at a commit.
        assert_eq!(set(&mut host, *handle, Property::VISIBLE, 0), Reply::Undefined);
        check(&mut host, "defining");
        if at % 3 == 0 {
            step(&mut host, &mut tick);
            check(&mut host, "after a commit");
        }
    }
    assert_eq!(
        defined,
        (REGIS_MAX_TOTAL_CONTENTS_BYTES - CHANNEL_SHARE_CONTENTS_BYTES) / encode_plain(&big).unwrap().len(),
        "the page's share of contents in drawings of over 60,000 bytes"
    );
    step(&mut host, &mut tick);
    // Redefined and removed, the old contents are held until the commit.
    for handle in &handles[1..4] {
        assert_eq!(call_ints(&mut host, Op::RG_DEFINE, &[*handle], &big), Reply::Undefined);
        check(&mut host, "redefining");
    }
    assert!(matches!(call(&mut host, Op::RG_CLEAR, &[]), Reply::Text(_)));
    check(&mut host, "clearing");
    step(&mut host, &mut tick);
    check(&mut host, "cleared");
    println!("the most counted as held for a ReGIS page at its bounds: {most} bytes of {MAX_HOST_STATE_BYTES}");
}

/// The step that leaves a page (`PAGE_HIDE`) ends the page's one stream:
/// the drawings are forgotten, and the next page's stream starts from a
/// receiver that has been sent nothing.
#[test]
fn a_page_left_ends_its_stream() {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, Renderer::Regis), 100);
    host.begin_step(0, true);
    let channel = |host: &mut Host, op: Op, ints: &[i32]| host.call(CHANNEL, op.number(), ints, Payload::None);
    let first = made(channel(&mut host, Op::RG_DRAWING, &[0, 0, 3, 0, 0, 0, -1, 1]));
    assert!(host.take_batch(1, &[]).regis.is_some());
    host.begin_step(1, true);
    host.regis_reports(&Step {
        events: vec![Event { kind: Carried::PageHide, numbers: [0x100, 0, 0, 0], payload: Vec::new() }],
        flags: StepFlags::default(),
        ..Step::default()
    });
    assert!(matches!(channel(&mut host, Op::RG_SET, &[first, Property::X.number(), 1]), Reply::Type(_)));
    let second = made(channel(&mut host, Op::RG_DRAWING, &[0, 0, 3, 0, 0, 0, -1, 1]));
    assert_eq!(second, HANDLE_NUMBERS + 32_768);
    let batch = host.take_batch(2, &[]);
    assert!(Receiver::new().take(&batch), "a receiver that was sent nothing commits the new page's first step");
}

/// The names the prelude gives the commit errors are the renderer's
/// codes, 1 to 8 (regis-v2.md, "Failed commits").
#[test]
fn the_commit_errors_names_follow_the_renderers_codes() {
    let named = [
        (CommitError::Contents, "contents"),
        (CommitError::Staged, "staged"),
        (CommitError::Depth, "depth"),
        (CommitError::Cycle, "cycle"),
        (CommitError::Parent, "parent"),
        (CommitError::Work, "work"),
        (CommitError::Malformed, "malformed"),
        (CommitError::Drawings, "drawings"),
    ];
    let prelude = include_str!("../../regis.js");
    let list: Vec<String> = std::iter::once(String::new()).chain(named.iter().map(|(_, name)| (*name).to_owned())).collect();
    assert!(prelude.contains(&format!("const COMMIT_ERRORS = {list:?};")), "the prelude's names, in the codes' order");
    for (index, (error, name)) in named.iter().enumerate() {
        assert_eq!(usize::from(error.code()), index + 1);
        let (given, bound) = commit_error_name(error.code());
        assert_eq!(given, *name);
        assert!(prelude.contains(&format!("\"{bound}\"")), "{bound}");
    }
}

/// TODO-7373's measurement of the host's own part of a step that moves
/// every drawing of a page at its bound: the operations' checking and
/// staging, the commit through the renderer's validator, and the batch.
/// Printed; it holds nothing, since the figures are a machine's.
#[test]
fn the_hosts_part_of_a_step_that_moves_every_drawing_is_measured() {
    use std::time::{Duration, Instant};
    /// The steps measured: a second of ticks.
    const STEPS: u32 = 60;
    let mut host = host();
    let master = made(call(&mut host, Op::RG_DRAWING, &[0, 0, 1, 0, 0, 0, -1, 0]));
    assert_eq!(call_ints(&mut host, Op::RG_DEFINE, &[master], &filled(0, 0, 6, 6, 3)), Reply::Undefined);
    let mut handles = Vec::new();
    for at in 0..REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS - 1 {
        let at = i32::try_from(at).unwrap();
        handles.push(made(call(&mut host, Op::RG_DRAWING, &[0, master, 2, (at * 13) % 790, (at * 7) % 470, 0, -1, 1])));
        if at % 256 == 255 {
            let _ = host.take_batch(1, &[]);
            host.begin_step(u64::try_from(at).unwrap(), true);
        }
    }
    let _ = host.take_batch(1, &[]);
    let (mut reads, mut writes, mut commit, mut batch, mut bytes) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, Duration::ZERO, 0);
    for step in 0..STEPS {
        host.begin_step(10_000 + u64::from(step), true);
        let began = Instant::now();
        let mut xs = Vec::with_capacity(handles.len());
        for handle in &handles {
            let Reply::Int(x) = get(&mut host, *handle, Property::X.number()) else { panic!("no x") };
            xs.push(x);
        }
        reads += began.elapsed();
        let began = Instant::now();
        for (handle, x) in handles.iter().zip(&xs) {
            assert_eq!(set(&mut host, *handle, Property::X, (x + 1) % 790), Reply::Undefined);
        }
        writes += began.elapsed();
        let began = Instant::now();
        host.regis_commit();
        commit += began.elapsed();
        let began = Instant::now();
        let taken = host.take_batch(2, &[]);
        batch += began.elapsed();
        bytes = taken.regis.map_or(0, |regis| regis.len());
    }
    println!(
        "the host's part of a step that moves {} drawings: {:?} for the reads, {:?} for the writes with their staging, {:?} for the commit through the validator, {:?} for the batch; {bytes} bytes a commit",
        handles.len(),
        reads / STEPS,
        writes / STEPS,
        commit / STEPS,
        batch / STEPS
    );
}

/// Operation `RG_SET_MANY` (TODO-7373): one property of many drawings in
/// one call stages exactly what an assignment to each would, a value a
/// drawing already has staging nothing; every entry is held to what
/// `RG_SET` holds it to and checked before any is taken, a refusal
/// naming the entry and changing nothing.
#[test]
fn many_drawings_are_set_in_one_call_as_each_alone_would_be() {
    let mut host = host();
    let handles: Vec<i32> = (0..4).map(|_| drawing(&mut host)).collect();
    let _ = host.take_batch(1, &[]);
    host.begin_step(1, true);
    let many = |host: &mut Host, property: Property, handles: &[i32], values: &[i32]| {
        call_ints(host, Op::RG_SET_MANY, &[property.number()], &[handles, values].concat())
    };
    assert_eq!(many(&mut host, Property::X, &handles, &[10, 0, -5, 32_767]), Reply::Undefined);
    assert_eq!(many(&mut host, Property::Y, &handles[..2], &[7, 8]), Reply::Undefined);
    assert_eq!(many(&mut host, Property::DEPTH, &handles[3..], &[-2]), Reply::Undefined);
    assert_eq!(many(&mut host, Property::VISIBLE, &handles[..2], &[0, 1]), Reply::Undefined);
    assert_eq!(many(&mut host, Property::DETECTABLE, &handles[..2], &[0, 1]), Reply::Undefined);
    assert_eq!(many(&mut host, Property::X, &[], &[]), Reply::Undefined);
    assert_eq!(get(&mut host, handles[2], Property::X.number()), Reply::Int(-5));
    assert_eq!(sent(&mut host), "D(S1(X10))D(S3(X-5))D(S4(X32767))D(S1(Y7))D(S2(Y8))D(S4(L-2))D(S1(V0))D(S2(I1))D(C)");
    // With no property: a handle, a property and a value for each
    // change, a drawing's changes together as one `D(S n(…))`; a later
    // change of the same attribute in the call stands.
    host.begin_step(5, true);
    let (x, y) = (Property::X.number(), Property::Y.number());
    let triples = [handles[0], x, 640, handles[0], y, 280, handles[1], x, 3, handles[1], x, 0, handles[2], x, -5];
    assert_eq!(call_ints(&mut host, Op::RG_SET_MANY, &[0], &triples), Reply::Undefined);
    assert_eq!(sent(&mut host), "D(S1(X640,Y280))D(S2(X3,X0))D(C)");
    mistypes(&mut host, |host| call_ints(host, Op::RG_SET_MANY, &[0], &[handles[0], x]));
    mistypes(&mut host, |host| call_ints(host, Op::RG_SET_MANY, &[0], &[handles[0], Property::LAYER.number(), 2]));
    host.begin_step(2, true);
    let theirs = made(host.call(CHANNEL, Op::RG_DRAWING.number(), &[0, 0, 3, 0, 0, 0, -1, 1], Payload::None));
    // One bad entry refuses the whole call, by its place.
    for (property, bad_handles, bad_values) in [
        (Property::X, vec![handles[0], handles[1]], vec![1, 40_000]),
        (Property::VISIBLE, vec![handles[0], handles[1]], vec![1, 2]),
        (Property::X, vec![handles[0], 999], vec![1, 2]),
        (Property::X, vec![handles[0], theirs], vec![1, 2]),
        (Property::X, vec![handles[0], 0], vec![1, 2]),
    ] {
        let before = fingerprint(&mut host);
        match many(&mut host, property, &bad_handles, &bad_values) {
            Reply::Range(message) | Reply::Type(message) => assert!(message.contains("entry 1 of 2 changes"), "{message}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(fingerprint(&mut host), before, "a refused call changed the host");
    }
    assert_range(&many(&mut host, Property::Y, &handles[..1], &[-40_000]), "MAX_COORDINATE");
    // Only the five a script moves things by; an odd count is no pairing.
    mistypes(&mut host, |host| many(host, Property::LAYER, &handles[..1], &[2]));
    mistypes(&mut host, |host| many(host, Property::PARENT, &handles[..1], &[0]));
    mistypes(&mut host, |host| call_ints(host, Op::RG_SET_MANY, &[Property::X.number()], &[handles[0], 1, 2]));
    mistypes(&mut host, |host| call_ints(host, Op::RG_SET_MANY, &[99], &[handles[0], 1]));
    // And the step's output is held as for any operation.
    host.set_limits_for_test(usize::MAX, host.batch_bytes() + 20);
    refuses(&mut host, "MAX_BATCH_BYTES", |host| many(host, Property::X, &handles, &[100, 200, 300, 400]));
}
