//! Painting as a show in the decoder (neoteletext-common's DEC-7031 item
//! 5; REQ-7024, REQ-7025, REQ-7026): PAINT records, a stream fed at a
//! line's speed, the page's default paint rate and the rule table's
//! `painted` events, driven through the engine on a scripted clock.
//!
//! A picture "fed the first N bytes" is the same stream given at once to
//! a second engine, with the viewer's setting "Paint drawings gradually"
//! off so that no NAPLPS WAIT holds it back: the renderer's own picture of
//! those bytes, by the same code that draws every picture.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "a test reports a failure by panicking, and indexes and counts what it made"
)]

use super::Engine;
use crate::animation::{due_at, tick_at};
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_rules::command::{self, Command, Operand, TYPE_REPORT, TYPE_VARIABLE, VariableOperation};
use neoteletext_rules::table::{Framed, Rule, Table, Target, Trigger, TriggerTarget};
use neoteletext_web::consts::{LINE_PACE_TICKS, MIN_LINE_SPEED, line_bytes};
use neoteletext_web::records::frame::{FrameMark, FrameRecord};
use neoteletext_web::records::paint::PaintRecord;
use neoteletext_web::records::regis::RegisRecord;
use neoteletext_web::records::sheet::SheetKind;
use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::regis::{RegisPicture, unpack};
use protocol::rules::{KeyAction, PageSub, Report, RuleInput};
use std::fmt::Write as _;

/// The page the tests draw and open.
const PAGE: u16 = 0x101;
/// Its sub-page, the one the tests' streams and tables are for.
const SUB: u16 = 1;
/// The page and sub-code the tests open.
const OPEN: PageSub = PageSub::new(PAGE, SUB);
/// The presentation tick the tests start at: well after the decoder's start.
const START: u32 = 1000;
/// A line speed of the day: 1,200 bits a second, 120 bytes a second, two
/// bytes a tick.
const SPEED: u32 = 1200;

fn place(page: u16, sub: u16) -> (PageAddress, SubCode) {
    (PageAddress::from_mpp(page).unwrap(), SubCode::new(sub))
}

fn stream_record(page: u16, sub: u16, generation: u16, offset: usize, data: &[u8]) -> RegisRecord<'_> {
    let (page, subcode) = place(page, sub);
    RegisRecord { page, subcode, generation, offset: u32::try_from(offset).unwrap(), data }
}

