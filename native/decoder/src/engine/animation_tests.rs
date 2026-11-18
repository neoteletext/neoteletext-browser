//! Animation in the decoder (neoteletext-common's `docs/animation.md`;
//! REQ-6536): records and ticks driven through the engine, and the
//! pictures and rectangles that come of them.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "a test reports a failure by panicking, and indexes and counts what it made"
)]

use super::Engine;
use crate::animation::{due_at, tick_at};
use crate::rules::Picture;
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::Item;
use neoteletext_formats::detect::{AutoConfig, AutoReader, Format};
use neoteletext_formats::item::Reader;
use neoteletext_rules::animate::{Operation, OperationKind};
use neoteletext_rules::evaluator::Bounds;
use neoteletext_rules::limits::MAX_POINTER_OBJECTS;
use neoteletext_rules::table::Target;
use neoteletext_web::consts::{MAX_FRAME_BYTES, MAX_FRAME_TICKS};
use neoteletext_web::records::frame::{FrameMark, FrameRecord};
use neoteletext_web::records::naplps_objects::{NaplpsObject, ObjectKind};
use neoteletext_web::records::regis::RegisRecord;
use neoteletext_web::records::sheet::{SheetKind, SheetRecord};
use protocol::regis::{PictureLanguage, REGIS_HEIGHT, REGIS_WIDTH, RegisPicture, unpack};
use std::fmt::Write as _;

const PAGE: u16 = 0x100;

fn address(page: u16) -> PageAddress {
    PageAddress::from_mpp(page).unwrap()
}

/// A page file as the packets of one pushed message.
fn tti(file: &[u8]) -> Vec<u8> {
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

fn stream(page: u16, sub: u16, generation: u16, offset: usize, data: &[u8]) -> RegisRecord<'_> {
    RegisRecord { page: address(page), subcode: SubCode::new(sub), generation, offset: u32::try_from(offset).unwrap(), data }
}

/// A REGIS record as the packets of a pushed message.
pub(crate) fn regis(page: u16, generation: u16, offset: usize, data: &[u8]) -> Vec<u8> {
    let mut packets = Vec::new();
    neoteletext_web::records::regis::encode(stream(page, 0, generation, offset, data), 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
    packets
}

/// A NAPLPS record as the packets of a pushed message.
fn naplps(page: u16, generation: u16, offset: usize, data: &[u8]) -> Vec<u8> {
    let mut packets = Vec::new();
    neoteletext_web::records::regis::encode_naplps(stream(page, 0, generation, offset, data), 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
        .unwrap();
    packets
}

/// An ANIMATE record for sub-page 0 of the page, the one the tests'
/// streams draw: a record names one sub-page (page-rules.md RULE-112).
fn animate(page: u16, operations: &[Operation]) -> Vec<u8> {
    let mut packets = Vec::new();
    neoteletext_web::records::animate::encode(address(page), SubCode::new(0), operations, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
        .unwrap();
    packets
}

/// A FRAME record for sub-page 0.
fn frame(page: u16, mark: FrameMark, identifier: u16) -> Vec<u8> {
    frame_of(page, 0, mark, identifier)
}

/// A FRAME record for one page number and sub-code.
fn frame_of(page: u16, sub: u16, mark: FrameMark, identifier: u16) -> Vec<u8> {
    let mut packets = Vec::new();
    let record = FrameRecord { page: address(page), subcode: SubCode::new(sub), mark, identifier };
    neoteletext_web::records::frame::encode(record, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
    packets
}

fn on(number: u16, kind: OperationKind) -> Operation {
    Operation { target: Target::object(number), kind }
}

/// A receiver's copy of a picture: whole pictures replace it and parts
/// are laid over it, as the interface does.
#[derive(Default)]
pub(crate) struct Receiver {
    pixels: Vec<u8>,
    /// The picture's width: a ReGIS picture's 800, a NAPLPS picture's 480.
    width: usize,
    /// The pixels of the parts taken since it was last asked.
    sent: usize,
    /// Whether a whole picture was taken since it was last asked.
    whole: bool,
}

impl Receiver {
    fn lay(&mut self, picture: &RegisPicture) {
        let area = picture.area;
        let part = unpack(&picture.packed);
        if picture.whole() {
            self.pixels = part;
            self.width = usize::from(picture.language.width());
            self.whole = true;
        } else {
            let (left, across) = (usize::from(area.x), usize::from(area.width));
            for (row, line) in part.chunks_exact(across.max(1)).enumerate().take(usize::from(area.height)) {
                let start = (usize::from(area.y) + row) * self.width + left;
                self.pixels[start..start + across].copy_from_slice(line);
            }
            self.sent += area.pixels();
        }
    }

    /// Takes the ReGIS picture of `PAGE` if one is to be sent; whether one was.
    fn regis(&mut self, engine: &mut Engine, now_us: u64) -> bool {
        self.regis_of(engine, PAGE, now_us)
    }

    /// Takes the ReGIS picture of `page` if one is to be sent; whether one was.
    pub(crate) fn regis_of(&mut self, engine: &mut Engine, page: u16, now_us: u64) -> bool {
        let Some(drawn) = engine.regis().take(page, None, now_us) else { return false };
        self.lay(&drawn.picture);
        drawn.more.iter().for_each(|part| self.lay(part));
        true
    }

    /// Takes the NAPLPS picture of `PAGE` if one is to be sent; whether one was.
    fn naplps(&mut self, engine: &mut Engine) -> bool {
        let Some(drawn) = engine.naplps().take(PAGE, None) else { return false };
        self.lay(&drawn.picture);
        drawn.more.iter().for_each(|part| self.lay(part));
        true
    }

    pub(crate) fn at(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * self.width + x]
    }

    fn count(&self, colour: u8) -> usize {
        self.pixels.iter().map(|&pixel| usize::from(pixel == colour)).sum()
    }

    /// The pixels sent as parts, and whether a whole picture was, since the last call.
    pub(crate) fn take_sent(&mut self) -> (usize, bool) {
        (core::mem::take(&mut self.sent), core::mem::take(&mut self.whole))
    }
}

/// A filled square of 20 pixels in colour `colour`, as a drawing's contents.
pub(crate) fn square(number: u16, colour: u8) -> String {
    format!("D(B{number})W(I{colour})F(V[0,0][19,0][19,19][0,19])D(E)")
}

/// ANIM-6, ANIM-5, ANIM-9, ANIM-22 (REQ-6536): an ANIMATE record moves a
/// ReGIS numbered drawing with no byte of its stream sent again, and only
/// the rectangles it left and reached are sent; the rule table reads its
/// new position and bounds; and a commit that then sets the drawing's `X`
/// is the last writer, so the position the record gave is forgotten.
#[test]
fn a_regis_drawing_moved_by_an_animate_record_is_sent_as_its_damage() {
    let mut engine = Engine::new();
    let scene = format!("P[10,10]V[700,400]{}D(S1(X100,Y50,I1),C)", square(1, 3));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    let mut receiver = Receiver::default();
    assert!(receiver.regis(&mut engine, 0) && receiver.take_sent().1, "the first picture is whole");
    assert_eq!(receiver.at(105, 55), 3);

    engine.feed_whole(true, true, &animate(PAGE, &[on(1, OperationKind::Move { relative: false, x: 400, y: 300 })]));
    assert!(receiver.regis(&mut engine, 0));
    let (sent, whole) = receiver.take_sent();
    assert!(!whole && sent <= 2 * 40 * 40, "{sent} pixels sent for a move of a drawing of 20 by 20");
    assert_eq!((receiver.at(105, 55), receiver.at(405, 305)), (0, 3), "it left its place and reached the new one");

    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y, state.shown), (400, 300, true));
    assert_eq!(state.bounds, Some(Bounds { left: 400, top: 300, right: 420, bottom: 320 }), "ANIM-13: its rectangle where it is");
    assert_eq!(engine.animation_picture(PAGE).size(), (800, 480), "RULE-13: a ReGIS picture");

    // A relative move adds to the position in force.
    engine.feed_whole(true, true, &animate(PAGE, &[on(1, OperationKind::Move { relative: true, x: -100, y: 5 })]));
    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y), (300, 305));

    // The stream writes the same thing: the last writer wins.
    engine.feed_whole(true, true, &regis(PAGE, 1, scene.len(), b"D(S1(X10),C)"));
    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y), (10, 50), "the commit's X forgot the position the record gave");
    assert!(receiver.regis(&mut engine, 0));
    assert_eq!((receiver.at(305, 310), receiver.at(15, 55)), (0, 3));
}