/// A REGIS record of sub-page `sub` of `page`, as the packets of a pushed message.
fn regis_of(page: u16, sub: u16, generation: u16, offset: usize, data: &[u8]) -> Vec<u8> {
    let mut packets = Vec::new();
    neoteletext_web::records::regis::encode(stream_record(page, sub, generation, offset, data), 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
        .unwrap();
    packets
}

/// A REGIS record of the tests' page.
fn regis(generation: u16, offset: usize, data: &[u8]) -> Vec<u8> {
    regis_of(PAGE, SUB, generation, offset, data)
}

/// A NAPLPS record of sub-page `sub` of `page`.
fn naplps_of(page: u16, sub: u16, generation: u16, offset: usize, data: &[u8]) -> Vec<u8> {
    let mut packets = Vec::new();
    neoteletext_web::records::regis::encode_naplps(stream_record(page, sub, generation, offset, data), 0, 0, &mut |packet| {
        packets.extend_from_slice(&packet.bytes);
    })
    .unwrap();
    packets
}

/// A NAPLPS record of the tests' page.
fn naplps(generation: u16, offset: usize, data: &[u8]) -> Vec<u8> {
    naplps_of(PAGE, SUB, generation, offset, data)
}

/// A PAINT record (DEC-7031 item 1) for sub-page `sub` of `page`.
fn paint_of(page: u16, sub: u16, language: SheetKind, generation: u16, line_speed: Option<u32>, default_rate: Option<u16>) -> Vec<u8> {
    let (page, subcode) = place(page, sub);
    let record = PaintRecord { page, subcode, language, generation, line_speed, default_rate };
    let mut packets = Vec::new();
    neoteletext_web::records::paint::encode(record, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
    packets
}

/// A PAINT record of the tests' page stating a line speed alone.
fn paint(language: SheetKind, generation: u16, line_speed: u32) -> Vec<u8> {
    paint_of(PAGE, SUB, language, generation, Some(line_speed), None)
}

/// A FRAME record of the tests' page.
fn frame(mark: FrameMark, identifier: u16) -> Vec<u8> {
    let (page, subcode) = place(PAGE, SUB);
    let mut packets = Vec::new();
    neoteletext_web::records::frame::encode(FrameRecord { page, subcode, mark, identifier }, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
        .unwrap();
    packets
}

/// Feeds a pushed message at presentation tick `tick`.
fn feed(engine: &mut Engine, tick: u32, bytes: &[u8]) -> Vec<(u16, String)> {
    engine.set_now_us(due_at(tick));
    engine.feed_whole(true, true, bytes)
}

/// The reports among what the decoder produced.
fn reports(messages: &[FromDecoder]) -> Vec<Report> {
    messages.iter().filter_map(|message| if let FromDecoder::Report(report) = message { Some(*report) } else { None }).collect()
}

/// One wake of the decoder's loop at presentation tick `tick`, as the loop
/// makes it: the open page's rules, then the animation step, then the
/// rules' next turn, which follows at once while the rule table has
/// output. What the rule table produced.
fn wake(engine: &mut Engine, tick: u32) -> Vec<FromDecoder> {
    let now = due_at(tick);
    engine.set_now_us(now);
    let mut produced = engine.run_rules(now);
    engine.animation_step(&[PAGE], now);
    produced.extend(engine.run_rules(now));
    produced
}

/// The rules that reported at a wake at presentation tick `tick`.
fn fired(engine: &mut Engine, tick: u32) -> Vec<u8> {
    reports(&wake(engine, tick)).iter().map(|report| report.rule).collect()
}

/// Lays a part of a picture over the pixels held, as the interface does.
fn lay(pixels: &mut Vec<u8>, picture: &RegisPicture) {
    let part = unpack(&picture.packed);
    if picture.whole() {
        *pixels = part;
        return;
    }
    let width = usize::from(picture.language.width());
    let (left, across) = (usize::from(picture.area.x), usize::from(picture.area.width));
    for (row, line) in part.chunks_exact(across.max(1)).enumerate().take(usize::from(picture.area.height)) {
        let start = (usize::from(picture.area.y) + row) * width + left;
        pixels[start..start + across].copy_from_slice(line);
    }
}

/// A sub-page's whole picture as the renderer has it now: its pixels and
/// its colours. The picture is asked for whole, as when the page comes on
/// screen, so that each call stands alone.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    pixels: Vec<u8>,
    palette: Vec<[u8; 3]>,
}

impl Shown {
    /// The pixels of colour-map entry `entry`.
    fn count(&self, entry: u8) -> usize {
        self.pixels.iter().map(|&pixel| usize::from(pixel == entry)).sum()
    }
}

fn shown_of(drawn: Option<crate::regis::Drawn>) -> Shown {
    let drawn = drawn.expect("a picture to send");
    assert!(drawn.picture.whole(), "a page come on screen is sent whole");
    let mut pixels = Vec::new();
    lay(&mut pixels, &drawn.picture);
    drawn.more.iter().for_each(|part| lay(&mut pixels, part));
    Shown { pixels, palette: drawn.picture.palette.to_vec() }
}

/// The ReGIS picture of sub-page `sub` of `page` at presentation tick `tick`.
fn regis_shown_of(engine: &mut Engine, page: u16, sub: u16, tick: u32) -> Shown {
    engine.show_pictures_again(page);
    shown_of(engine.regis().take(page, Some(sub), due_at(tick)))
}

/// The ReGIS picture of the tests' page at presentation tick `tick`.
fn regis_shown(engine: &mut Engine, tick: u32) -> Shown {
    regis_shown_of(engine, PAGE, SUB, tick)
}

/// The NAPLPS picture of the tests' page.
fn naplps_shown(engine: &mut Engine) -> Shown {
    engine.show_pictures_again(PAGE);
    shown_of(engine.naplps().take(PAGE, Some(SUB)))
}

/// An engine with nothing paced and nothing waiting: the setting off.
fn at_once_engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_paint_gradually(false);
    engine
}

/// The bytes of a stream the tests put in one record, within the
/// record's `MAX_DATA`.
const RECORD_PIECE: usize = 4000;
const _: () = assert!(RECORD_PIECE <= neoteletext_web::records::regis::MAX_DATA);

/// The ReGIS picture of the first `count` bytes of `stream`, given at once.
fn regis_fed(stream: &[u8], count: usize) -> Shown {
    let mut engine = at_once_engine();
    // An empty record for none: a terminal made, and given nothing.
    feed(&mut engine, START, &regis(1, 0, &[]));
    for (index, piece) in stream[..count.min(stream.len())].chunks(RECORD_PIECE).enumerate() {
        feed(&mut engine, START, &regis(1, index * RECORD_PIECE, piece));
    }
    regis_shown(&mut engine, START)
}

/// The NAPLPS picture of the first `count` bytes of `stream`, given at once.
fn naplps_fed(stream: &[u8], count: usize) -> Shown {
    let mut engine = at_once_engine();
    // An empty record for none: a terminal made, and given nothing.
    feed(&mut engine, START, &naplps(1, 0, &[]));
    for (index, piece) in stream[..count.min(stream.len())].chunks(RECORD_PIECE).enumerate() {
        feed(&mut engine, START, &naplps(1, index * RECORD_PIECE, piece));
    }
    naplps_shown(&mut engine)
}

/// The bytes a line of `speed` bits a second has delivered after `ticks`
/// ticks of its run, by the engine's own reckoning (REQ-7024).
fn delivered(ticks: u32, speed: u32) -> usize {
    usize::try_from(line_bytes(u64::from(ticks), speed)).unwrap()
}

/// The first tick of a run by which a line of `speed` bits a second has
/// delivered `count` bytes.
fn ticks_for(count: usize, speed: u32) -> u32 {
    (0..=u32::MAX).find(|&ticks| delivered(ticks, speed) >= count).unwrap()
}

/// A ReGIS v1 picture of `lines` horizontal lines of colour 3, each of
/// its commands about twenty bytes: the base drawing, drawn as its bytes
/// are read.
fn v1_lines(lines: u16) -> Vec<u8> {
    let mut bytes = String::from("W(I3)");
    for row in 0..lines {
        let _ = write!(bytes, "P[0,{}]V[799,{}]", row * 4, row * 4);
    }
    bytes.into_bytes()
}

const SO: u8 = 0x0E;
/// WAIT and its fixed-format byte (ANSI X3.110-1983 section 5.3.2.8).
const WAIT: [u8; 2] = [0x3D, 0x5C];
/// SELECT COLOR of entry 0 (section 5.3.2.5): a command that ends the one
/// before it, since a command is drawn when the next begins.
const SELECT_COLOR: [u8; 2] = [0x3E, 0x40];

/// A three-byte coordinate pair in the default domain: (`x`/256, `y`/256)
/// of the unit screen (section 5.3.2.2.3).
fn pair(x: u8, y: u8) -> [u8; 3] {
    let (x, y) = (u16::from(x), u16::from(y));
    let byte = |shift: u16| 0x40 | (u8::try_from((x >> shift) & 7).unwrap() << 3) | u8::try_from((y >> shift) & 7).unwrap();
    [byte(6), byte(3), byte(0)]
}

/// PDI bytes of a filled rectangle of colour-map entry `entry` from (`x`,
/// `y`) of size (`width`, `height`), in 256ths of the unit screen: SELECT
/// COLOR, POINT SET absolute, RECTANGLE filled.
fn rectangle(entry: u8, x: u8, y: u8, width: u8, height: u8) -> Vec<u8> {
    let mut bytes = vec![0x3E, 0x40 | (entry << 2), 0x24];
    bytes.extend(pair(x, y));
    bytes.push(0x31);
    bytes.extend(pair(width, height));
    bytes
}

/// A NAPLPS picture of `count` small rectangles in a row, colour 3, then
/// a command that ends the last.
fn naplps_rectangles(count: u8) -> Vec<u8> {
    let mut bytes = vec![SO];
    for index in 0..count {
        bytes.extend(rectangle(3, index * 8, 100, 6, 16));
    }
    bytes.extend(SELECT_COLOR);
    bytes
}

// ---------------------------------------------------------------------------
// The rule table
// ---------------------------------------------------------------------------

/// Builds a table for [`OPEN`]: each rule a trigger and its group.
struct Builder {
    table: Box<Table>,
}

impl Builder {
    fn new() -> Self {
        Self { table: Box::new(Table::new(OPEN.page, OPEN.sub)) }
    }

    fn body(&self, made: &Command) -> Vec<u8> {
        let mut body = [0_u8; neoteletext_rules::limits::MAX_COMMAND_BYTES];
        let length = command::encode(made, self.table.page, self.table.sub, &mut body).unwrap();
        body[..length].to_vec()
    }

    /// A rule on `trigger` running a group of `commands`.
    fn rule(mut self, trigger: Trigger, commands: &[(u8, Vec<u8>)]) -> Self {
        let framed: Vec<Framed<'_>> = commands.iter().map(|(kind, body)| Framed { kind: *kind, body, bindings: &[] }).collect();
        let group = self.table.push_group(&framed).unwrap();
        self.table.rules.push(Rule::new(trigger, group)).unwrap();
        self
    }

    /// A rule on `trigger` that reports its firing.
    fn reporting(self, trigger: Trigger) -> Self {
        let report = (TYPE_REPORT, self.body(&Command::Report { value: None }));
        self.rule(trigger, &[report])
    }

    /// A rule on `trigger` that only counts in variable 0: it reports nothing.
    fn counting(self, trigger: Trigger) -> Self {
        let count = (TYPE_VARIABLE, self.body(&Command::Variable { operation: VariableOperation::Inc, variable: 0, operand: Operand::None }));
        self.rule(trigger, &[count])
    }
}

/// The `painted` trigger of a numbered drawing.
fn painted(number: u16) -> Trigger {
    Trigger::Painted { target: TriggerTarget::of(Target::object(number)) }
}

/// The `painted` trigger of the page's whole picture.
const fn painted_page() -> Trigger {
    Trigger::Painted { target: TriggerTarget::PAGE }
}

/// The packets of the RULES records of a table's image, written fragment
/// by fragment as the engine's `encode_image` writes them, but without
/// its check, so that a test can send an image the reader is to refuse.
fn rules_image(image: &[u8], generation: u16) -> Vec<u8> {
    use neoteletext_rules::limits::MAX_TABLE_FRAGMENT;
    use neoteletext_web::records::rules::{Fragment, RulesRecord, encode};
    let (page, subcode) = place(OPEN.page, OPEN.sub);
    let total = u16::try_from(image.len()).unwrap();
    let mut bytes = Vec::new();
    for (number, data) in image.chunks(MAX_TABLE_FRAGMENT).enumerate() {
        let offset = u16::try_from(number * MAX_TABLE_FRAGMENT).unwrap();
        let record = RulesRecord { page, subcode, generation, fragment: Some(Fragment { total, offset, data }) };
        encode(&record, 0, u8::try_from(number + 1).unwrap(), &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
    }
    bytes
}

/// A table's image.
fn image_of(table: &Table) -> Vec<u8> {
    let mut image = vec![0_u8; neoteletext_rules::limits::MAX_TABLE_BYTES];
    let length = neoteletext_rules::encode(table, neoteletext_web::records::body_check, &mut image).unwrap();
    image.truncate(length);
    image
}

/// Opens [`OPEN`] at presentation tick `tick`, with no table yet.
fn open(engine: &mut Engine, tick: u32) {
    engine.set_now_us(due_at(tick));
    let _ = engine.rule_message(ToDecoder::RulesPage(Some(OPEN))).unwrap();
}

/// Sends `table` for the open page at presentation tick `tick`, and runs
/// its first tick: what that produced.
fn load(engine: &mut Engine, tick: u32, table: &Table) -> Vec<FromDecoder> {
    feed(engine, tick, &rules_image(&image_of(table), 1));
    let now = due_at(tick);
    engine.run_rules(now)
}

/// An engine with [`OPEN`] open and `table` loaded at [`START`].
fn opened(table: &Table) -> Engine {
    let mut engine = Engine::new();
    open(&mut engine, START);
    let _ = load(&mut engine, START, table);
    engine
}

// ---------------------------------------------------------------------------
// REQ-7024: a line speed
// ---------------------------------------------------------------------------

/// REQ-7024, DEC-7031 items 1 and 5: a ReGIS v1 picture whose PAINT record
/// states 1,200 bits a second shows, at each chosen tick of its run, the
/// picture a renderer fed the first `line_bytes(tick, 1200)` bytes shows;
/// tick 0 of the run is the first animation step after the bytes arrived,
/// which has fed none of them.
#[test]
fn a_regis_v1_picture_at_a_line_speed_shows_at_each_tick_the_bytes_the_line_delivered() {
    let stream = v1_lines(30);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Regis, 1, SPEED));
    feed(&mut engine, START, &regis(1, 0, &stream));
    let end = ticks_for(stream.len(), SPEED);
    let mut differed = false;
    for run in [0, 1, 2, 7, 50, 123, end - 1, end] {
        wake(&mut engine, START + run);
        let count = delivered(run, SPEED);
        let expected = regis_fed(&stream, count);
        let seen = regis_shown(&mut engine, START + run);
        assert!(seen == expected, "tick {run} of the run: the picture of the first {count} of {} bytes", stream.len());
        differed |= count > 0 && count < stream.len() && seen != regis_fed(&stream, stream.len());
    }
    assert!(differed, "the chosen ticks saw the picture part drawn");
    assert_eq!(regis_fed(&stream, 0).count(3), 0, "the run's tick 0 shows none of it");
}