/// Renderer parity PAR-32 (decision 67): while a page is held its
/// picture is not replaced, though its records are still read; its cursor
/// and animation need no tick; at the release the latest is shown at once.
#[test]
fn a_held_pictures_records_are_read_and_shown_only_at_the_release() {
    use protocol::decoder::View;
    let mut engine = Engine::new();
    let scene = format!("{}D(S1(X100,Y50,I1),C)", square(1, 3));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    let mut receiver = Receiver::default();
    let take = |engine: &mut Engine, receiver: &mut Receiver| {
        let (regis, _) = engine.take_pictures(PAGE, None, 0);
        let Some(drawn) = regis else { return false };
        receiver.lay(&drawn.picture);
        drawn.more.iter().for_each(|part| receiver.lay(part));
        true
    };
    assert!(take(&mut engine, &mut receiver));
    assert_eq!(receiver.at(105, 55), 3);
    engine.set_view(View { page: PAGE, hold: true, ..View::default() });
    assert!(engine.held(PAGE));
    engine.feed_whole(true, true, &regis(PAGE, 1, scene.len(), b"D(S1(X400,Y300),C)"));
    engine.feed_whole(true, true, &animate(PAGE, &[on(1, OperationKind::Cycle { first: 0, last: 1, period: 4, once: false })]));
    assert!(!take(&mut engine, &mut receiver), "held: the picture shown is not replaced");
    assert_eq!(receiver.at(105, 55), 3, "it still shows the drawing where it was");
    assert_eq!(engine.animation_due_us(&[PAGE], 0), None, "nothing of a held picture needs a tick");
    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y), (400, 300), "the records were read meanwhile");
    engine.set_view(View { page: PAGE, ..View::default() });
    assert!(take(&mut engine, &mut receiver), "released: the latest at once");
    assert_eq!((receiver.at(105, 55), receiver.at(405, 305)), (0, 3));
}

/// Renderer parity PAR-34: twenty ReGIS sub-pages visited, more than the
/// four live terminals hold, are each shown again from the source kept,
/// with no record sent again; so is a NAPLPS sub-page with its objects and
/// its cursor; and a sub-page's drawing links go to the trusted core again.
#[test]
fn twenty_picture_sub_pages_visited_are_shown_again_from_the_sources_kept() {
    use neoteletext_web::records::drawing_links::{DrawingLink, encode};
    const VISITED: u16 = 20;
    let mut engine = Engine::new();
    let take = |engine: &mut Engine, page: u16| {
        engine.show_pictures_again(page);
        let (regis, naplps) = engine.take_pictures(page, None, 0);
        regis.or(naplps).map(|drawn| unpack(&drawn.picture.packed))
    };
    for page in PAGE..PAGE + VISITED {
        let scene = format!("D(B1)W(I3)F(V[0,0][19,0][19,19][0,19])D(E)D(S1(X{},Y50),C)", 100 + (page - PAGE) * 10);
        engine.feed_whole(true, true, &regis(page, 1, 0, scene.as_bytes()));
        assert!(take(&mut engine, page).is_some(), "page {page:x} visited");
    }
    let mut packets = Vec::new();
    encode(address(PAGE), SubCode::new(0), &[DrawingLink { drawing: 1, target: b"200" }], 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
        .unwrap();
    engine.feed_whole(true, true, &packets);
    let _ = engine.regis().take_links();
    assert!(!engine.regis().has(PAGE), "the live terminals no longer hold the first");
    for page in PAGE..PAGE + VISITED {
        let pixels = take(&mut engine, page).unwrap_or_else(|| panic!("page {page:x} shown again"));
        let x = 105 + usize::from(page - PAGE) * 10;
        assert_eq!(pixels[55 * REGIS_WIDTH + x], 3, "page {page:x}: its own picture");
    }
    assert_eq!(engine.regis().take_links(), [(PAGE, 0, vec![(1, "200".to_owned())])], "its links again, for the trusted core");

    // A NAPLPS sub-page with its objects and its cursor.
    let mut bytes = vec![DEF_MACRO, 0x20];
    bytes.extend(rectangle(3, 0, 0, 32, 32));
    bytes.extend([END, 0x3E, 0x40]);
    let naplps_page = 0x300;
    engine.feed_whole(true, true, &naplps(naplps_page, 1, 0, &bytes));
    let mut packets = Vec::new();
    neoteletext_web::records::naplps_objects::encode(
        (address(naplps_page), SubCode::new(0), 1),
        &[object(1, ObjectKind::Macro, (100, 100), &[0x20])],
        0,
        0,
        &mut |packet| {
            packets.extend_from_slice(&packet.bytes);
        },
    )
    .unwrap();
    engine.feed_whole(true, true, &packets);
    let mut packets = Vec::new();
    let cursor = neoteletext_web::records::regis_cursor::RegisCursorRecord { page: address(naplps_page), subcode: SubCode::new(0), drawing: 1 };
    neoteletext_web::records::regis_cursor::encode(cursor, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
    engine.feed_whole(true, true, &packets);
    let first = take(&mut engine, naplps_page).unwrap();
    // Its live terminal goes, as a page closed drops it (RULE-111).
    engine.naplps().forget(naplps_page, 0);
    assert!(!engine.naplps().has(naplps_page));
    assert_eq!(take(&mut engine, naplps_page), Some(first), "drawn again from its stream and objects");
    assert_eq!(engine.naplps().cursor(naplps_page, 0), 1, "with its cursor");
    // The channel sends the same stream again on the page's next selection:
    // it is drawn again from its first byte, not lost as a gap.
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, b"D(B1)W(I5)F(V[0,0][19,0][19,19][0,19])D(E)D(S1(X100,Y50),C)"));
    assert_eq!(take(&mut engine, PAGE).unwrap()[55 * REGIS_WIDTH + 105], 5, "the stream sent again is drawn");
}