/// REQ-7024, DEC-7031 items 1, 3 and 5: the same for a NAPLPS picture.
#[test]
fn a_naplps_picture_at_a_line_speed_shows_at_each_tick_the_bytes_the_line_delivered() {
    let stream = naplps_rectangles(24);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Naplps, 1, SPEED));
    feed(&mut engine, START, &naplps(1, 0, &stream));
    let end = ticks_for(stream.len(), SPEED);
    let mut differed = false;
    for run in [0, 1, 6, 30, 77, end - 1, end] {
        wake(&mut engine, START + run);
        let count = delivered(run, SPEED);
        let expected = naplps_fed(&stream, count);
        let seen = naplps_shown(&mut engine);
        assert!(seen == expected, "tick {run} of the run: the picture of the first {count} of {} bytes", stream.len());
        differed |= count > 0 && count < stream.len() && seen != naplps_fed(&stream, stream.len());
    }
    assert!(differed, "the chosen ticks saw the picture part drawn");
}

/// REQ-7024 ("at the end the screen is exactly the picture painted at
/// once"): the last frame of a paced replay, in either language, is the
/// picture of every byte given at once, and a ReGIS v2 commit paced with
/// the base drawing appears as its bytes arrive.
#[test]
fn a_paced_picture_ends_as_the_picture_painted_at_once() {
    let mut stream = v1_lines(10);
    stream.extend_from_slice(b"D(B1)W(I5)F(V[0,0][39,0][39,39][0,39])D(E)D(S1(X300,Y200),C)");
    stream.extend_from_slice(&v1_lines(5)[5..]);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Regis, 1, SPEED));
    feed(&mut engine, START, &regis(1, 0, &stream));
    let end = ticks_for(stream.len(), SPEED);
    wake(&mut engine, START);
    wake(&mut engine, START + end - 1);
    assert_eq!(regis_shown(&mut engine, START + end - 1), regis_fed(&stream, delivered(end - 1, SPEED)), "a tick short of the end, a byte short");
    wake(&mut engine, START + end);
    assert_eq!(regis_shown(&mut engine, START + end), regis_fed(&stream, stream.len()), "ReGIS: the last frame is the picture at once");
    assert!(regis_shown(&mut engine, START + end).count(5) > 1000, "with the numbered drawing committed by the replay");

    let stream = naplps_rectangles(16);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Naplps, 1, MIN_LINE_SPEED));
    feed(&mut engine, START, &naplps(1, 0, &stream));
    let end = ticks_for(stream.len(), MIN_LINE_SPEED);
    wake(&mut engine, START);
    wake(&mut engine, START + end - 1);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, delivered(end - 1, MIN_LINE_SPEED)), "a tick short of the end, a byte short");
    wake(&mut engine, START + end);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, stream.len()), "NAPLPS: the last frame is the picture at once");
}

/// REQ-7024, DEC-6524 item 12, DEC-7031 item 3: a NAPLPS WAIT inside a
/// paced picture adds its time to the line's. Before the WAIT the picture
/// is the line's bytes; for the WAIT's 30 ticks it stands; after, it is
/// the bytes of a line that ran 30 ticks less, and the last byte is drawn
/// 30 ticks later than a line alone would draw it.
#[test]
fn a_naplps_wait_under_a_line_speed_adds_its_time_to_the_lines() {
    const WAIT_TICKS: u32 = 30;
    let mut stream = naplps_rectangles(8);
    stream.pop();
    stream.pop();
    let before = stream.len();
    stream.extend(WAIT);
    stream.push(0x40 | 5);
    stream.extend_from_slice(&naplps_rectangles(8)[1..]);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Naplps, 1, SPEED));
    feed(&mut engine, START, &naplps(1, 0, &stream));
    wake(&mut engine, START);
    // Before the WAIT's bytes: the line alone.
    let early = ticks_for(before, SPEED) - 1;
    wake(&mut engine, START + early);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, delivered(early, SPEED)), "before the WAIT, the line's bytes");
    let end = ticks_for(stream.len(), SPEED) + WAIT_TICKS;
    // When a line alone would have ended, 30 ticks of it are still to come.
    let alone = end - WAIT_TICKS;
    wake(&mut engine, START + alone);
    let short = delivered(alone - WAIT_TICKS, SPEED);
    assert!(short > before + 4 && short + 40 < stream.len());
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, short), "after the WAIT, the line's bytes less its 30 ticks");
    // Three ticks short of the end, still the line less the WAIT.
    let late = end - 3;
    wake(&mut engine, START + late);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, delivered(late - WAIT_TICKS, SPEED)));
    wake(&mut engine, START + end);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, stream.len()), "it ends 30 ticks after the line alone would");
}

/// REQ-7024 (the run's tick 0 is the first animation step after the
/// bytes arrive): bytes of the same generation that arrive after the
/// replay ran dry begin a new run, as on an idle line, and wait their
/// full time from the step that follows them, however long the decoder
/// slept before; the ticks of that sleep are not spent on them.
#[test]
#[ignore = "fault: a stream's bytes arriving after its replay ran dry are fed the ticks the decoder slept (stale told), reported for a Fable fix"]
fn bytes_after_a_replay_ran_dry_wait_their_line_time_from_the_next_step() {
    let first = v1_lines(4);
    let more = b"W(I5)P[0,300]V[799,300]P[0,310]V[799,310]".to_vec();
    let mut whole = first.clone();
    whole.extend_from_slice(&more);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Regis, 1, SPEED));
    feed(&mut engine, START, &regis(1, 0, &first));
    wake(&mut engine, START);
    let dry = START + ticks_for(first.len(), SPEED);
    wake(&mut engine, dry);
    assert_eq!(regis_shown(&mut engine, dry), regis_fed(&whole, first.len()), "the first bytes drawn");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(dry)), None, "and the loop sleeps");
    // Ten seconds asleep, then more of the stream.
    let later = dry + 600;
    feed(&mut engine, later, &regis(1, first.len(), &more));
    wake(&mut engine, later);
    assert_eq!(regis_shown(&mut engine, later), regis_fed(&whole, first.len()), "tick 0 of the new run: none of the new bytes");
    wake(&mut engine, later + 3);
    assert_eq!(regis_shown(&mut engine, later + 3), regis_fed(&whole, first.len() + delivered(3, SPEED)), "three ticks on, three ticks of the line");
}

/// REQ-7024, DEC-7031 item 3, on NAPLPS: the same for the player.
#[test]
#[ignore = "fault: a stream's bytes arriving after its replay ran dry are fed the ticks the decoder slept (stale told), reported for a Fable fix"]
fn naplps_bytes_after_a_replay_ran_dry_wait_their_line_time_from_the_next_step() {
    let first = naplps_rectangles(2);
    let more = rectangle(6, 0, 0, 64, 64);
    let mut whole = first.clone();
    whole.extend_from_slice(&more);
    whole.extend(SELECT_COLOR);
    let mut more = more;
    more.extend(SELECT_COLOR);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Naplps, 1, SPEED));
    feed(&mut engine, START, &naplps(1, 0, &first));
    wake(&mut engine, START);
    let dry = START + ticks_for(first.len(), SPEED);
    wake(&mut engine, dry);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&whole, first.len()));
    let later = dry + 600;
    feed(&mut engine, later, &naplps(1, first.len(), &more));
    wake(&mut engine, later);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&whole, first.len()), "tick 0 of the new run: none of the new bytes");
    wake(&mut engine, later + 3);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&whole, first.len() + delivered(3, SPEED)), "three ticks on, three ticks of the line");
}

// ---------------------------------------------------------------------------
// REQ-7025: the page's default paint rate
// ---------------------------------------------------------------------------