/// Renderer parity PAR-36 item 1 (PAR-8): a static channel's sub-page file,
/// fetched, carries the sub-page's packets and then its picture's records
/// as packets 4/31; they are read as a live channel's are, by the same
/// readers, so a ReGIS and a NAPLPS sub-page are drawn, with links and a
/// cursor, though nothing was pushed.
#[test]
fn a_static_channels_fetched_file_draws_its_picture_records() {
    use neoteletext_web::records::drawing_links::{DrawingLink, encode};
    let mut engine = Engine::new();
    let mut file = tti(b"PN,10000\nSC,0000\nPS,8000\nOL,1,A picture page\n\n");
    file.extend(regis(PAGE, 1, 0, b"D(B1)W(I3)F(V[0,0][19,0][19,19][0,19])D(E)D(S1(X100,Y50),C)"));
    encode(address(PAGE), SubCode::new(0), &[DrawingLink { drawing: 1, target: b"200" }], 0, 0, &mut |packet| file.extend_from_slice(&packet.bytes)).unwrap();
    engine.feed_whole(true, false, &file);
    let (regis, _) = engine.take_pictures(PAGE, None, 0);
    let pixels = unpack(&regis.expect("the fetched file's ReGIS is drawn").picture.packed);
    assert_eq!(pixels[55 * REGIS_WIDTH + 105], 3);
    assert_eq!(engine.regis().take_links(), [(PAGE, 0, vec![(1, "200".to_owned())])], "and its links read");
    let naplps_page = 0x200;
    let mut bytes = vec![DEF_MACRO, 0x20];
    bytes.extend(rectangle(3, 0, 0, 32, 32));
    bytes.extend([END, 0x3E, 0x40]);
    let mut file = tti(b"PN,20000\nSC,0000\nPS,8000\nOL,1,Another\n\n");
    file.extend(naplps(naplps_page, 1, 0, &bytes));
    engine.feed_whole(true, false, &file);
    let (_, naplps) = engine.take_pictures(naplps_page, None, 0);
    assert!(naplps.is_some(), "the fetched file's NAPLPS is drawn");
}

/// RULE-26, ANIM-6 (REQ-6536): through the picture the rule table's host
/// is given, a rule's actions hide, show and re-look a ReGIS drawing, and
/// set its frame; its conditions read the state back; an operation on a
/// drawing that does not exist does nothing and is counted.
#[test]
fn a_rule_shows_hides_looks_and_frames_a_regis_drawing() {
    let mut engine = Engine::new();
    let scene = format!("{}{}{}{}D(S1(X100,Y50,I1,N2),S2(V0),S5(X300,Y50,F(3,4)),S3(V0),S4(V0),C)", square(1, 3), square(2, 5), square(3, 6), square(4, 7));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    let mut receiver = Receiver::default();
    assert!(receiver.regis(&mut engine, 0) && receiver.take_sent().1);
    assert_eq!((receiver.at(105, 55), receiver.at(305, 55)), (3, 6), "drawing 1 itself, and drawing 5's frame 0");

    let act = |engine: &mut Engine, number: u16, kind: OperationKind| engine.animation_picture(PAGE).animate(on(number, kind), 0);
    assert!(act(&mut engine, 1, OperationKind::Hide));
    let hidden = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert!(!hidden.shown && hidden.bounds.is_none(), "hidden, with no rectangle");
    assert!(receiver.regis(&mut engine, 0));
    assert_eq!(receiver.at(105, 55), 0, "the hidden drawing is not painted");
    assert!(!receiver.take_sent().1, "and only its rectangle was sent");

    assert!(act(&mut engine, 1, OperationKind::Show));
    assert!(act(&mut engine, 1, OperationKind::Look(3)));
    assert!(act(&mut engine, 5, OperationKind::Frame(1)));
    assert!(receiver.regis(&mut engine, 0));
    assert_eq!((receiver.at(105, 55), receiver.at(305, 55)), (5, 7), "the alternative look, and frame 1");
    let looked = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((looked.shown, looked.look), (true, 3));
    assert_eq!(engine.animation_picture(PAGE).target(Target::object(5)).unwrap().frame, 1);

    assert!(act(&mut engine, 1, OperationKind::Look(0)));
    assert!(receiver.regis(&mut engine, 0));
    assert_eq!(receiver.at(105, 55), 3, "the pointer decides the look again");

    assert!(!act(&mut engine, 99, OperationKind::Hide), "no such drawing");
    assert!(engine.animation_picture(PAGE).target(Target::object(99)).is_none());
    assert!(engine.animation_picture(PAGE).target(Target::control(1)).is_none(), "a ReGIS picture has no controls");
    assert_eq!(engine.take_animation_counters(PAGE).skipped, 1);
    assert_eq!(engine.animation_picture(PAGE).objects_at(105, 55, &mut [0; MAX_POINTER_OBJECTS]), 0, "the trusted core says which drawing the pointer is on");
}

/// ANIM-14, ANIM-1 (REQ-6536): a `cycle` makes the decoder change a ReGIS
/// drawing's frame by itself, at the period's ticks and no others, and
/// the loop is woken for exactly those; at most 16 objects cycle, and a
/// 17th `cycle` does nothing and is counted.
#[test]
fn a_cycle_changes_a_regis_drawings_frame_on_the_clock() {
    let mut engine = Engine::new();
    let cyclers = (10..27).fold(String::new(), |mut all, number| {
        let _ = write!(all, "S{number}(X{},Y200,F(3,4)),", number * 25);
        all
    });
    let scene = format!("{}{}D(S5(X300,Y50,G2,F(3,4)),S3(V0),S4(V0),{cyclers}C)", square(3, 6), square(4, 7));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    let mut receiver = Receiver::default();
    assert!(receiver.regis(&mut engine, 0));
    assert_eq!(engine.animation_due_us(&[PAGE], 0), None, "nothing moves: the decoder sleeps until it is written to");

    let cycle = OperationKind::Cycle { first: 0, last: 1, period: 6, once: false };
    engine.feed_whole(true, true, &animate(PAGE, &[on(5, cycle)]));
    // The cycle starts at the tick after the one its record was applied in.
    let start = engine.regis().next_tick(PAGE, None, 0).unwrap();
    assert!(start >= 1 && start <= tick_at(engine.now_us()) + 1);
    assert!(engine.animation_picture(PAGE).target(Target::object(5)).unwrap().cycling);
    for (tick, frame, colour) in [(start, 0, 6), (start + 5, 0, 6), (start + 6, 1, 7), (start + 11, 1, 7), (start + 12, 0, 6)] {
        engine.animation_step(&[PAGE], due_at(tick));
        let _ = receiver.regis(&mut engine, due_at(tick));
        assert_eq!(engine.animation_picture(PAGE).target(Target::object(5)).unwrap().frame, frame, "tick {tick}");
        assert_eq!(receiver.at(305, 55), colour, "tick {tick}");
    }
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(start + 12)), Some(due_at(start + 18)), "woken for the next frame and no sooner");

    // Sixteen cycle; the seventeenth does not.
    let seventeen: Vec<Operation> = (10..26).map(|number| on(number, cycle)).collect();
    engine.feed_whole(true, true, &animate(PAGE, &seventeen));
    assert_eq!(engine.take_animation_counters(PAGE).skipped, 1, "drawing 5 and fifteen more cycle; the last of these does nothing");
    assert!(!engine.animation_picture(PAGE).target(Target::object(25)).unwrap().cycling);
    assert!(engine.animation_picture(PAGE).target(Target::object(24)).unwrap().cycling);

    // `once` ends at the last frame, and the decoder then wakes for nothing of it.
    engine.feed_whole(true, true, &animate(PAGE, &(10..25).map(|number| on(number, OperationKind::StopCycle)).collect::<Vec<_>>()));
    engine.feed_whole(true, true, &animate(PAGE, &[on(5, OperationKind::Cycle { first: 0, last: 1, period: 3, once: true })]));
    let start = engine.regis().next_tick(PAGE, None, 0).unwrap();
    engine.animation_step(&[PAGE], due_at(start + 3));
    let state = engine.animation_picture(PAGE).target(Target::object(5)).unwrap();
    assert_eq!((state.frame, state.cycling), (1, false));
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(start + 3)), None);
}