/// A numbered drawing of `lines` lines of 300 pixels in colour `colour`.
fn lined(number: u16, colour: u8, lines: u16) -> String {
    let mut contents = format!("D(B{number})W(I{colour})");
    for row in 0..lines {
        let _ = write!(contents, "P[0,{}]V[299,{}]", row * 2, row * 2);
    }
    contents.push_str("D(E)");
    contents
}

/// Three drawings: 1 sets no `T`, 2 sets `T0`, 3 sets `T4`; `first` is
/// what drawing 1's attributes add.
fn three_drawings(first: &str) -> Vec<u8> {
    format!("{}{}{}D(S1(X10,Y10{first}),S2(X10,Y200,T0),S3(X400,Y10,T4),C)", lined(1, 3, 10), lined(2, 4, 10), lined(3, 5, 10)).into_bytes()
}

/// REQ-7025, DEC-7031 items 1, 2 and 5: under a page default rate of 20, a
/// drawing with no `T` paints exactly as one given `T20`, one with `T0`
/// appears at once and one with `T4` paints at its own rate; and the
/// finished picture is the picture painted at once.
#[test]
fn under_a_default_rate_no_t_paints_at_it_t0_at_once_and_an_own_t_at_its_own() {
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint_of(PAGE, SUB, SheetKind::Regis, 1, None, Some(20)));
    feed(&mut engine, START, &regis(1, 0, &three_drawings("")));
    let mut reference = Engine::new();
    feed(&mut reference, START, &regis(1, 0, &three_drawings(",T20")));
    let whole = regis_fed(&three_drawings(""), usize::MAX);
    let mut part_painted = false;
    for tick in [0, 1, 30, 60, 89, 120, 300, 450, 600] {
        wake(&mut engine, START + tick);
        wake(&mut reference, START + tick);
        let seen = regis_shown(&mut engine, START + tick);
        assert!(seen == regis_shown(&mut reference, START + tick), "tick {tick}: drawing 1 paints as one given T20, the others as their own T");
        assert_eq!(seen.count(4), whole.count(4), "tick {tick}: T0 is whole at once");
        if seen.count(3) > 0 && seen.count(3) < whole.count(3) {
            part_painted = true;
            assert!(seen.count(5) < seen.count(3), "tick {tick}: T4 paints slower than the default 20");
        }
    }
    assert!(part_painted, "drawing 1 was seen part painted");
    wake(&mut engine, START + 2000);
    assert_eq!(regis_shown(&mut engine, START + 2000), whole, "the finished picture is the picture painted at once");
}

/// REQ-7025 (children wait for a parent painting at the default): a child
/// with `T0` does not appear while its parent, which sets no `T`, paints
/// at the page's default rate, and appears whole when the parent ends.
#[test]
fn under_a_default_rate_a_child_waits_for_its_parent() {
    let scene = format!("{}{}D(S10(X10,Y10),S11(P10,X0,Y100,T0),C)", lined(10, 3, 10), lined(11, 6, 2)).into_bytes();
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint_of(PAGE, SUB, SheetKind::Regis, 1, None, Some(20)));
    feed(&mut engine, START, &regis(1, 0, &scene));
    let whole = regis_fed(&scene, usize::MAX);
    assert!(whole.count(6) > 0);
    let mut parent_seen_part = false;
    let mut finished = None;
    for tick in 0..400 {
        wake(&mut engine, START + tick);
        let seen = regis_shown(&mut engine, START + tick);
        if seen.count(3) < whole.count(3) {
            parent_seen_part |= seen.count(3) > 0;
            assert_eq!(seen.count(6), 0, "tick {tick}: the child waits while its parent paints");
        } else {
            assert_eq!(seen.count(6), whole.count(6), "tick {tick}: the parent whole, the child whole with it");
            finished = Some(tick);
            break;
        }
    }
    assert!(parent_seen_part && finished.is_some_and(|tick| tick > 30), "the parent painted over ticks: {finished:?}");
}

/// REQ-7025 ("a page without the default paints exactly as before"): with
/// no PAINT record a drawing that sets no `T` is whole at its commit, one
/// with `T4` paints at its own rate, and nothing else wakes the loop.
#[test]
fn a_regis_page_with_no_default_rate_paints_as_before() {
    let mut engine = Engine::new();
    feed(&mut engine, START, &regis(1, 0, &three_drawings("")));
    wake(&mut engine, START);
    let seen = regis_shown(&mut engine, START);
    let whole = regis_fed(&three_drawings(""), usize::MAX);
    assert_eq!((seen.count(3), seen.count(4)), (whole.count(3), whole.count(4)), "no T and T0: whole at the commit");
    assert!(seen.count(5) < whole.count(5), "T4 at its own rate");
    wake(&mut engine, START + 3000);
    assert_eq!(regis_shown(&mut engine, START + 3000), whole);
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START + 3000)), None);
}

/// REQ-7025, REQ-6536 item 1 (the ticks a drawing earns are those after
/// its commit): a drawing committed after the decoder slept ten seconds
/// paints from its commit, and is not given the ticks of the sleep.
#[test]
#[ignore = "fault: a drawing committed after the decoder slept is given the slept ticks (stale told; also on livedevelopment with its own T), reported for a Fable fix"]
fn a_drawing_committed_after_a_sleep_paints_from_its_commit() {
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint_of(PAGE, SUB, SheetKind::Regis, 1, None, Some(20)));
    feed(&mut engine, START, &regis(1, 0, b"P[0,0]"));
    wake(&mut engine, START);
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START)), None, "the loop sleeps");
    let later = START + 600;
    let scene = format!("{}D(S1(X10,Y10),C)", lined(1, 3, 10));
    feed(&mut engine, later, &regis(1, 6, scene.as_bytes()));
    wake(&mut engine, later + 1);
    let seen = regis_shown(&mut engine, later + 1);
    let mut whole = b"P[0,0]".to_vec();
    whole.extend_from_slice(scene.as_bytes());
    assert!(seen.count(3) < regis_fed(&whole, usize::MAX).count(3), "one tick after its commit it is not whole");
}

// ---------------------------------------------------------------------------
// The viewer's setting and reduced motion
// ---------------------------------------------------------------------------

/// A base drawing of ten lines, then drawing 1 with no `T`, whole at
/// once only when no default rate is in force.
fn base_and_drawing() -> Vec<u8> {
    let mut stream = v1_lines(10);
    stream.extend_from_slice(format!("{}D(S1(X400,Y300),C)", lined(1, 5, 10)).as_bytes());
    stream
}

/// The PAINT record of [`base_and_drawing`]: a line speed and a default rate.
fn paint_both() -> Vec<u8> {
    paint_of(PAGE, SUB, SheetKind::Regis, 1, Some(SPEED), Some(4))
}

/// A table on [`OPEN`] reporting `painted` of drawing 1 (rule 0) and of
/// the page (rule 1).
fn watching_both() -> Box<Table> {
    Builder::new().reporting(painted(1)).reporting(painted_page()).table
}

/// REQ-7024, REQ-7025, REQ-7026, DEC-7031 item 5: with the setting off at
/// a generation's start, a picture with a line speed and a default rate
/// is whole at once, and both `painted` events fire in the tick it
/// appears, and never again.
#[test]
fn the_setting_off_at_a_generations_start_paints_at_once_and_fires_in_the_appearing_tick() {
    let mut engine = opened(&watching_both());
    engine.set_paint_gradually(false);
    let _ = fired(&mut engine, START + 1);
    feed(&mut engine, START + 5, &paint_both());
    feed(&mut engine, START + 5, &regis(1, 0, &base_and_drawing()));
    let mut seen = fired(&mut engine, START + 5);
    seen.sort_unstable();
    assert_eq!(seen, [0, 1], "both events in the appearing tick");
    assert_eq!(regis_shown(&mut engine, START + 5), regis_fed(&base_and_drawing(), usize::MAX), "whole at once");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START + 5)), None, "nothing paced, nothing painting");
    for tick in 6..20 {
        assert_eq!(fired(&mut engine, START + tick), [], "tick {tick}: once");
    }
}

/// The same with the platform's reduced motion on (DEC-7031 "reduced
/// motion paints at once like the setting off").
#[test]
fn reduced_motion_at_a_generations_start_paints_at_once_and_fires_in_the_appearing_tick() {
    let mut engine = opened(&watching_both());
    engine.set_reduced_motion(true);
    let _ = fired(&mut engine, START + 1);
    feed(&mut engine, START + 5, &paint_both());
    feed(&mut engine, START + 5, &regis(1, 0, &base_and_drawing()));
    let mut seen = fired(&mut engine, START + 5);
    seen.sort_unstable();
    assert_eq!(seen, [0, 1], "both events in the appearing tick");
    assert_eq!(regis_shown(&mut engine, START + 5), regis_fed(&base_and_drawing(), usize::MAX), "whole at once");
    for tick in 6..20 {
        assert_eq!(fired(&mut engine, START + tick), [], "tick {tick}: once");
    }

    // NAPLPS: the line speed is not taken either.
    let mut engine = Engine::new();
    engine.set_reduced_motion(true);
    let stream = naplps_rectangles(12);
    feed(&mut engine, START, &paint(SheetKind::Naplps, 1, MIN_LINE_SPEED));
    feed(&mut engine, START, &naplps(1, 0, &stream));
    wake(&mut engine, START);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, usize::MAX), "a NAPLPS picture at once too");
}

/// REQ-7024, REQ-7026, DEC-7031 item 5: the setting turned off in the
/// middle of a replay draws everything held and every drawing whole, and
/// the drawing's and the page's events both fire in that tick.
#[test]
fn the_setting_turned_off_mid_replay_draws_everything_and_fires_both_events_in_that_tick() {
    let mut engine = opened(&watching_both());
    feed(&mut engine, START + 1, &paint_both());
    feed(&mut engine, START + 1, &regis(1, 0, &base_and_drawing()));
    for tick in 1..40 {
        assert_eq!(fired(&mut engine, START + tick), [], "tick {tick}: the replay goes on");
    }
    assert_ne!(regis_shown(&mut engine, START + 39), regis_fed(&base_and_drawing(), usize::MAX), "part drawn");
    engine.set_now_us(due_at(START + 40));
    engine.set_paint_gradually(false);
    // Told as the setting changes, before any animation step of the tick.
    let mut seen: Vec<u8> = reports(&engine.run_rules(due_at(START + 40))).iter().map(|report| report.rule).collect();
    seen.sort_unstable();
    assert_eq!(seen, [0, 1], "both events in the tick the setting went off");
    assert_eq!(regis_shown(&mut engine, START + 40), regis_fed(&base_and_drawing(), usize::MAX), "everything whole");
    for tick in 41..60 {
        assert_eq!(fired(&mut engine, START + tick), [], "tick {tick}: once");
    }
}

/// DEC-7031 item 5, on NAPLPS: the setting turned off mid replay draws
/// what the player held at once, and the page's event fires in that tick.
#[test]
fn the_setting_turned_off_mid_naplps_replay_draws_everything_in_that_tick() {
    let mut engine = opened(&Builder::new().reporting(painted_page()).table);
    let stream = naplps_rectangles(20);
    feed(&mut engine, START + 1, &paint(SheetKind::Naplps, 1, MIN_LINE_SPEED));
    feed(&mut engine, START + 1, &naplps(1, 0, &stream));
    for tick in 1..30 {
        assert_eq!(fired(&mut engine, START + tick), [], "tick {tick}");
    }
    engine.set_now_us(due_at(START + 30));
    engine.set_paint_gradually(false);
    let seen: Vec<u8> = reports(&engine.run_rules(due_at(START + 30))).iter().map(|report| report.rule).collect();
    assert_eq!(seen, [0], "the page's event as the setting changes, before any animation step of the tick");
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, usize::MAX));
}

// ---------------------------------------------------------------------------
// The PAINT record's generation, frames and the page cache
// ---------------------------------------------------------------------------

/// DEC-7031 item 1: a PAINT record naming another generation than the one
/// that starts is not applied, and is dropped: the generation it named,
/// starting later, is not paced by it either.
#[test]
fn a_paint_record_naming_another_generation_is_ignored_and_dropped() {
    let stream = v1_lines(20);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Regis, 5, SPEED));
    feed(&mut engine, START, &regis(1, 0, &stream));
    wake(&mut engine, START);
    assert_eq!(regis_shown(&mut engine, START), regis_fed(&stream, usize::MAX), "generation 1 is painted at once");
    feed(&mut engine, START + 1, &regis(5, 0, &stream));
    wake(&mut engine, START + 1);
    assert_eq!(regis_shown(&mut engine, START + 1), regis_fed(&stream, usize::MAX), "generation 5 too: its PAINT record was dropped");

    let stream = naplps_rectangles(12);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Naplps, 5, SPEED));
    feed(&mut engine, START, &naplps(1, 0, &stream));
    feed(&mut engine, START, &naplps(5, 0, &stream));
    wake(&mut engine, START);
    assert_eq!(naplps_shown(&mut engine), naplps_fed(&stream, usize::MAX), "NAPLPS: the same");
}

/// DEC-7031 item 1 ("holds it inside an atomic frame"), ANIM-2: a PAINT
/// record held in a frame with its stream is applied with it, here when
/// the frame ends at its limit of ticks, so the generation is paced from
/// that tick.
#[test]
fn a_paint_record_held_in_a_frame_applies_with_its_stream() {
    let old = v1_lines(3);
    let new = v1_lines(25);
    let mut engine = Engine::new();
    feed(&mut engine, START, &regis(1, 0, &old));
    wake(&mut engine, START);
    feed(&mut engine, START, &frame(FrameMark::Begin, 9));
    feed(&mut engine, START, &paint(SheetKind::Regis, 2, SPEED));
    feed(&mut engine, START, &regis(2, 0, &new));
    let ended = START + neoteletext_web::consts::MAX_FRAME_TICKS;
    wake(&mut engine, ended - 1);
    assert_eq!(regis_shown(&mut engine, ended - 1), regis_fed(&old, usize::MAX), "held: the old picture");
    wake(&mut engine, ended);
    assert_eq!(regis_shown(&mut engine, ended), regis_fed(&new, 0), "the frame ended: tick 0 of the paced run");
    wake(&mut engine, ended + 20);
    assert_eq!(regis_shown(&mut engine, ended + 20), regis_fed(&new, delivered(20, SPEED)), "paced by the PAINT record it held");
}

/// DEC-7031 item 5, amending ANIM-3 and REQ-6536 item 3: a generation
/// with a line speed that ends a frame is presented at the frame's end,
/// cleared, and fills as the line delivers it; the old picture is not
/// held until the new one is whole.
#[test]
fn a_paced_generation_ending_a_frame_is_presented_cleared_then_fills() {
    let old = v1_lines(3);
    let new = v1_lines(25);
    let mut engine = Engine::new();
    feed(&mut engine, START, &regis(1, 0, &old));
    let mut pixels = Vec::new();
    lay(&mut pixels, &engine.regis().take(PAGE, Some(SUB), due_at(START)).unwrap().picture);
    feed(&mut engine, START, &frame(FrameMark::Begin, 9));
    feed(&mut engine, START, &paint(SheetKind::Regis, 2, SPEED));
    feed(&mut engine, START, &regis(2, 0, &new));
    assert!(engine.regis().take(PAGE, Some(SUB), due_at(START)).is_none(), "nothing while the frame is open");
    feed(&mut engine, START + 1, &frame(FrameMark::End, 9));
    wake(&mut engine, START + 1);
    let drawn = engine.regis().take(PAGE, Some(SUB), due_at(START + 1)).expect("presented at the frame's end");
    lay(&mut pixels, &drawn.picture);
    drawn.more.iter().for_each(|part| lay(&mut pixels, part));
    assert_eq!(pixels, regis_fed(&new, 0).pixels, "cleared: the old picture is gone");
    for run in [10, 40] {
        wake(&mut engine, START + 1 + run);
        let drawn = engine.regis().take(PAGE, Some(SUB), due_at(START + 1 + run)).expect("the part the line delivered");
        lay(&mut pixels, &drawn.picture);
        drawn.more.iter().for_each(|part| lay(&mut pixels, part));
        assert_eq!(pixels, regis_fed(&new, delivered(run, SPEED)).pixels, "tick {run} of the run: filling");
    }
}

/// ANIM-3, REQ-6536 item 3: a generation with no line speed in a frame
/// still swaps whole: the old picture stays presented until the frame
/// ends, and the new one is presented whole at its end.
#[test]
fn an_unpaced_generation_in_a_frame_still_swaps_whole() {
    let old = v1_lines(3);
    let new = v1_lines(25);
    let mut engine = Engine::new();
    feed(&mut engine, START, &regis(1, 0, &old));
    let _ = engine.regis().take(PAGE, Some(SUB), due_at(START)).unwrap();
    feed(&mut engine, START, &frame(FrameMark::Begin, 9));
    feed(&mut engine, START, &regis(2, 0, &new));
    wake(&mut engine, START + 1);
    assert!(engine.regis().take(PAGE, Some(SUB), due_at(START + 1)).is_none(), "the old picture stays presented");
    feed(&mut engine, START + 2, &frame(FrameMark::End, 9));
    wake(&mut engine, START + 2);
    assert_eq!(regis_shown(&mut engine, START + 2), regis_fed(&new, usize::MAX), "the new one whole on one tick");
}