/// ANIM-15 (REQ-6536): a drawing with a paint rate is painted gradually
/// as the decoder tells the renderer the ticks that pass, and the loop is
/// woken each tick while it paints; with the viewer's setting "Paint
/// drawings gradually" off it is whole at once.
#[test]
fn a_paint_rate_is_played_on_the_clock_and_the_setting_turns_it_off() {
    // A hundred lines of 700 pixels at 100 work units a second.
    let lines = (0..100).fold(String::new(), |mut all, row| {
        let _ = write!(all, "P[0,{}]V[699,{}]", row * 4, row * 4);
        all
    });
    let scene = format!("D(B1)W(I3){lines}D(E)D(S1(X10,Y10,T1),C)");
    let inked = |engine: &mut Engine, now: u64| {
        let mut receiver = Receiver::default();
        engine.show_pictures_again(PAGE);
        assert!(receiver.regis(engine, now));
        receiver.count(3)
    };
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    engine.animation_step(&[PAGE], due_at(1000));
    let first = inked(&mut engine, due_at(1000));
    assert!(first < 70_000 / 2, "{first} pixels at once of a drawing painted at 100 units a second");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(1000)), Some(due_at(1001)), "woken at the next tick while it paints");
    engine.animation_step(&[PAGE], due_at(1600));
    let later = inked(&mut engine, due_at(1600));
    assert!(later > first, "ten seconds on, more is painted: {first} then {later}");

    engine.set_paint_gradually(false);
    assert_eq!(inked(&mut engine, due_at(1600)), 70_000, "the setting off: whole at once");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(1600)), None);
    // And a drawing committed afterwards is whole at its commit.
    engine.feed_whole(true, true, &regis(PAGE, 2, 0, scene.as_bytes()));
    assert_eq!(inked(&mut engine, due_at(1600)), 70_000);
}

/// ANIM-17, page-rules.md RULE-112 (REQ-6536, REQ-6615): a ReGIS sheet
/// is one sub-page's, sent once and fed to that sub-page's renderer
/// before its own stream, which names the sheet's hidden drawings; a new
/// stream generation keeps the sheet; another sub-code of the same page
/// number is another page, and is fed no sheet of this one's.
#[test]
fn a_regis_sheet_is_fed_before_its_own_sub_pages_stream_alone() {
    let mut engine = Engine::new();
    let sheet = format!("{}D(S9(V0),C)", square(9, 5));
    let mut packets = Vec::new();
    let record = SheetRecord { page: address(PAGE), subcode: SubCode::new(1), kind: SheetKind::Regis, generation: 1, offset: 0, data: sheet.as_bytes() };
    neoteletext_web::records::sheet::encode(record, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
    engine.feed_whole(true, true, &packets);
    for (sub, generation, sheeted) in [(1_u16, 1_u16, true), (2, 1, false), (1, 2, true)] {
        let mut packets = Vec::new();
        let uses = stream(PAGE, sub, generation, 0, b"D(S1(X100,Y50,R9),C)");
        neoteletext_web::records::regis::encode(uses, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
        engine.feed_whole(true, true, &packets);
        let drawn = engine.regis().take(PAGE, Some(sub), 0).unwrap();
        let pixel = unpack(&drawn.picture.packed)[55 * REGIS_WIDTH + 105];
        assert_eq!(pixel == 5, sheeted, "sub-page {sub}, generation {generation}: the sheet's drawing used only by its own sub-page");
    }
}

/// ANIM-2, ANIM-3 (REQ-6536): between a FRAME record's begin and its end
/// a page's rows and its ReGIS are held, and nothing of them is
/// presented; at the end both are applied together, so the page and its
/// picture appear on one tick, the picture never cleared between. Another
/// page's packets pass as they arrive.
#[test]
fn a_frame_holds_what_changes_its_page_until_its_end() {
    let mut engine = Engine::new();
    let old = format!("{}D(S1(X100,Y50),C)", square(1, 3));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, old.as_bytes()));
    let mut receiver = Receiver::default();
    assert!(receiver.regis(&mut engine, 0));

    assert_eq!(engine.feed_whole(true, true, &frame(PAGE, FrameMark::Begin, 7)), []);
    assert_eq!(engine.feed_whole(true, true, &tti(b"PN,10000\nSC,0000\nPS,C000\nOL,1,HELD\n")), [], "the page's rows are held");
    assert!(engine.render(PAGE).is_none());
    // A whole new generation of the page's ReGIS: the double-buffered swap.
    let new = format!("{}D(S1(X400,Y300),C)", square(1, 5));
    assert_eq!(engine.feed_whole(true, true, &regis(PAGE, 2, 0, new.as_bytes())), []);
    assert!(!receiver.regis(&mut engine, 0), "nothing of the new picture, cleared or half drawn, is presented");
    assert_eq!(receiver.at(105, 55), 3, "the page is presented as it was");
    let other = engine.feed_whole(true, true, &tti(b"PN,20000\nSC,0000\nPS,C000\nOL,1,OTHER\n"));
    assert_eq!(other.iter().map(|(page, _)| *page).collect::<Vec<_>>(), [0x200], "another page's packets pass as they arrive");
    assert!(engine.animation_due_us(&[PAGE], 0).is_some(), "the loop wakes each tick while a frame is open, for its limit");

    // An end with another identifier is no end.
    assert_eq!(engine.feed_whole(true, true, &frame(PAGE, FrameMark::End, 8)), []);
    assert!(engine.render(PAGE).is_none());
    let arrived = engine.feed_whole(true, true, &frame(PAGE, FrameMark::End, 7));
    assert_eq!(arrived.iter().map(|(page, _)| *page).collect::<Vec<_>>(), [PAGE], "the page arrives at its frame's end");
    assert!(engine.render(PAGE).is_some());
    assert!(receiver.regis(&mut engine, 0));
    assert_eq!((receiver.at(105, 55), receiver.at(405, 305)), (0, 5), "and its new picture with it");
}

/// RULE-112, DEC-6599 (TODO-6681 defect 1): a frame is of one page number
/// and sub-code together. A frame open for sub-page 1 holds sub-page 1's
/// rows, and sub-page 0 of the same page passes as it arrives. Before, the
/// decoder keyed a frame on the page number alone, and held both.
#[test]
fn a_frame_holds_only_its_own_sub_page() {
    let mut engine = Engine::new();
    assert_eq!(engine.feed_whole(true, true, &frame_of(PAGE, 1, FrameMark::Begin, 3)), []);
    let other = engine.feed_whole(true, true, &tti(b"PN,10000\nSC,0000\nPS,C000\nOL,1,SUB ZERO\n"));
    assert_eq!(other.iter().map(|(page, _)| *page).collect::<Vec<_>>(), [PAGE], "sub-page 0 passes as it arrives");
    assert_eq!(engine.feed_whole(true, true, &tti(b"PN,10001\nSC,0001\nPS,C000\nOL,1,SUB ONE\n")), [], "sub-page 1 is held");
    assert!(!engine.subpage_changes(&[PAGE]).contains(&(PAGE, 2)), "and not stored while its frame is open");
    let _ = engine.feed_whole(true, true, &frame_of(PAGE, 1, FrameMark::End, 3));
    assert_eq!(engine.subpage_changes(&[PAGE]), [(PAGE, 2)], "it is stored at its frame's end");
}