/// DEC-7031 item 1 ("keeps it with the stream in the page cache so a
/// restored page replays again"), PAR-34: a paced picture drawn again from
/// its kept source is replayed at its line speed from its first byte.
#[cfg(feature = "page-cache")]
#[test]
fn a_picture_restored_from_its_kept_source_is_replayed_at_its_line_speed() {
    let stream = v1_lines(25);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Regis, 1, SPEED));
    feed(&mut engine, START, &regis(1, 0, &stream));
    let end = ticks_for(stream.len(), SPEED);
    wake(&mut engine, START);
    wake(&mut engine, START + end);
    assert_eq!(regis_shown(&mut engine, START + end), regis_fed(&stream, usize::MAX));
    // Its live terminal goes, as a page closed drops it (RULE-111).
    engine.regis().forget(PAGE, SUB);
    let again = START + end + 100;
    engine.set_now_us(due_at(again));
    let (drawn, _) = engine.take_pictures(PAGE, Some(SUB), due_at(again));
    assert_eq!(shown_of(drawn), regis_fed(&stream, 0), "restored: tick 0 of its replay, nothing drawn");
    wake(&mut engine, again);
    wake(&mut engine, again + 30);
    assert_eq!(regis_shown(&mut engine, again + 30), regis_fed(&stream, delivered(30, SPEED)), "replayed at its line speed");
}

/// DEC-7031 item 1, PAR-34: a kept PAINT record that names another
/// generation than the kept stream's (the next stream's, come before it)
/// is not replayed: the kept picture is drawn at once.
#[cfg(feature = "page-cache")]
#[test]
fn a_kept_paint_record_of_another_generation_is_not_replayed() {
    let stream = v1_lines(25);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Regis, 1, SPEED));
    feed(&mut engine, START, &regis(1, 0, &stream));
    feed(&mut engine, START, &paint(SheetKind::Regis, 2, SPEED));
    engine.regis().forget(PAGE, SUB);
    engine.set_now_us(due_at(START + 1));
    let (drawn, _) = engine.take_pictures(PAGE, Some(SUB), due_at(START + 1));
    assert_eq!(shown_of(drawn), regis_fed(&stream, usize::MAX), "generation 1 is drawn at once");
}

// ---------------------------------------------------------------------------
// REQ-7026: the `painted` events
// ---------------------------------------------------------------------------

/// A drawing of one line of 100 pixels at `T1`, 100 work units a second:
/// it paints over about 60 ticks.
fn slow_drawing() -> Vec<u8> {
    b"D(B1)W(I3)P[0,0]V[99,0]D(E)D(S1(X10,Y10,T1),C)".to_vec()
}

/// Wakes each tick from `from` until the picture is `whole`, collecting
/// the rules fired: the tick the picture became whole, and what fired at
/// each tick.
fn until_whole(engine: &mut Engine, from: u32, whole: &Shown) -> (u32, Vec<(u32, u8)>) {
    let mut seen = Vec::new();
    for tick in from..from + 2000 {
        seen.extend(fired(engine, tick).into_iter().map(|rule| (tick, rule)));
        if regis_shown(engine, tick) == *whole {
            for later in tick + 1..tick + 10 {
                seen.extend(fired(engine, later).into_iter().map(|rule| (later, rule)));
            }
            return (tick, seen);
        }
    }
    panic!("never whole");
}

/// REQ-7026, DEC-7031 items 2, 4 and 5: a rule on `painted` of a drawing
/// painted at its `T` fires exactly once, in the tick its painting ends.
#[test]
fn painted_fires_for_a_drawing_once_in_the_tick_its_painting_ends() {
    let mut engine = opened(&Builder::new().reporting(painted(1)).table);
    let _ = fired(&mut engine, START + 1);
    feed(&mut engine, START + 2, &regis(1, 0, &slow_drawing()));
    let (whole_at, seen) = until_whole(&mut engine, START + 2, &regis_fed(&slow_drawing(), usize::MAX));
    assert!(whole_at > START + 30, "it painted over ticks");
    assert_eq!(seen, [(whole_at, 0)], "once, in the tick it became whole");
}

/// REQ-7026, DEC-7031 item 5: a rule on `painted` of the page fires once,
/// in the tick the replay has given the renderer its last byte.
#[test]
fn painted_fires_for_the_page_when_its_replay_has_drawn_its_last_byte() {
    let stream = v1_lines(20);
    let mut engine = opened(&Builder::new().reporting(painted_page()).table);
    let _ = fired(&mut engine, START + 1);
    feed(&mut engine, START + 2, &paint(SheetKind::Regis, 1, SPEED));
    feed(&mut engine, START + 2, &regis(1, 0, &stream));
    let end = START + 2 + ticks_for(stream.len(), SPEED);
    for tick in START + 2..end {
        assert_eq!(fired(&mut engine, tick), [], "tick {tick}: bytes still on the line");
    }
    assert_eq!(fired(&mut engine, end), [0], "the tick of the last byte");
    for tick in end + 1..end + 20 {
        assert_eq!(fired(&mut engine, tick), [], "once");
    }
}

/// REQ-7026 ("painting that restarts fires the trigger again"): a drawing
/// redefined paints and fires again; a new generation of the stream fires
/// the drawing's and the page's events again.
#[test]
fn painted_fires_again_after_a_redefinition_and_a_new_generation() {
    let mut engine = opened(&Builder::new().reporting(painted(1)).reporting(painted_page()).table);
    let _ = fired(&mut engine, START + 1);
    let stream = slow_drawing();
    feed(&mut engine, START + 2, &regis(1, 0, &stream));
    let whole = regis_fed(&stream, usize::MAX);
    let (first, seen) = until_whole(&mut engine, START + 2, &whole);
    assert_eq!(seen.iter().filter(|(_, rule)| *rule == 0).count(), 1, "{seen:?}");
    assert_eq!(seen.iter().filter(|(_, rule)| *rule == 1).count(), 1, "{seen:?}");

    // Its contents defined again, in another colour.
    let redefined = b"D(B1)W(I5)P[0,0]V[99,0]D(E)D(C)";
    let tick = first + 20;
    feed(&mut engine, tick, &regis(1, stream.len(), redefined));
    let mut all = stream.clone();
    all.extend_from_slice(redefined);
    let (again, seen) = until_whole(&mut engine, tick, &regis_fed(&all, usize::MAX));
    assert!(again > tick + 30, "it painted again over ticks");
    assert!(seen.contains(&(again, 0)), "the drawing's event again when it ends: {seen:?}");
    assert_eq!(seen.iter().filter(|(_, rule)| *rule == 0).count(), 1, "{seen:?}");

    // A new generation: a new terminal, watched as the table says.
    let tick = again + 20;
    feed(&mut engine, tick, &regis(2, 0, &stream));
    let (third, seen) = until_whole(&mut engine, tick, &whole);
    assert!(seen.contains(&(third, 0)), "the drawing's event for the new generation: {seen:?}");
    assert_eq!(seen.iter().filter(|(_, rule)| *rule == 1).count(), 1, "and the page's: {seen:?}");
}

/// REQ-7026 as DEC-7031 makes it more exact ("a table's loading fires
/// painted for drawings already painted", since a channel sends the
/// stream before the table): a table loaded after its picture was drawn
/// is told at once that the drawing and the picture are painted.
#[test]
fn a_table_loaded_after_its_picture_is_told_the_drawing_and_the_picture_are_painted() {
    let stream = format!("{}{}", String::from_utf8(v1_lines(3)).unwrap(), "D(B1)W(I3)P[0,0]V[99,0]D(E)D(S1(X10,Y300),C)").into_bytes();
    let mut engine = Engine::new();
    open(&mut engine, START);
    feed(&mut engine, START, &regis(1, 0, &stream));
    wake(&mut engine, START);
    let mut seen = reports(&load(&mut engine, START + 5, &watching_both())).iter().map(|report| report.rule).collect::<Vec<_>>();
    seen.extend(fired(&mut engine, START + 5));
    seen.extend(fired(&mut engine, START + 6));
    seen.sort_unstable();
    assert_eq!(seen, [0, 1], "the drawing's and the picture's events, once each");
    for tick in 7..30 {
        assert_eq!(fired(&mut engine, START + tick), [], "tick {tick}: once");
    }
}

/// REQ-7026, DEC-7031 item 5: the page's event fires at most once a tick;
/// bytes drawn again in that tick owe it again, and it fires in the next
/// tick, which the loop wakes for; and again after later bytes are drawn.
#[test]
fn the_pages_event_fires_at_most_once_a_tick_and_again_after_later_bytes() {
    let mut engine = opened(&Builder::new().reporting(painted_page()).table);
    let _ = fired(&mut engine, START + 1);
    feed(&mut engine, START + 2, &regis(1, 0, b"P[0,0]V[100,0]"));
    feed(&mut engine, START + 2, &regis(1, 14, b"P[0,10]V[100,10]"));
    assert_eq!(fired(&mut engine, START + 2), [0], "once in the tick, for two messages drawn in it");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START + 2)), Some(due_at(START + 3)), "owed again: the loop wakes at the next tick");
    assert_eq!(fired(&mut engine, START + 3), [0], "the event owed, in the next tick");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START + 3)), None, "nothing owed");
    assert_eq!(fired(&mut engine, START + 4), []);
    feed(&mut engine, START + 10, &regis(1, 30, b"P[0,20]V[100,20]"));
    assert_eq!(fired(&mut engine, START + 10), [0], "again after later bytes are drawn");
}

/// REQ-7026, DEC-7031 item 5: the page's event is not given while the
/// page's frame is open, and is given when the frame ends.
#[test]
fn the_pages_event_waits_while_its_frame_is_open() {
    let mut engine = opened(&Builder::new().reporting(painted_page()).table);
    let _ = fired(&mut engine, START + 1);
    feed(&mut engine, START + 2, &regis(1, 0, b"P[0,0]V[100,0]"));
    feed(&mut engine, START + 2, &regis(1, 14, b"P[0,10]V[100,10]"));
    assert_eq!(fired(&mut engine, START + 2), [0]);
    // Owed for the second message, and then the page's frame opens.
    feed(&mut engine, START + 2, &frame(FrameMark::Begin, 4));
    for tick in 3..10 {
        assert_eq!(fired(&mut engine, START + tick), [], "tick {tick}: the frame is open");
    }
    feed(&mut engine, START + 10, &frame(FrameMark::End, 4));
    assert_eq!(fired(&mut engine, START + 10), [0], "given at the frame's end");
}

/// REQ-6536 item 7, REQ-7026: a key pressed during a replay fires its
/// rule as it is handled, at the latest in the next tick, and the replay
/// goes on at the line's pace.
#[test]
fn a_key_during_a_replay_fires_its_rule_by_the_next_tick() {
    use neoteletext_rules::table::{Key, KeyKind, KeyMatch};
    let keys = KeyMatch::of(&[Key::of(KeyKind::Character, u32::from('a'))]);
    let table = Builder::new().reporting(Trigger::Key { keys, repeat: false }).reporting(painted_page()).table;
    let mut engine = opened(&table);
    let _ = fired(&mut engine, START + 1);
    let stream = v1_lines(30);
    feed(&mut engine, START + 2, &paint(SheetKind::Regis, 1, SPEED));
    feed(&mut engine, START + 2, &regis(1, 0, &stream));
    for tick in 2..20 {
        assert_eq!(fired(&mut engine, START + tick), []);
    }
    engine.set_now_us(due_at(START + 20) + 100);
    let input = RuleInput::Key { action: KeyAction::Press, named: false, code: u32::from('a'), base: None, mods: 0 };
    let mut seen: Vec<u8> =
        reports(&engine.rule_message(ToDecoder::RuleInput { page: OPEN, sequence: 1, input }).unwrap()).iter().map(|report| report.rule).collect();
    seen.extend(fired(&mut engine, START + 21));
    assert_eq!(seen, [0], "the key's rule, by the next tick");
    assert_eq!(regis_shown(&mut engine, START + 21), regis_fed(&stream, delivered(19, SPEED)), "and the replay went on at the line's pace");
}

/// REQ-7026, REQ-6523 ("a painted event is a local event"): rules on
/// `painted` that only count send nothing to the server, though they fire.
#[test]
fn a_painted_event_sends_nothing_to_the_server_unless_a_rule_reports() {
    let table = Builder::new().counting(painted(1)).counting(painted_page()).table;
    let mut engine = opened(&table);
    let mut produced = wake(&mut engine, START + 1);
    feed(&mut engine, START + 2, &paint_both());
    feed(&mut engine, START + 2, &regis(1, 0, &base_and_drawing()));
    for tick in 2..1500 {
        produced.extend(wake(&mut engine, START + tick));
    }
    assert_eq!(regis_shown(&mut engine, START + 1500), regis_fed(&base_and_drawing(), usize::MAX), "painted to the end");
    assert_eq!(engine.rules.counters().unwrap().fired, 2, "both rules fired");
    let to_server: Vec<_> = produced.iter().filter(|message| matches!(message, FromDecoder::Report(_) | FromDecoder::PageState { .. })).collect();
    assert!(to_server.is_empty(), "nothing for the server: {to_server:?}");
}

// ---------------------------------------------------------------------------
// Hostile records
// ---------------------------------------------------------------------------

/// DEC-7031 item 1: at most four PAINT records wait, one a sub-page; a
/// fifth for another sub-page drops the lowest sub-page's, whose stream is
/// then painted at once, and the other four are paced. The same for
/// NAPLPS.
#[test]
fn more_than_four_pending_paint_records_drop_the_lowest_sub_pages() {
    let stream = v1_lines(20);
    let mut engine = Engine::new();
    for sub in 1..=5 {
        feed(&mut engine, START, &paint_of(PAGE, sub, SheetKind::Regis, 1, Some(SPEED), None));
    }
    for sub in 1..=5 {
        feed(&mut engine, START, &regis_of(PAGE, sub, 1, 0, &stream));
        let paced = regis_shown_of(&mut engine, PAGE, sub, START).count(3) == 0;
        assert_eq!(paced, sub != 1, "ReGIS sub-page {sub}");
    }

    let stream = naplps_rectangles(12);
    let mut engine = Engine::new();
    for sub in 1..=5 {
        feed(&mut engine, START, &paint_of(PAGE, sub, SheetKind::Naplps, 1, Some(SPEED), None));
    }
    for sub in 1..=5 {
        feed(&mut engine, START, &naplps_of(PAGE, sub, 1, 0, &stream));
        engine.show_pictures_again(PAGE);
        let paced = shown_of(engine.naplps().take(PAGE, Some(sub))).count(3) == 0;
        assert_eq!(paced, sub != 1, "NAPLPS sub-page {sub}");
    }
}

/// DEC-7031 item 1: a PAINT record for a page with no stream holds nothing
/// that is drawn or woken for, and does not touch another page's stream.
#[test]
fn a_paint_record_for_a_page_with_no_stream_changes_nothing() {
    let lonely = 0x200;
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint_of(lonely, 0, SheetKind::Regis, 1, Some(SPEED), Some(7)));
    feed(&mut engine, START, &paint_of(lonely, 0, SheetKind::Naplps, 1, Some(SPEED), None));
    wake(&mut engine, START);
    assert_eq!(engine.animation_due_us(&[lonely, PAGE], due_at(START)), None, "nothing to wake for");
    assert!(engine.regis().take(lonely, None, due_at(START)).is_none() && engine.naplps().take(lonely, None).is_none(), "nothing to draw");
    let stream = v1_lines(10);
    feed(&mut engine, START + 1, &regis(1, 0, &stream));
    wake(&mut engine, START + 1);
    assert_eq!(regis_shown(&mut engine, START + 1), regis_fed(&stream, usize::MAX), "another page's stream is painted at once");
}

/// A page file as the packets of one pushed message.
fn tti(file: &[u8]) -> Vec<u8> {
    use neoteletext_formats::Item;
    use neoteletext_formats::detect::{AutoConfig, AutoReader, Format};
    use neoteletext_formats::item::Reader;
    let mut packets = Vec::new();
    let mut reader = AutoReader::new(AutoConfig { format: Some(Format::Tti), encode_pages: true, ..AutoConfig::default() });
    let mut take = |item: Item<'_>| {
        if let Item::Packet { packet, .. } = item {
            packets.extend_from_slice(&packet.bytes);
        }
    };
    reader.feed(file, &mut take);
    reader.finish(&mut take);
    packets
}