/// ANIM-2 (REQ-6536): a frame also ends, and what it held is applied at
/// once, when a second begin arrives for the page, when the limit of
/// bytes is reached, when 120 ticks pass with no end, when the page stops
/// being shown, and when the connection ends: a fault delays a picture
/// and never loses one. A fifth page's begin is treated as no frame.
#[test]
fn a_frame_that_is_not_ended_is_applied_and_never_lost() {
    let page = b"PN,10000\nSC,0000\nPS,C000\nOL,1,HELD\n";
    let held = |engine: &mut Engine| {
        assert_eq!(engine.feed_whole(true, true, &frame(PAGE, FrameMark::Begin, 1)), []);
        assert_eq!(engine.feed_whole(true, true, &tti(page)), []);
        assert!(engine.render(PAGE).is_none());
    };
    let arrived = |pages: &[(u16, String)]| pages.iter().any(|(page, _)| *page == PAGE);

    // A second begin.
    let mut engine = Engine::new();
    held(&mut engine);
    assert!(arrived(&engine.feed_whole(true, true, &frame(PAGE, FrameMark::Begin, 2))));

    // The limit of bytes.
    let mut engine = Engine::new();
    held(&mut engine);
    let filler = [b' '; 8000];
    let mut offset = 0;
    let mut released = false;
    for _ in 0..=MAX_FRAME_BYTES / filler.len() {
        released |= arrived(&engine.feed_whole(true, true, &regis(PAGE, 1, offset, &filler)));
        offset += filler.len();
    }
    assert!(released, "the frame ended when it held its limit of bytes");

    // The limit of ticks, on the presentation clock.
    let mut engine = Engine::new();
    assert_eq!(engine.animation_step(&[PAGE], due_at(500)), []);
    held(&mut engine);
    assert_eq!(engine.animation_step(&[PAGE], due_at(500 + MAX_FRAME_TICKS - 1)), [], "not a tick sooner");
    assert!(arrived(&engine.animation_step(&[PAGE], due_at(500 + MAX_FRAME_TICKS))));
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(500 + MAX_FRAME_TICKS)), None, "and the loop sleeps again");

    // The page stops being shown.
    let mut engine = Engine::new();
    held(&mut engine);
    engine.animation_showing(&[0x200]);
    assert!(arrived(&engine.animation_step(&[0x200], 0)));

    // The connection ends.
    let mut engine = Engine::new();
    held(&mut engine);
    assert!(arrived(&engine.finish()));

    // Four pages have a frame open; a fifth begin holds nothing.
    let mut engine = Engine::new();
    for other in [0x200, 0x300, 0x400, 0x500] {
        engine.feed_whole(true, true, &frame(other, FrameMark::Begin, 1));
    }
    engine.feed_whole(true, true, &frame(PAGE, FrameMark::Begin, 1));
    assert!(arrived(&engine.feed_whole(true, true, &tti(page))));
}

const SO: u8 = 0x0E;
const DEF_MACRO: u8 = 0x80;
const END: u8 = 0x85;
/// WAIT and its fixed-format byte (ANSI X3.110-1983 section 5.3.2.8).
const WAIT: [u8; 2] = [0x3D, 0x5C];

/// A three-byte coordinate pair in the default domain: (`x`/256, `y`/256)
/// of the unit screen (section 5.3.2.2.3).
fn pair(x: u8, y: u8) -> [u8; 3] {
    let (x, y) = (u16::from(x), u16::from(y));
    let byte = |shift: u16| 0x40 | (u8::try_from((x >> shift) & 7).unwrap() << 3) | u8::try_from((y >> shift) & 7).unwrap();
    [byte(6), byte(3), byte(0)]
}

/// PDI bytes drawing a filled rectangle of colour-map entry `entry` from
/// (`x`, `y`) of size (`width`, `height`), in 256ths of the unit screen:
/// SELECT COLOR, POINT SET absolute, RECTANGLE filled.
fn rectangle(entry: u8, x: u8, y: u8, width: u8, height: u8) -> Vec<u8> {
    let mut bytes = vec![SO, 0x3E, 0x40 | (entry << 2), 0x24];
    bytes.extend(pair(x, y));
    bytes.push(0x31);
    bytes.extend(pair(width, height));
    bytes
}

/// ANIM-20 (REQ-6536; TODO-6409 item 2): an animated NAPLPS page plays. A
/// WAIT of 5 tenths shows the earlier picture for 30 ticks and then what
/// follows it, and the loop is woken for the wait's end; with the
/// viewer's setting off no WAIT delays anything.
#[test]
fn a_naplps_wait_is_played_on_the_clock() {
    let mut animated = rectangle(3, 0, 0, 32, 16);
    animated.extend(WAIT);
    animated.push(0x40 | 5);
    animated.extend(rectangle(6, 128, 100, 32, 16));
    // A command is drawn when the next begins: one more, SELECT COLOR.
    animated.extend([0x3E, 0x40]);

    let mut engine = Engine::new();
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &animated));
    engine.animation_step(&[PAGE], due_at(1000));
    let mut receiver = Receiver::default();
    assert!(receiver.naplps(&mut engine) && receiver.take_sent().1);
    assert!(receiver.count(3) > 1000 && receiver.count(6) == 0, "the picture before the WAIT");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(1000)), Some(due_at(1030)), "woken when the wait ends, and not sixty times a second");
    engine.animation_step(&[PAGE], due_at(1029));
    assert!(!receiver.naplps(&mut engine), "29 ticks on, still the earlier picture");
    engine.animation_step(&[PAGE], due_at(1030));
    assert!(receiver.naplps(&mut engine));
    assert!(receiver.count(6) > 1000, "30 ticks on, what followed the WAIT");
    assert!(!receiver.take_sent().1, "sent as the part that changed");
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(1030)), None);

    let mut engine = Engine::new();
    engine.set_paint_gradually(false);
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &animated));
    let mut receiver = Receiver::default();
    assert!(receiver.naplps(&mut engine));
    assert!(receiver.count(3) > 1000 && receiver.count(6) > 1000, "the setting off: nothing waits");
}

/// ANIM-20 (REQ-6536): a BLINK of 5 tenths on and 5 off changes the
/// picture's colours every 30 ticks, and the loop is woken for each
/// change.
#[test]
fn a_naplps_blink_changes_the_colours_every_thirty_ticks() {
    let mut blinking = rectangle(3, 0, 0, 32, 16);
    // BLINK entry 3 to entry 7's colour: on 5 tenths, off 5, no start delay.
    blinking.extend([0x3F, 0x40 | (7 << 2), 0x45, 0x45, 0x40]);
    // A command is carried out when the next begins: one more, SELECT COLOR.
    blinking.extend([0x3E, 0x40 | (3 << 2)]);
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &blinking));
    engine.animation_step(&[PAGE], due_at(1000));
    let first = engine.naplps().take(PAGE, None).unwrap().picture.palette;
    let mut changes = 0;
    let mut last = first;
    for tick in 1001..=1090 {
        let due = engine.animation_due_us(&[PAGE], due_at(tick - 1)).unwrap();
        assert!(due >= due_at(tick), "tick {tick}");
        engine.animation_step(&[PAGE], due_at(tick));
        if let Some(drawn) = engine.naplps().take(PAGE, None) {
            assert_ne!(drawn.picture.palette, last, "tick {tick}: sent because its colours changed");
            assert_eq!((tick - 1000) % 30, 0, "tick {tick}: a change falls on the interval's tick");
            last = drawn.picture.palette;
            changes += 1;
        }
    }
    assert_eq!(changes, 3, "three changes in 90 ticks");
    assert_ne!(first[3], last[3]);
}