/// DEC-7031 item 1: a PAINT record for a teletext page changes nothing of
/// it: it is drawn as one with none, and nothing wakes for it.
#[test]
fn a_paint_record_for_a_teletext_page_changes_nothing() {
    let page = b"PN,10100\nSC,0000\nPS,C000\nOL,1,PAINTED TEXT\n";
    let mut plain = Engine::new();
    feed(&mut plain, START, &tti(page));
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint_of(PAGE, 0, SheetKind::Regis, 1, Some(SPEED), Some(7)));
    feed(&mut engine, START, &paint_of(PAGE, 0, SheetKind::Naplps, 1, Some(SPEED), None));
    let arrived = feed(&mut engine, START, &tti(page));
    assert_eq!(arrived.iter().map(|(page, _)| *page).collect::<Vec<_>>(), [PAGE], "the page arrives as it would");
    wake(&mut engine, START);
    let (drawn, plain_drawn) = (engine.render(PAGE).expect("drawn"), plain.render(PAGE).expect("drawn"));
    assert!((&drawn.indices, &drawn.palette) == (&plain_drawn.indices, &plain_drawn.palette), "the same page as one with no PAINT record");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START)), None, "nothing to wake for");
}

/// DEC-7031 items 2, 3 and 5: a stream paced at a line's speed that grows
/// past the 262,144 bytes a replay holds is paced no longer: everything
/// is drawn at once, nothing waits, and the page's event fires.
#[test]
fn a_paced_stream_over_the_hold_ends_its_pacing_and_is_drawn_whole() {
    const PIECE: usize = RECORD_PIECE;
    let pieces = regis::limits::MAX_REPLAY_HELD_BYTES / PIECE + 1;
    let mut stream = vec![b' '; PIECE * pieces];
    stream.extend_from_slice(&v1_lines(10));
    let mut engine = opened(&Builder::new().reporting(painted_page()).table);
    let _ = fired(&mut engine, START + 1);
    feed(&mut engine, START + 2, &paint(SheetKind::Regis, 1, MIN_LINE_SPEED));
    for (index, piece) in stream.chunks(PIECE).enumerate() {
        feed(&mut engine, START + 2, &regis(1, index * PIECE, piece));
    }
    assert_eq!(fired(&mut engine, START + 2), [0], "the page's event: nothing is held");
    assert_eq!(regis_shown(&mut engine, START + 2), regis_fed(&stream, usize::MAX), "ReGIS: everything drawn at once");
    // The record after the one that ended the pacing was drawn in the same
    // tick, so the page's event is owed once more, in the next.
    assert_eq!(fired(&mut engine, START + 3), [0]);
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START + 3)), None, "and nothing waits");

    let mut stream = vec![SO];
    stream.extend(core::iter::repeat_n(0_u8, PIECE * pieces));
    stream.extend_from_slice(&naplps_rectangles(6)[1..]);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Naplps, 1, MIN_LINE_SPEED));
    for (index, piece) in stream.chunks(PIECE).enumerate() {
        feed(&mut engine, START, &naplps(1, index * PIECE, piece));
    }
    wake(&mut engine, START);
    let seen = naplps_shown(&mut engine);
    assert!(seen.count(3) > 0, "NAPLPS: the rectangles drawn");
    assert_eq!(seen, naplps_fed(&stream, usize::MAX), "NAPLPS: everything drawn at once");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(START)), None);
}

/// The packets of an image that names `count` distinct drawings in its
/// `painted` triggers, each rule reporting: built as a table of 32, whose
/// image is then given a 33rd number where the rule that repeats a name
/// holds it, since the engine's writer refuses what its reader refuses.
fn naming(count: u16) -> Vec<u8> {
    assert!((2..=33).contains(&count));
    let named = |last: u16| {
        let mut builder = Builder::new();
        for number in 1..count {
            builder = builder.reporting(painted(number));
        }
        image_of(&builder.reporting(painted(last)).table)
    };
    if count <= 32 {
        return named(count);
    }
    let (first, second) = (named(1), named(2));
    let check = |image: &[u8]| neoteletext_rules::check(image, OPEN.page, OPEN.sub, neoteletext_web::records::body_check);
    assert!(check(&first).is_ok(), "32 distinct drawings, one named twice, are taken");
    let mut image = first.clone();
    let differ: Vec<usize> = (0..first.len()).filter(|&index| first[index] != second[index]).collect();
    assert_eq!(differ.len(), 1, "the last trigger's number is one byte apart");
    image[differ[0]] = u8::try_from(count).unwrap();
    assert!(check(&image).is_err(), "the engine's reader refuses 33");
    image
}

/// DEC-7031 item 4, the rules library's `MAX_PAINT_WATCHED`: a table that
/// names 33 distinct drawings in its `painted` triggers is refused whole,
/// with fault 1, and none of its rules fires; one that names 32 is taken,
/// and each of them fires.
#[test]
fn a_table_naming_thirty_three_painted_drawings_is_refused() {
    let scene = (1..=33).fold(String::new(), |mut scene, number| {
        let _ = write!(scene, "D(B{number})W(I3)P[0,0]V[9,0]D(E)D(S{number}(X{},Y10),C)", number * 12);
        scene
    });
    let mut engine = Engine::new();
    open(&mut engine, START);
    let told = {
        feed(&mut engine, START, &rules_image(&naming(33), 1));
        engine.run_rules(due_at(START))
    };
    assert!(told.iter().any(|message| matches!(message, FromDecoder::RulesFault { fault: 1, .. })), "refused whole, with fault 1: {told:?}");
    feed(&mut engine, START + 1, &regis(1, 0, scene.as_bytes()));
    for tick in 1..10 {
        assert_eq!(fired(&mut engine, START + tick), [], "no rule of a refused table");
    }

    let mut engine = Engine::new();
    open(&mut engine, START);
    feed(&mut engine, START, &rules_image(&naming(32), 1));
    let _ = engine.run_rules(due_at(START));
    feed(&mut engine, START + 1, &regis(1, 0, scene.as_bytes()));
    let mut seen = Vec::new();
    for tick in 1..10 {
        seen.extend(fired(&mut engine, START + tick));
    }
    seen.sort_unstable();
    assert_eq!(seen, (0..32).collect::<Vec<u8>>(), "32 watched, each reported once");
}

// ---------------------------------------------------------------------------
// The loop's wakes
// ---------------------------------------------------------------------------

/// DEC-7031 item 5, ANIM-1: a page paced at 300 bits a second, half a
/// byte a tick, wakes the loop only at the ticks its replay's next byte
/// is due, every second tick, and not at the ticks between.
#[test]
fn a_paced_page_wakes_only_when_its_next_byte_is_due() {
    let stream = v1_lines(3);
    let mut engine = Engine::new();
    feed(&mut engine, START, &paint(SheetKind::Regis, 1, MIN_LINE_SPEED));
    feed(&mut engine, START, &regis(1, 0, &stream));
    wake(&mut engine, START);
    let byte_ticks = LINE_PACE_TICKS / MIN_LINE_SPEED;
    assert_eq!(byte_ticks, 2);
    let mut tick = START;
    for _ in 0..10 {
        let due = engine.animation_due_us(&[PAGE], due_at(tick)).expect("a byte is due");
        assert_eq!(tick_at(due), tick + byte_ticks, "after tick {tick}: woken for the next byte and no sooner");
        tick += byte_ticks;
        wake(&mut engine, tick);
    }
    let end = START + ticks_for(stream.len(), MIN_LINE_SPEED);
    wake(&mut engine, end);
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(end)), None, "the replay over, no wake");
}

/// DEC-7031 item 5: a page with no `painted` rule and nothing due does not
/// wake the loop, whether its table has other rules or it has none, and
/// neither does a page whose events were all given.
#[test]
fn a_page_with_no_painted_rule_and_nothing_due_does_not_wake() {
    use neoteletext_rules::table::{Key, KeyKind, KeyMatch};
    let keys = KeyMatch::of(&[Key::of(KeyKind::Character, u32::from('a'))]);
    let tables = [None, Some(Builder::new().reporting(Trigger::Key { keys, repeat: false }).table), Some(Builder::new().reporting(painted_page()).table)];
    for table in tables {
        let mut engine = match &table {
            Some(table) => opened(table),
            None => Engine::new(),
        };
        let _ = fired(&mut engine, START + 1);
        feed(&mut engine, START + 2, &regis(1, 0, &v1_lines(4)));
        let _ = fired(&mut engine, START + 2);
        let _ = fired(&mut engine, START + 3);
        assert_eq!(engine.animation_due_us(&[PAGE], due_at(START + 3)), None, "table {:?}: no wake", table.as_ref().map(|table| table.rules.len()));
    }
}