/// Renderer parity PAR-33 (REQ-754): with reduced motion on, a NAPLPS
/// WAIT is not waited for and a BLINK shows its first colour steadily,
/// waking the decoder for neither; off, both play on the clock.
#[test]
fn reduced_motion_waits_for_no_naplps_wait_and_shows_a_blinks_first_colour() {
    let mut animated = rectangle(3, 0, 0, 32, 16);
    animated.extend(WAIT);
    animated.push(0x40 | 5);
    animated.extend(rectangle(6, 128, 100, 32, 16));
    animated.extend([0x3E, 0x40]);
    let mut engine = Engine::new();
    engine.set_reduced_motion(true);
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &animated));
    let mut receiver = Receiver::default();
    assert!(receiver.naplps(&mut engine));
    assert!(receiver.count(3) > 1000 && receiver.count(6) > 1000, "nothing waits");

    let mut blinking = rectangle(3, 0, 0, 32, 16);
    blinking.extend([0x3F, 0x40 | (7 << 2), 0x45, 0x45, 0x40]);
    blinking.extend([0x3E, 0x40 | (3 << 2)]);
    let palettes = |reduced: bool| {
        let mut engine = Engine::new();
        engine.set_reduced_motion(reduced);
        engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &blinking));
        engine.animation_step(&[PAGE], due_at(1000));
        let first = engine.naplps().take(PAGE, None).unwrap().picture.palette;
        let due = engine.animation_due_us(&[PAGE], due_at(1000));
        engine.animation_step(&[PAGE], due_at(1030));
        let later = engine.naplps().take(PAGE, None).map(|drawn| drawn.picture.palette);
        (first, due, later)
    };
    let (first, due, later) = palettes(false);
    assert!(due.is_some() && later.is_some_and(|later| later[3] != first[3]), "off: it blinks");
    let (steady, due, later) = palettes(true);
    assert_eq!((due, later), (None, None), "on: nothing changes and nothing wakes");
    assert_eq!(steady[3], first[3], "its first colour");
}

/// Renderer parity PAR-33 (REQ-754): with reduced motion on, a ReGIS
/// drawing's frame cycle shows its first frame and does not advance, a
/// paint rate draws at once, and the cursor drawing shows steadily while
/// keys go to the channel; the channel's commits still apply.
#[test]
fn reduced_motion_stills_regis_cycles_paint_rates_and_the_cursor() {
    // A cycle from frame 1 to 0 shows frame 1, colour 7, and stays.
    let mut engine = Engine::new();
    // Frames 0, 1 and 2 of drawing 5 are drawings 3, 4 and 6, in colours
    // 6, 7 and 5; a cycle of frames 1 and 2 shows frame 1 and stays.
    let scene = format!("{}{}{}D(S5(X300,Y50,G2,F(3,4,6)),S3(V0),S4(V0),S6(V0),C)", square(3, 6), square(4, 7), square(6, 5));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    engine.set_reduced_motion(true);
    let mut receiver = Receiver::default();
    assert!(receiver.regis(&mut engine, 0));
    engine.feed_whole(true, true, &animate(PAGE, &[on(5, OperationKind::Cycle { first: 1, last: 2, period: 6, once: false })]));
    for tick in [10, 16, 22, 100] {
        engine.animation_step(&[PAGE], due_at(tick));
        let _ = receiver.regis(&mut engine, due_at(tick));
        assert_eq!(receiver.at(305, 55), 7, "tick {tick}: its first frame");
    }
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(100)), None, "no wake for a cycle");
    // A commit still applies.
    engine.feed_whole(true, true, &regis(PAGE, 1, scene.len(), b"D(S5(X400),C)"));
    let _ = receiver.regis(&mut engine, due_at(100));
    assert_eq!((receiver.at(305, 55), receiver.at(405, 55)), (0, 7), "the commit moved it");

    // A paint rate draws at once.
    let lines = (0..100).fold(String::new(), |mut all, row| {
        let _ = write!(all, "P[0,{}]V[699,{}]", row * 4, row * 4);
        all
    });
    let mut engine = Engine::new();
    engine.set_reduced_motion(true);
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, format!("D(B1)W(I3){lines}D(E)D(S1(X10,Y10,T1),C)").as_bytes()));
    engine.animation_step(&[PAGE], due_at(1000));
    let mut receiver = Receiver::default();
    assert!(receiver.regis(&mut engine, due_at(1000)));
    assert_eq!(receiver.count(3), 70_000, "whole at once");

    // The cursor drawing shows steadily while keys go to the channel.
    let mut pages = crate::regis::RegisPages::default();
    pages.set_reduced(true);
    let cursor_scene = "D(B1)W(I7)T'Hi'D(E)D(B2)W(I3)V[0,0][8,0]D(E)D(S1(X60,Y200),S2(X78,Y200),C)";
    pages.read(&stream(PAGE, 0, 1, 0, cursor_scene.as_bytes()));
    pages.set_cursor(PAGE, 0, 2);
    pages.set_viewer(PAGE, 0, protocol::regis::RegisViewer { hovered: 0, pressed: 0, typing: true });
    let _ = pages.take(PAGE, Some(0), 0);
    let half = neoteletext_core::consts::presentation_tick_due_us(interface_core::regis::BLINK_HALF_TICKS);
    assert!(pages.take(PAGE, Some(0), half + 1).is_none(), "not hidden for the second half period");
    assert_eq!(pages.owed_us(PAGE, Some(0), half + 1), None, "no wake for a blink");
}

/// Renderer parity PAR-33 (REQ-754): with reduced motion on, a NAPLPS
/// sub-page's cursor object shows steadily while keys go to the channel,
/// and a frame cycle shows its first frame and does not advance.
#[test]
fn reduced_motion_stills_the_naplps_cursor_and_cycles() {
    let mut bytes = vec![DEF_MACRO, 0x20];
    bytes.extend(rectangle(3, 0, 0, 32, 32));
    bytes.push(END);
    bytes.extend([DEF_MACRO, 0x21]);
    bytes.extend(rectangle(6, 0, 0, 32, 32));
    bytes.push(END);
    bytes.extend([DEF_MACRO, 0x22]);
    bytes.extend(rectangle(9, 0, 0, 32, 32));
    bytes.push(END);
    bytes.extend([0x3E, 0x40]);
    let mut engine = Engine::new();
    engine.set_reduced_motion(true);
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &bytes));
    let mut packets = Vec::new();
    let objects = [object(1, ObjectKind::Macro, (100, 100), &[0x20, 0x21, 0x22])];
    neoteletext_web::records::naplps_objects::encode((address(PAGE), SubCode::new(0), 1), &objects, 0, 0, &mut |packet| {
        packets.extend_from_slice(&packet.bytes);
    })
    .unwrap();
    engine.feed_whole(true, true, &packets);
    let mut receiver = Receiver::default();
    assert!(receiver.naplps(&mut engine));
    let at = |receiver: &Receiver| {
        let (x, y) = raster(110, 110);
        receiver.at(x, y)
    };
    // Frames 1 and 2, colours 6 and 9: frame 1 shows, and stays.
    engine.feed_whole(true, true, &animate(PAGE, &[on(1, OperationKind::Cycle { first: 1, last: 2, period: 4, once: false })]));
    for tick in [10, 14, 18, 100] {
        engine.animation_step(&[PAGE], due_at(tick));
        let _ = receiver.naplps(&mut engine);
        assert_eq!(at(&receiver), 6, "tick {tick}: its first frame");
    }
    // The cursor: object 1, with keys going to the channel.
    engine.naplps().set_cursor(PAGE, 0, 1);
    engine.naplps().set_viewer(PAGE, 0, protocol::regis::RegisViewer { hovered: 0, pressed: 0, typing: true });
    for tick in [200, 232, 264, 296] {
        engine.animation_step(&[PAGE], due_at(tick));
        let _ = receiver.naplps(&mut engine);
        assert_eq!(at(&receiver), 6, "tick {tick}: shown steadily");
    }
    assert_eq!(engine.animation_due_us(&[PAGE], due_at(296)), None, "no wake for a blink or a cycle");
}

/// The dot of the teletext raster a NAPLPS picture is carried as (REQ-5819)
/// that shows the pixel (`x`, `y`) of the 640 by 480 picture objects are
/// placed in: its canvas of 480 by 462 lies on lines 19 to 480.
fn raster(x: usize, y: usize) -> (usize, usize) {
    ((x * 480 + 320) / 640, (y * 462 + 240) / 480 + 19)
}

/// A NAPLPS object as its record holds it.
fn object(number: u16, kind: ObjectKind, (x, y): (i16, i16), frames: &[u8]) -> NaplpsObject<'_> {
    let region = kind == ObjectKind::Region;
    NaplpsObject {
        number,
        kind,
        shown: true,
        detectable: true,
        layer: if region { 1 } else { 2 },
        order: 0,
        x,
        y,
        width: 80,
        height: 60,
        origin: (0, if region { 0 } else { 420 }),
        cover: if region { 5 } else { 0 },
        looks: [0; 3],
        frames,
    }
}

/// ANIM-11, ANIM-6, ANIM-14 (REQ-6536; DEC-6522 item 11): a NAPLPS
/// sub-page's objects are composed over its stream's picture; a region
/// can be picked and, hidden, is its cover; a macro object is moved and
/// its frames cycled by the decoder; the pick record sent with the
/// picture names the objects, so the existing pick path makes them
/// clickable; and the objects a pixel is inside are given to the rules.
#[test]
fn naplps_objects_are_composed_picked_hidden_moved_and_cycled() {
    // Macro 2/0 fills in entry 3 and macro 2/1 in entry 6, each a rectangle
    // of 80 by 60 pixels at the bottom left of the unit screen; then the
    // stream's own picture, a rectangle of entry 2.
    let mut bytes = vec![DEF_MACRO, 0x20];
    bytes.extend(rectangle(3, 0, 0, 32, 32));
    bytes.extend([END, DEF_MACRO, 0x21]);
    bytes.extend(rectangle(6, 0, 0, 32, 32));
    bytes.push(END);
    bytes.extend(rectangle(2, 128, 128, 64, 64));
    bytes.extend([0x3E, 0x40]);
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &bytes));
    let mut packets = Vec::new();
    let objects = [object(1, ObjectKind::Macro, (100, 100), &[0x20, 0x21]), object(2, ObjectKind::Region, (320, 100), &[])];
    neoteletext_web::records::naplps_objects::encode((address(PAGE), SubCode::new(0), 1), &objects, 0, 0, &mut |packet| {
        packets.extend_from_slice(&packet.bytes);
    })
    .unwrap();
    engine.feed_whole(true, true, &packets);

    let mut receiver = Receiver::default();
    let drawn = engine.naplps().take(PAGE, None).unwrap();
    let pick = drawn.pick.clone().unwrap();
    receiver.lay(&drawn.picture);
    assert!(receiver.take_sent().1, "the first picture is whole");
    let (at, picked) = (
        |x, y| {
            let (x, y) = raster(x, y);
            receiver.at(x, y)
        },
        |x, y| {
            let (x, y) = raster(x, y);
            pick.at(PictureLanguage::Naplps, x, y).drawing
        },
    );
    assert_eq!(at(110, 110), 3, "the macro object's frame 0, at its place");
    assert_eq!(picked(110, 110), 1, "the object can be picked");
    assert_eq!(picked(330, 110), 2, "and the region");
    assert_eq!(pick.at(PictureLanguage::Naplps, 10, 10).drawing, 0, "nothing else of a NAPLPS picture");
    assert_ne!(at(330, 110), 5, "a shown region changes nothing");

    let mut found = [0; MAX_POINTER_OBJECTS];
    let picture = engine.animation_picture(PAGE);
    assert_eq!(picture.size(), (640, 480), "RULE-13: a NAPLPS picture");
    assert_eq!(picture.objects_at(330, 110, &mut found), 1);
    assert_eq!(found[0], 2);
    let state = picture.target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y, state.layer), (100, 100, 2));
    assert_eq!(state.bounds, Some(Bounds { left: 100, top: 100, right: 180, bottom: 160 }));

    // Hide the region, move the object and cycle its frames.
    let operations = [
        on(2, OperationKind::Hide),
        on(1, OperationKind::Move { relative: true, x: 300, y: 200 }),
        on(1, OperationKind::Cycle { first: 0, last: 1, period: 4, once: false }),
    ];
    engine.feed_whole(true, true, &animate(PAGE, &operations));
    assert!(receiver.naplps(&mut engine));
    let at = |receiver: &Receiver, x, y| {
        let (x, y) = raster(x, y);
        receiver.at(x, y)
    };
    assert_eq!(at(&receiver, 330, 110), 5, "the hidden region is its cover");
    assert_eq!((at(&receiver, 110, 110), at(&receiver, 410, 310)), (0, 3), "the object moved");
    assert!(!receiver.take_sent().1, "only what changed was sent");
    assert_eq!(engine.animation_picture(PAGE).objects_at(330, 110, &mut found), 0, "a hidden region takes no pointer");
    // The cycle starts at the tick after the one its record was applied in.
    let start = engine.naplps().next_tick(PAGE, None, 0).unwrap();
    engine.animation_step(&[PAGE], due_at(start + 4));
    assert!(receiver.naplps(&mut engine));
    assert_eq!(at(&receiver, 410, 310), 6, "its cycle's next frame");

    // New objects replace them all, and forget their animation state.
    let mut packets = Vec::new();
    neoteletext_web::records::naplps_objects::encode((address(PAGE), SubCode::new(0), 1), &objects[..1], 0, 0, &mut |packet| {
        packets.extend_from_slice(&packet.bytes);
    })
    .unwrap();
    engine.feed_whole(true, true, &packets);
    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y, state.cycling), (100, 100, false));
    assert!(engine.animation_picture(PAGE).target(Target::object(2)).is_none());
    assert_eq!(REGIS_HEIGHT, 480);
}

/// Renderer parity PAR-2 and PAR-27: a `REGIS_CURSOR` record, the same
/// bytes a ReGIS sub-page's cursor comes in, read from the channel's pushed
/// message, names an object of a NAPLPS sub-page as its cursor.
#[test]
fn a_regis_cursor_record_names_a_naplps_sub_pages_cursor_object() {
    use neoteletext_web::records::regis_cursor::{RegisCursorRecord, encode};
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, b"A"));
    let mut packets = Vec::new();
    encode(RegisCursorRecord { page: address(PAGE), subcode: SubCode::new(0), drawing: 4 }, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
        .unwrap();
    engine.feed_whole(true, true, &packets);
    assert_eq!(engine.naplps().cursor(PAGE, 0), 4);
}

/// DEC-6638 item 3, TODO-6694 step 3: a `NAPLPS_OBJECTS` record's labels
/// and title, read from the channel's pushed message, reach the renderer;
/// a record of one object, sent after, redefines that object alone and
/// keeps the other and the title.
#[test]
fn a_naplps_objects_records_labels_title_and_one_object_reach_the_renderer() {
    use neoteletext_web::records::naplps_objects::{Contents, LabelledObject, encode_contents};
    let mut bytes = vec![DEF_MACRO, 0x20];
    bytes.extend(rectangle(3, 0, 0, 32, 32));
    bytes.extend([END, 0x3E, 0x40]);
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, &bytes));
    let send = |engine: &mut Engine, contents: &Contents<'_>| {
        let mut packets = Vec::new();
        encode_contents((address(PAGE), SubCode::new(0), 1), contents, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
        engine.feed_whole(true, true, &packets);
    };
    let objects = [
        LabelledObject { object: object(1, ObjectKind::Macro, (100, 100), &[0x20]), label: "Sun" },
        LabelledObject { object: object(2, ObjectKind::Region, (320, 100), &[]), label: "Cloud" },
    ];
    send(&mut engine, &Contents::Whole { objects: &objects, title: Some("Weather") });
    assert_eq!(engine.naplps().title(PAGE), Some("Weather"));
    assert_eq!(engine.naplps().labels(PAGE, 0), [(1, "Sun"), (2, "Cloud")]);
    send(&mut engine, &Contents::One(LabelledObject { object: object(1, ObjectKind::Macro, (200, 300), &[0x20]), label: "Moon" }));
    assert_eq!(engine.naplps().labels(PAGE, 0), [(1, "Moon"), (2, "Cloud")], "one object redefined, the other kept");
    assert_eq!(engine.naplps().title(PAGE), Some("Weather"), "and the title");
    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y), (200, 300), "at its new definition");
    assert!(engine.animation_picture(PAGE).target(Target::object(2)).is_some());
    // The page arrives: its title for the page list is the picture's,
    // not the text of its rows (DEC-6638 item 3).
    assert_eq!(engine.feed_whole(true, true, &tti(b"PN,10000\nSC,0000\nPS,C000\nOL,1,ROW TEXT\n")), [(PAGE, "Weather".to_owned())]);
}

/// TODO-6748, DEC-6638 item 3: a NAPLPS title that comes after its page
/// has arrived, or a new one, is reported for the page list with the
/// latest title; one that comes before arrives with the page and is not
/// reported again, and the same title again is not reported.
#[test]
fn a_naplps_title_after_its_page_arrived_is_reported() {
    use neoteletext_web::records::naplps_objects::{Contents, LabelledObject, encode_contents};
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, b"A"));
    let send = |engine: &mut Engine, title: &str| {
        let mut packets = Vec::new();
        let objects = [LabelledObject { object: object(2, ObjectKind::Region, (320, 100), &[]), label: "" }];
        let contents = Contents::Whole { objects: &objects, title: Some(title) };
        encode_contents((address(PAGE), SubCode::new(0), 1), &contents, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
        engine.feed_whole(true, true, &packets);
    };
    send(&mut engine, "Early");
    assert_eq!(engine.take_retitled(), [], "a page not yet arrived arrives with its title");
    assert_eq!(engine.feed_whole(true, true, &tti(b"PN,10000\nSC,0000\nPS,C000\nOL,1,ROW TEXT\n")), [(PAGE, "Early".to_owned())]);
    send(&mut engine, "Early");
    assert_eq!(engine.take_retitled(), [], "the same title again");
    send(&mut engine, "Weather map");
    send(&mut engine, "Weather maps");
    assert_eq!(engine.take_retitled(), [(PAGE, "Weather maps".to_owned())], "the latest title, once");
    assert_eq!(engine.take_retitled(), []);

    // A page that arrived before its picture had a title.
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &naplps(PAGE, 1, 0, b"A"));
    assert_eq!(engine.feed_whole(true, true, &tti(b"PN,10000\nSC,0000\nPS,C000\nOL,1,ROW TEXT\n")), [(PAGE, "ROW TEXT".to_owned())]);
    send(&mut engine, "Weather map");
    assert_eq!(engine.take_retitled(), [(PAGE, "Weather map".to_owned())]);
}

/// ANIM-7, ANIM-6, ANIM-13 (REQ-6536): a teletext sub-page's SPRITES
/// record gives its sprites; the rule table reads each one's cells and
/// its rectangle in dots; an ANIMATE record moves and hides them, and the
/// page is then drawn again; new sprites forget the state of the old.
#[test]
fn teletext_sprites_are_kept_moved_and_read_by_the_rules() {
    use neoteletext_web::records::sprites::Sprite;

    let mut engine = Engine::new();
    engine.feed_whole(true, true, &tti(b"PN,10000\nSC,0000\nPS,C000\nOL,1,PAGE\n"));
    // A local passive object, number 0: source 1, type 3.
    let reference = ((1_u16 << 11) | (3 << 9)).to_be_bytes();
    let sprites = [Sprite::new(1, true, true, 2, 0, (10, 5), (4, 3), &reference)];
    let send = |engine: &mut Engine, sprites: &[Sprite<'_>]| {
        let mut packets = Vec::new();
        neoteletext_web::records::sprites::encode(address(PAGE), SubCode::new(0), sprites, 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
            .unwrap();
        engine.feed_whole(true, true, &packets);
    };
    send(&mut engine, &sprites);
    assert_eq!(engine.take_animated(&[PAGE]), [PAGE], "the page is drawn again with its sprites");
    assert_eq!(engine.take_animated(&[PAGE]), [], "once");
    assert!(engine.render(PAGE).is_some());

    let picture = engine.animation_picture(PAGE);
    assert_eq!(picture.size(), (480, 500), "RULE-13: a teletext page");
    let state = picture.target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y, state.layer, state.shown), (10, 5, 2, true));
    assert_eq!(state.bounds, Some(Bounds { left: 120, top: 100, right: 168, bottom: 160 }), "4 by 3 cells of 12 by 20 dots at column 10, row 5");
    let mut found = [0; MAX_POINTER_OBJECTS];
    assert_eq!(picture.objects_at(130, 110, &mut found), 1);
    assert_eq!((found[0], picture.objects_at(10, 10, &mut found)), (1, 0));

    let operations = [on(1, OperationKind::Move { relative: true, x: 2, y: 1 }), on(9, OperationKind::Hide)];
    engine.feed_whole(true, true, &animate(PAGE, &operations));
    assert_eq!(engine.take_animated(&[PAGE]), [PAGE]);
    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y), (12, 6));
    assert_eq!(state.bounds.map(|bounds| (bounds.left, bounds.top)), Some((144, 120)));
    assert_eq!(engine.take_animation_counters(PAGE).skipped, 1, "sprite 9 does not exist");

    assert!(engine.animation_picture(PAGE).animate(on(1, OperationKind::Hide), 0));
    let hidden = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert!(!hidden.shown && hidden.bounds.is_none());

    send(&mut engine, &sprites);
    let state = engine.animation_picture(PAGE).target(Target::object(1)).unwrap();
    assert_eq!((state.x, state.y, state.shown), (10, 5, true), "new sprites forget the state of the old");
}
