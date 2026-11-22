//! The painter's calls against the renderer's own pixels (script-api.md
//! section 6.3; work package R2, rdm TODO-7300): what the host writes for
//! each call draws, in the renderer, exactly what ReGIS written by hand
//! for the same thing draws; text is the engine's provided sets'; a
//! macrograph is the receiver's; and each refusal names its bound.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::expect_used, reason = "tests")]

use protocol::script::{ContextFlags, ContextOpen, Level};
use regis::{Picture, Screen, Terminal};

use super::*;
use crate::limits::MAX_BATCH_BYTES;
use crate::limits::regis::{MAX_FILL_VERTICES, MAX_POINTS, MAX_TEXT_CHARS, REGIS_MAX_CONTENTS_BYTES};
use crate::ops::RegisPaint as P;
use crate::random::SEED_BYTES;

#[path = "../../../tests/regis_page/fonts.rs"]
mod fonts;

const PAGE: i32 = 2;
const CHANNEL: i32 = 1;
const NONE: i32 = OPTION_NOT_GIVEN;

/// What a painter program writes as a drawing's contents, in `language`;
/// a macrograph's handle is taken as naming macrograph A.
fn written(program: &[i32], language: &str) -> Result<String, Reply> {
    painter_contents(program, language, false).map(|bytes| bytes.iter().map(|byte| char::from(*byte)).collect())
}

fn contents(program: &[i32]) -> String {
    written(program, "en").unwrap()
}

#[track_caller]
fn refused(program: &[i32]) -> Reply {
    written(program, "en").unwrap_err()
}

#[track_caller]
fn assert_range(reply: &Reply, bound: &str) {
    match reply {
        Reply::Range(message) => assert!(message.starts_with(&format!("{bound}: ")), "the RangeError does not begin with {bound}: {message}"),
        other => panic!("expected a RangeError naming {bound}, got {other:?}"),
    }
}

/// The picture a receiver shows of one drawing set at 200,150 whose
/// contents are `contents`, after `before` outside any drawing: its
/// pixels.
fn painted(before: &str, contents: &[u8]) -> Vec<u8> {
    let mut terminal = Terminal::new();
    fonts::give(&mut terminal);
    let mut screen = Screen::new();
    let mut stream = before.as_bytes().to_vec();
    stream.extend_from_slice(b"D(B1)");
    stream.extend_from_slice(contents);
    stream.extend_from_slice(b"D(E)D(S1(X200,Y150))D(C)");
    let events = terminal.feed_regis(&mut screen, &stream);
    assert!(events.iter().any(|event| matches!(event, RegisEvent::Committed(_))), "{events:?} for {}", String::from_utf8_lossy(&stream));
    let mut picture = Picture::new();
    terminal.compose(&screen, &mut picture);
    picture.canvas().pixels().to_vec()
}

/// Asserts that `program` draws what the hand-written `reference` draws,
/// and that it draws something.
#[track_caller]
fn draws_as(program: &[i32], reference: &str) {
    let ours = contents(program);
    let (drawn, wanted) = (painted("", ours.as_bytes()), painted("", reference.as_bytes()));
    assert!(drawn.iter().any(|pixel| *pixel != 0), "{ours} draws nothing");
    let differing = drawn.iter().zip(&wanted).filter(|(ours, theirs)| ours != theirs).count();
    assert_eq!(differing, 0, "{ours} and {reference} differ in {differing} pixels");
}

/// A `text` call: its options, its language and its string.
fn text(options: [i32; 10], language: &str, string: &str) -> Vec<i32> {
    let mut words = vec![P::TEXT.number()];
    words.extend_from_slice(&options);
    for part in [language, string] {
        words.push(i32::try_from(part.chars().count()).unwrap());
        words.extend(part.chars().map(|character| i32::try_from(u32::from(character)).unwrap()));
    }
    words
}

fn plain(string: &str) -> Vec<i32> {
    text([NONE; 10], "", string)
}

fn opening(context: ContextId) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer: Renderer::Regis,
        level: Level::TwoFive,
        page: 0x1A0,
        subcode: 0,
        flags: ContextFlags::default(),
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

fn host() -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel), 100);
    host.open(opening(ContextId::Page), 200);
    host.begin_step(0, true);
    host
}

fn words(numbers: &[i32]) -> Vec<u8> {
    numbers.iter().flat_map(|number| number.to_le_bytes()).collect()
}

fn call_ints(host: &mut Host, context: i32, op: Op, ints: &[i32], payload: &[i32]) -> Reply {
    let bytes = words(payload);
    host.call(context, op.number(), ints, Payload::Ints(&bytes))
}

/// The budget report's entry for the macrographs' bytes.
fn macrograph_bytes(budget: &str) -> String {
    budget.split_once("\"macrographBytes\"").unwrap().1.split('}').next().unwrap().to_owned()
}

fn handle(reply: Reply) -> i32 {
    match reply {
        Reply::Int(handle) => handle,
        other => panic!("{other:?}"),
    }
}

/// A receiver with the decoder's fonts, fed batches.
struct Receiver {
    terminal: Terminal,
    screen: Screen,
}

impl Receiver {
    fn new() -> Self {
        let mut terminal = Terminal::new();
        fonts::give(&mut terminal);
        Self { terminal, screen: Screen::new() }
    }

    /// Feeds a batch's ReGIS; whether a commit was applied.
    #[track_caller]
    fn take(&mut self, batch: &Batch) -> bool {
        let Some(bytes) = &batch.regis else { return false };
        let events = self.terminal.feed_regis(&mut self.screen, bytes);
        assert!(!events.iter().any(|event| matches!(event, RegisEvent::CommitFailed { .. })), "the receiver refused what the host sent: {events:?}");
        events.iter().any(|event| matches!(event, RegisEvent::Committed(_)))
    }

    fn pixel(&mut self, x: u32, y: u32) -> u8 {
        let mut picture = Picture::new();
        self.terminal.compose(&self.screen, &mut picture);
        picture.canvas().pixel(x, y).unwrap()
    }
}

/// Section 6.3, positions and vectors: `vector` is ReGIS's pixel vectors
/// with their multiplier, which lasts for that call alone; the position
/// is followed through them, so that a place with a negative coordinate
/// after them is reached.
#[test]
fn pixel_vectors_move_by_their_multiplier_in_eight_directions() {
    let vector = |multiplier: i32, digits: &[i32]| {
        let mut words = vec![P::VECTOR.number(), multiplier, i32::try_from(digits.len()).unwrap()];
        words.extend_from_slice(digits);
        words
    };
    assert_eq!(contents(&vector(10, &[0, 2, 4, 6])), "V(W(M10))0246");
    // A square of 30, then an octagon's diagonals, from 50,50.
    let mut program = vec![P::MOVE_TO.number(), 50, 50];
    program.extend(vector(30, &[0, 6, 4, 2]));
    program.extend(vector(7, &[1, 3, 5, 7]));
    draws_as(&program, "P[50,50]V[80,50][80,80][50,80][50,50]V[57,43][50,36][43,43][50,50]");
    // The multiplier was that call's alone: a vector after it is one pixel a digit.
    let mut program = vec![P::MOVE_TO.number(), 50, 50];
    program.extend(vector(30, &[0]));
    program.extend(vector(1, &[6, 6, 6]));
    draws_as(&program, "P[50,50]V[80,50][80,53]");
    // The position is followed: from 5,5 four pixels up-left twice is
    // -3,-3, and a line to -10,-3 from there is seven pixels left.
    let mut program = vec![P::MOVE_TO.number(), 5, 5];
    program.extend(vector(4, &[3, 3]));
    program.extend_from_slice(&[P::LINE_TO.number(), -10, -3]);
    assert_eq!(contents(&program), "P[5,5]V(W(M4))33V[-7,+0]");
    draws_as(&program, "P[5,5]V[-8,-8]V[-7]");
    assert!(matches!(refused(&vector(1, &[8])), Reply::Type(_)));
    assert!(matches!(refused(&vector(1, &[])), Reply::Type(_)));
    assert!(matches!(refused(&[P::VECTOR.number(), 1, 5, 0]), Reply::Type(_)), "fewer digits than said");
    assert_range(&refused(&vector(0, &[1])), "MAX_COORDINATE");
    assert_range(&refused(&vector(1, &vec![0; MAX_POINTS + 1])), "MAX_POINTS");
    assert_range(&refused(&vector(32_767, &[0, 0])), "MAX_COORDINATE");
}

/// Section 6.3, curves: `circle` and `circleThrough` about the position,
/// which stays; `arc` from the position about a centre, which leaves the
/// position at the arc's end as ReGIS does, so that arcs link; `curve`
/// through points, open or closed.
#[test]
fn circles_arcs_and_curves_are_regiss_own() {
    assert_eq!(contents(&[P::CIRCLE.number(), 40]), "C[+40]");
    draws_as(&[P::MOVE_TO.number(), 100, 80, P::CIRCLE.number(), 40, P::LINE_BY.number(), 5, 0], "P[100,80]C[140,80]V[105,80]");
    draws_as(&[P::MOVE_TO.number(), 100, 80, P::CIRCLE_THROUGH.number(), 100, 50, P::CIRCLE_THROUGH.number(), -20, 80], "P[100,80]C[100,50]C[-120]");
    // An arc of a quarter turn anticlockwise about 100,80 from 140,80,
    // and a second linked to its end about the same centre.
    let arcs = [P::MOVE_TO.number(), 140, 80, P::ARC.number(), 100, 80, 90, P::ARC.number(), 100, 80, 45];
    assert_eq!(contents(&arcs), "P[140,80]C(C,A+90)[100,80]C(C,A+45)[100,80]");
    draws_as(&arcs, "P[140,80]C(A90,C)[100,80]C(A45,C)[100,80]");
    draws_as(&[P::MOVE_TO.number(), 140, 80, P::ARC.number(), 100, 80, -90], "P[140,80]C(A-90,C)[100,80]");
    // After an arc the position is the renderer's to know: a line to a
    // negative coordinate needs a moveTo first, and a moveTo to one is
    // written from the origin.
    let after_arc = |then: &[i32]| [&[P::MOVE_TO.number(), 40, 0, P::ARC.number(), 0, 0, 90][..], then].concat();
    assert!(matches!(refused(&after_arc(&[P::LINE_TO.number(), -5, 5])), Reply::Type(_)));
    assert_eq!(contents(&after_arc(&[P::MOVE_TO.number(), -5, 5, P::LINE_TO.number(), -10, 5])), "P[40,0]C(C,A+90)[0,0]P[0,0]P[-5,+5]V[-5,+0]");
    draws_as(&after_arc(&[P::MOVE_TO.number(), -5, 5, P::LINE_TO.number(), -10, 5]), "P[40,0]C(A90,C)[0,0]P[0,0]P[-5,+5]V[-5]");
    // A whole circle about a centre leaves the position where it was.
    assert_eq!(contents(&[P::MOVE_TO.number(), 40, 0, P::ARC.number(), 0, 0, 360, P::LINE_TO.number(), -5, 0]), "P[40,0]C(C,A+360)[0,0]V[-45,+0]");
    assert!(matches!(refused(&[P::ARC.number(), 0, 0, 361]), Reply::Type(_)));
    assert!(matches!(refused(&[P::ARC.number(), 0, 0, 0]), Reply::Type(_)));
    assert_range(&refused(&[P::CIRCLE.number(), -1]), "MAX_COORDINATE");
    // A curve: open ends at its last point, closed where it began.
    let curve = |closed: i32, then: &[i32]| [&[P::CURVE.number(), closed, 4, 20, 20, 60, -10, 100, 20, 60, 50][..], then].concat();
    assert_eq!(contents(&curve(0, &[P::LINE_TO.number(), -1, 50])), "P[20,20]C(S)[+40,-30][100,20][60,50](E)V[-61,+0]");
    assert_eq!(contents(&curve(1, &[P::LINE_TO.number(), -1, 20])), "P[20,20]C(B)[+40,-30][100,20][60,50](E)V[-21,+0]");
    draws_as(&curve(0, &[]), "P[20,20]C(S)[+40,-30][+40,+30][-40,+30](E)");
    draws_as(&curve(1, &[P::LINE_BY.number(), 0, 9]), "P[20,20]C(B)[+40,-30][+40,+30][-40,+30](E)V[,+9]");
    assert!(matches!(refused(&[P::CURVE.number(), 2, 1, 0, 0]), Reply::Type(_)));
    assert_range(&refused(&[P::CURVE.number(), 0, i32::try_from(MAX_POINTS).unwrap() + 1]), "MAX_POINTS");
    // In a fill, a circle is its outline.
    draws_as(&[P::MOVE_TO.number(), 100, 80, P::FILL.number(), 2, P::CIRCLE.number(), 30], "P[100,80]F(C[+30])");
}

/// Section 6.3, the write controls: ReGIS's four writing modes applied to
/// the pixels beneath, negative writing, the standard patterns and a
/// pattern of eight bits with their multiplier, the plane mask, and
/// shading on, to a line at a y or an x, and with a character.
#[test]
fn the_write_controls_are_regiss_own() {
    let block = |ink: i32| [P::INK.number(), ink, P::FILL.number(), 5, P::RECT.number(), 0, 0, 60, 40];
    let over = |control: &[i32]| {
        [
            &block(5)[..],
            control,
            &[P::INK.number(), 3, P::FILL.number(), 5, P::RECT.number(), 30, 20, 60, 40, P::MOVE_TO.number(), 0, 70, P::LINE_TO.number(), 120, 70],
        ]
        .concat()
    };
    for (mode, letter) in [(0, 'V'), (1, 'R'), (2, 'C'), (3, 'E')] {
        assert_eq!(contents(&[P::MODE.number(), mode]), format!("W({letter})"));
        draws_as(
            &over(&[P::MODE.number(), mode]),
            &format!("W(I5)P[0,0]F(V[59,0][59,39][0,39][0,0])W({letter})W(I3)P[30,20]F(V[89,20][89,59][30,59][30,20])P[0,70]V[120,70]"),
        );
    }
    assert!(matches!(refused(&[P::MODE.number(), 4]), Reply::Type(_)));
    // Complement turns what is beneath: where the two blocks overlap the
    // pixel is neither's ink.
    let complemented = painted("", contents(&over(&[P::MODE.number(), 2])).as_bytes());
    let at = |pixels: &[u8], x: usize, y: usize| pixels[(150 + y) * 800 + 200 + x];
    assert_eq!(at(&complemented, 10, 10), 5, "the first block");
    assert!(![3, 5].contains(&at(&complemented, 40, 30)), "the overlap is complemented");
    // Negative writing and patterns: a dashed line and outline. The
    // multiplier is always written: ReGIS's own default is 2, and the
    // API's is 1.
    for (program, reference) in [
        (vec![P::PATTERN.number(), 0, 4, 1], "W(P(M1))W(P4)"),
        (vec![P::PATTERN.number(), 0, 2, 3], "W(P2(M3))"),
        (vec![P::PATTERN.number(), 1, 0b1100_1010, 2], "W(P11001010(M2))"),
        (vec![P::PATTERN.number(), 1, 0b0000_1111, 1], "W(P00001111(M1))"),
        (vec![P::NEGATIVE.number(), 1, P::PATTERN.number(), 0, 4, 1], "W(N1,P4(M1))"),
        (vec![P::NEGATIVE.number(), 1, P::NEGATIVE.number(), 0, P::PATTERN.number(), 0, 9, 16], "W(N0,P9(M16))"),
    ] {
        let line = [P::MODE.number(), 1, P::MOVE_TO.number(), 0, 10, P::LINE_TO.number(), 200, 10, P::RECT.number(), 0, 20, 100, 30];
        draws_as(&[&program[..], &line].concat(), &format!("{reference}W(R)P[0,10]V[200,10]P[0,20]V[99,20][99,49][0,49][0,20]"));
    }
    assert_eq!(contents(&[P::PATTERN.number(), 1, 0b0000_1111, 1]), "W(P00001111(M1))");
    assert_range(&refused(&[P::PATTERN.number(), 0, 1, 17]), "MAX_PATTERN_MULTIPLIER");
    assert_range(&refused(&[P::PATTERN.number(), 0, 1, 0]), "MAX_PATTERN_MULTIPLIER");
    assert!(matches!(refused(&[P::PATTERN.number(), 0, 10, 1]), Reply::Type(_)));
    assert!(matches!(refused(&[P::PATTERN.number(), 1, 256, 1]), Reply::Type(_)));
    assert!(matches!(refused(&[P::NEGATIVE.number(), 2]), Reply::Type(_)));
    // The plane mask: ink 15 written to planes 0 and 2 alone is 5.
    draws_as(&[P::PLANES.number(), 5, P::INK.number(), 15, P::RECT.number(), 0, 0, 50, 50], "W(F5,I15)P[0,0]V[49,0][49,49][0,49][0,0]");
    assert_eq!(at(&painted("", contents(&[P::PLANES.number(), 5, P::INK.number(), 15, P::DOT.number()]).as_bytes()), 0, 0), 5);
    assert_range(&refused(&[P::PLANES.number(), 16]), "MAX_PLANES");
    // Shading: on, to the position's own line; to a line at a y; to a
    // line at an x; with a character's shape; and off again.
    let wave = [P::POLYLINE.number(), 4, 0, 40, 30, 10, 60, 40, 90, 20];
    let quote = i32::from(b'\'');
    for (shade, reference) in [
        (vec![P::SHADE.number(), 1, 0], "P[0,40]W(S1)"),
        (vec![P::SHADE.number(), 2, 60], "P[0,40]W(S[,60])"),
        (vec![P::SHADE.number(), 2, -20], "P[0,40]W(S[,-60])"),
        (vec![P::SHADE.number(), 3, 120], "P[0,40]W(S(X)[120])"),
        (vec![P::SHADE.number(), 4, i32::from(b'X')], "P[0,40]W(S\"X\")"),
        (vec![P::SHADE.number(), 4, quote], "P[0,40]W(S\"'\")"),
        (vec![P::SHADE.number(), 1, 0, P::SHADE.number(), 0, 0], "P[0,40]W(S0)"),
    ] {
        draws_as(&[&[P::MOVE_TO.number(), 0, 40][..], &shade, &wave].concat(), &format!("{reference}P[0,40]V[30,10][60,40][90,20]"));
    }
    assert_eq!(contents(&[P::MOVE_TO.number(), 0, 40, P::SHADE.number(), 2, -20]), "P[0,40]W(S[,-60])");
    assert_eq!(contents(&[P::SHADE.number(), 4, quote]), "W(S'''')");
    assert!(matches!(refused(&[P::SHADE.number(), 4, 0x20]), Reply::Type(_)));
    assert!(matches!(refused(&[P::SHADE.number(), 5, 0]), Reply::Type(_)));
    assert_range(&refused(&[P::SHADE.number(), 2, 40_000]), "MAX_COORDINATE");
    // Inside a fill only the outline is traced: no write control, no text.
    for control in [
        vec![P::MODE.number(), 1],
        vec![P::NEGATIVE.number(), 1],
        vec![P::PATTERN.number(), 0, 1, 1],
        vec![P::PLANES.number(), 1],
        vec![P::SHADE.number(), 1, 0],
        plain("x"),
    ] {
        let length = i32::try_from(control.len()).unwrap();
        assert!(matches!(refused(&[&[P::FILL.number(), length][..], &control].concat()), Reply::Type(_)), "{control:?}");
    }
}

/// Section 6.3, text: the string is written in the engine's provided
/// character sets by the engine's own picture text encoder, in the
/// language's own set; the options are ReGIS's text controls, each held
/// to its range; and the position is followed through the string.
#[test]
fn text_is_the_provided_sets_and_regiss_text_controls() {
    // English is set 1 of the 55, alphabet 101; alphabet 0 is selected
    // again after the string (renderer parity PAR-4).
    assert_eq!(contents(&plain("Hi")), "T(A101)\"Hi\"T(A0)");
    assert_eq!(contents(&plain("say \"hi\"")), "T(A101)\"say \"\"hi\"\"\"T(A0)");
    draws_as(&[&[P::MOVE_TO.number(), 10, 10][..], &plain("HARBOUR patrol 123")].concat(), "P[10,10]T(A101)'HARBOUR patrol 123'");
    // Every option, in the one text command before the string.
    let all = text([3, 30, 50, 24, 45, 6, 90, -22, 0, -31], "", "AB");
    assert_eq!(contents(&all), "T(S3,S[30,50],U[24,45],H6,D90,I-22)[+0,-31]T(A101)\"AB\"T(A0)");
    draws_as(&[&[P::MOVE_TO.number(), 100, 200][..], &all].concat(), "P[100,200]T(S3)T(S[30,50])T(U[24,45])T(H6)T(D90)T(I-22)T[+0,-31]T(A101)'AB'");
    for (options, reference) in [
        ([0, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE], "T(S0)"),
        ([16, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE], "T(S16)"),
        ([NONE, 20, 30, NONE, NONE, NONE, NONE, NONE, NONE, NONE], "T(S[20,30])"),
        ([NONE, NONE, NONE, 16, 30, NONE, NONE, NONE, NONE, NONE], "T(U[16,30])"),
        ([NONE, NONE, NONE, NONE, NONE, 4, NONE, NONE, NONE, NONE], "T(H4)"),
        ([NONE, NONE, NONE, NONE, NONE, NONE, 45, NONE, NONE, NONE], "T(D45)"),
        ([NONE, NONE, NONE, NONE, NONE, NONE, -90, NONE, NONE, NONE], "T(D-90)"),
        ([NONE, NONE, NONE, NONE, NONE, NONE, NONE, 45, NONE, NONE], "T(I45)"),
        ([NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, 14, 3], "T[+14,+3]"),
    ] {
        draws_as(&[&[P::MOVE_TO.number(), 100, 200][..], &text(options, "", "Wave")].concat(), &format!("P[100,200]{reference}T(A101)'Wave'"));
    }
    for (bad, bound) in [
        ([17, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE], Some("MAX_TEXT_SIZE")),
        ([-1, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE], Some("MAX_TEXT_SIZE")),
        ([NONE, 0, 10, NONE, NONE, NONE, NONE, NONE, NONE, NONE], Some("MAX_CELL_SIDE")),
        ([NONE, NONE, NONE, 2561, 10, NONE, NONE, NONE, NONE, NONE], Some("MAX_CELL_SIDE")),
        ([NONE, NONE, NONE, NONE, NONE, 257, NONE, NONE, NONE, NONE], Some("MAX_TEXT_HEIGHT")),
        ([NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, 40_000, 0], Some("MAX_COORDINATE")),
        ([NONE, 10, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE], None),
        ([NONE, NONE, NONE, NONE, NONE, NONE, 30, NONE, NONE, NONE], None),
        ([NONE, NONE, NONE, NONE, NONE, NONE, 405, NONE, NONE, NONE], None),
        ([NONE, NONE, NONE, NONE, NONE, NONE, NONE, 30, NONE, NONE], None),
    ] {
        let reply = refused(&text(bad, "", "x"));
        match bound {
            Some(bound) => assert_range(&reply, bound),
            None => assert!(matches!(reply, Reply::Type(_)), "{bad:?}: {reply:?}"),
        }
    }
    // The position is followed through the string: five characters of
    // nine pixels from 0,0 end at 45,0, and a line to -5,0 is fifty left.
    let hello = [&plain("Hello")[..], &[P::LINE_TO.number(), -5, 0]].concat();
    assert_eq!(contents(&hello), "T(A101)\"Hello\"T(A0)V[-50,+0]");
    // At size 2, tilted a quarter turn, each character moves 18 up.
    let tilted = [&text([2, NONE, NONE, NONE, NONE, NONE, 90, NONE, NONE, NONE], "", "abc")[..], &[P::LINE_TO.number(), -1, -54]].concat();
    assert_eq!(contents(&tilted), "T(S2,D90)T(A101)\"abc\"T(A0)V[-1,+0]");
    let spaced = text([NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, 20, 5], "", "ab");
    assert_eq!(contents(&[&spaced[..], &[P::LINE_TO.number(), -1, 10]].concat()), "T[+20,+5]T(A101)\"ab\"T(A0)V[-41,+0]");
    // And the renderer agrees where the string ended: the line drawn
    // from there is the line drawn from that place written out.
    draws_as(&[&[P::MOVE_TO.number(), 100, 100][..], &hello].concat(), "P[100,100]T(A101)'Hello'P[145,100]V[-150,-100]");
    draws_as(&[&[P::MOVE_TO.number(), 100, 100][..], &tilted].concat(), "P[100,100]T(S2,D90)T(A101)'abc'P[100,46]V[-101,-100]");
    // Other languages: each in its own set of the 55. German is set 2,
    // Greek set 17, Russian one of the Cyrillic sets.
    assert!(written(&plain("Gr\u{fc}\u{df}e"), "de").unwrap().starts_with("T(A102)\""));
    let russian = written(&plain("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}"), "ru").unwrap();
    assert!(["T(A114)\"", "T(A115)\"", "T(A116)\""].iter().any(|set| russian.starts_with(set)), "{russian}");
    assert!(written(&plain("\u{3b1}\u{3b2}\u{3b3}"), "el").unwrap().starts_with("T(A117)\""));
    // A language named by the call, not the context's.
    assert_eq!(written(&text([NONE; 10], "de", "\u{df}"), "en").unwrap(), written(&plain("\u{df}"), "de").unwrap());
    // Persian: a joined Arabic-script set (144), the string in visual
    // order, so its first letter is written last; the receiver joins it.
    let codes = written(&plain("\u{633}\u{644}\u{627}\u{645}"), "fa").unwrap();
    assert!(codes.starts_with("T(A144)\""), "{codes}");
    let (forwards, backwards) = (written(&plain("\u{633}\u{644}"), "fa").unwrap(), written(&plain("\u{644}\u{633}"), "fa").unwrap());
    let inner = |text: &str| text.trim_start_matches("T(A144)\"").trim_end_matches("\"T(A0)").to_owned();
    assert_eq!(inner(&forwards).chars().rev().collect::<String>(), inner(&backwards), "right-to-left text is written in visual order");
    let drawn = painted("", format!("P[10,10]{codes}").as_bytes());
    assert!(drawn.iter().filter(|pixel| **pixel != 0).count() > 40, "the receiver draws the joined letters");
    // A character no set of the language shows is a TypeError naming it,
    // never another character; so is a language the sets do not write, and
    // a control character.
    match refused(&plain("snow \u{2603}")) {
        Reply::Type(message) => assert!(message.contains("U+2603"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(written(&text([NONE; 10], "xx", "a"), "en"), Err(Reply::Type(_))));
    assert!(matches!(refused(&plain("a\tb")), Reply::Type(_)));
    assert!(matches!(refused(&[&[P::TEXT.number()][..], &[NONE; 10], &[0, 1, 0x0011_0000]].concat()), Reply::Type(_)), "not a Unicode scalar value");
    assert_range(&refused(&plain(&"x".repeat(MAX_TEXT_CHARS + 1))), "MAX_TEXT_CHARS");
    assert_range(&refused(&text([NONE; 10], &"x".repeat(17), "a")), "MAX_LANGUAGE_BYTES");
    assert_eq!(contents(&plain(&"x".repeat(MAX_TEXT_CHARS))).len(), MAX_TEXT_CHARS + "T(A101)\"\"T(A0)".len());
    // A context whose language the engine has no set for is written as
    // Latin English, as a page's is.
    assert_eq!(written(&plain("Hi"), "tlh").unwrap(), "T(A101)\"Hi\"T(A0)");
}

/// Section 6.2, `macrograph`, `redefine` and the painter's `call`: one
/// of the 26 macrographs, its letter the host's, written among the
/// step's operations and outside every drawing; a drawing uses it as it
/// stood at the step's commit; the bytes of all are held to the
/// renderer's cap and counted; each context has its share.
#[test]
fn macrographs_are_the_receivers_and_held_to_their_bounds() {
    let mut host = host();
    // A square from wherever the position is: a macrograph is drawn where
    // it is called. In ReGIS a fill's outline is the places its vectors
    // reach, so the square's fourth side is traced back to its start.
    let square = |side: i32, ink: i32| {
        vec![
            P::INK.number(),
            ink,
            P::FILL.number(),
            12,
            P::LINE_BY.number(),
            side,
            0,
            P::LINE_BY.number(),
            0,
            side,
            P::LINE_BY.number(),
            -side,
            0,
            P::LINE_BY.number(),
            0,
            -side,
        ]
    };
    let first = handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &square(10, 2)));
    let second = handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &[P::MOVE_BY.number(), 20, 0, P::CALL.number(), first]));
    assert_eq!((first, second), (MACROGRAPH_HANDLE_BASE, MACROGRAPH_HANDLE_BASE - 1));
    let drawing = handle(host.call(PAGE, Op::RG_DRAWING.number(), &[0, 0, 1, 100, 100, 0, -1, 1], Payload::None));
    // The drawing calls both: a square at its origin and one twenty right.
    let calls = [P::CALL.number(), first, P::MOVE_TO.number(), 0, 0, P::CALL.number(), second];
    assert_eq!(call_ints(&mut host, PAGE, Op::RG_DEFINE, &[drawing], &calls), Reply::Undefined);
    let batch = host.take_batch(1, &[]);
    assert_eq!(
        String::from_utf8_lossy(batch.regis.as_ref().unwrap()),
        "@:AW(I2)F(V[+10,+0]V[+0,+10]V[-10,+0]V[+0,-10])@;@:BP[+20,+0]@A@;D(S1(X100,Y100))D(B1)@AP[0,0]@BD(E)D(C)"
    );
    let mut receiver = Receiver::new();
    assert!(receiver.take(&batch));
    assert_eq!([receiver.pixel(105, 105), receiver.pixel(125, 105), receiver.pixel(115, 105), receiver.pixel(131, 105)], [2, 2, 0, 0]);
    let share = MAX_MACROGRAPH_BYTES - CHANNEL_SHARE_MACROGRAPH_BYTES;
    assert!(host.budget(ContextId::Page).contains(&format!(
        "\"macrographBytes\":{{\"used\":{},\"limit\":{share}}}",
        "W(I2)F(V[+10,+0]V[+0,+10]V[-10,+0]V[+0,-10])".len() + "P[+20,+0]@A".len()
    )));
    // Redefined, with nothing else staged: the step still commits, and
    // every drawing that calls it shows it as it now is.
    host.begin_step(1, true);
    assert_eq!(handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[first], &square(14, 3))), first);
    let batch = host.take_batch(2, &[]);
    assert_eq!(String::from_utf8_lossy(batch.regis.as_ref().unwrap()), "@:AW(I3)F(V[+14,+0]V[+0,+14]V[-14,+0]V[+0,-14])@;D(C)");
    assert!(receiver.take(&batch), "a step that only defined a macrograph again commits");
    assert_eq!([receiver.pixel(105, 105), receiver.pixel(112, 112), receiver.pixel(125, 105), receiver.pixel(133, 112)], [3, 3, 3, 3]);
    // After a macrograph's call the position is the macrograph's to have
    // left, and a body starts with the position unknown.
    host.begin_step(2, true);
    assert!(matches!(call_ints(&mut host, PAGE, Op::RG_DEFINE, &[drawing], &[P::CALL.number(), first, P::LINE_TO.number(), -5, 0]), Reply::Type(_)));
    assert!(matches!(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &[P::LINE_TO.number(), -5, 0]), Reply::Type(_)));
    // A handle is the context's own macrograph of this page, or a TypeError.
    for forged in [0, 5, -1, MACROGRAPH_HANDLE_BASE - 2, MACROGRAPH_HANDLE_BASE - 26, i32::MIN, drawing] {
        assert!(matches!(call_ints(&mut host, PAGE, Op::RG_DEFINE, &[drawing], &[P::CALL.number(), forged]), Reply::Type(_)), "{forged}");
        if forged != 0 {
            assert!(matches!(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[forged], &[P::DOT.number()]), Reply::Type(_)), "{forged}");
        }
    }
    assert!(matches!(call_ints(&mut host, CHANNEL, Op::RG_MACROGRAPH, &[first], &[P::DOT.number()]), Reply::Type(_)), "the other context's");
    // Text that would end the definition is refused in a macrograph, and
    // taken in a drawing, where nothing reads it so.
    assert!(matches!(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &plain("mail @; home")), Reply::Type(_)));
    assert_eq!(call_ints(&mut host, PAGE, Op::RG_DEFINE, &[drawing], &plain("mail @; home")), Reply::Undefined);
    assert!(receiver.take(&host.take_batch(3, &[])));
    // The bounds: 24 macrographs for the page and 2 for the channel
    // context; the bytes of each context's share.
    host.begin_step(3, true);
    for _ in 2..MACROGRAPHS - CHANNEL_SHARE_MACROGRAPHS {
        handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &[P::DOT.number()]));
    }
    assert_range(&call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &[P::DOT.number()]), "MACROGRAPHS");
    let theirs = handle(call_ints(&mut host, CHANNEL, Op::RG_MACROGRAPH, &[0], &[P::DOT.number()]));
    handle(call_ints(&mut host, CHANNEL, Op::RG_MACROGRAPH, &[0], &[P::DOT.number()]));
    assert_range(&call_ints(&mut host, CHANNEL, Op::RG_MACROGRAPH, &[0], &[P::DOT.number()]), "CHANNEL_SHARE_MACROGRAPHS");
    // About 34 bytes a rectangle: 500 of them pass the page's share, and
    // 100 the channel context's.
    let many = |count: usize| (0..count).flat_map(|_| [P::RECT.number(), 100, 100, 50, 50]).collect::<Vec<i32>>();
    let before = host.budget(ContextId::Page);
    assert_range(&call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[first], &many(500)), "MAX_MACROGRAPH_BYTES");
    assert_range(&call_ints(&mut host, CHANNEL, Op::RG_MACROGRAPH, &[theirs], &many(100)), "MAX_MACROGRAPH_BYTES");
    assert_eq!(host.budget(ContextId::Page), before, "a refused definition changes nothing");
    assert_eq!(handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[first], &many(250))), first);
    assert_eq!(handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[second], &many(150))), second);
    assert_range(&call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[second], &many(200)), "MAX_MACROGRAPH_BYTES");
    let batch = host.take_batch(4, &[]);
    assert!(batch.regis.as_ref().is_some_and(|bytes| bytes.len() <= MAX_BATCH_BYTES));
    assert!(receiver.take(&batch), "the receiver holds every macrograph the host counted");
    // A page left takes its macrographs with it.
    host.close(ContextId::Page);
    host.open(opening(ContextId::Page), 200);
    host.begin_step(5, true);
    assert!(matches!(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[first], &[P::DOT.number()]), Reply::Type(_)));
    assert_eq!(handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &[P::DOT.number()])), MACROGRAPH_HANDLE_BASE - 26);
}

/// Section 6.2, "Commit", with macrographs: the renderer takes a
/// macrograph's definition at once, commit or no commit, so when a step's
/// commit is refused the host gives its validator back the definitions
/// the receiver, which was sent nothing, still has. The next step is then
/// judged as the receiver will judge it.
#[test]
fn a_refused_commit_puts_the_macrographs_back() {
    let mut host = host();
    let small = [P::INK.number(), 2, P::FILL.number(), 5, P::RECT.number(), 0, 0, 10, 10];
    let macrograph = handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &small));
    let first = handle(host.call(PAGE, Op::RG_DRAWING.number(), &[0, 0, 1, 100, 100, 0, -1, 1], Payload::None));
    assert_eq!(call_ints(&mut host, PAGE, Op::RG_DEFINE, &[first], &[P::CALL.number(), macrograph]), Reply::Undefined);
    let mut receiver = Receiver::new();
    assert!(receiver.take(&host.take_batch(1, &[])));
    let budget_before = host.budget(ContextId::Page);
    // The step: the macrograph defined again as 300 lines of 30,000
    // pixels, nine million work units for each of the twelve drawings
    // that call it, which is more than one repaint may cost.
    host.begin_step(1, true);
    let dear: Vec<i32> = (0..300).flat_map(|_| [P::MOVE_TO.number(), 0, 0, P::LINE_TO.number(), 30_000, 0]).collect();
    assert_eq!(handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[macrograph], &dear)), macrograph);
    let made: Vec<i32> = (0..11).map(|_| handle(host.call(PAGE, Op::RG_DRAWING.number(), &[0, first, 1, 0, 0, 0, -1, 1], Payload::None))).collect();
    let extra = handle(call_ints(&mut host, PAGE, Op::RG_MACROGRAPH, &[0], &[P::DOT.number()]));
    let batch = host.take_batch(2, &[]);
    assert_eq!(batch.regis, None, "nothing of the refused step is sent");
    assert!(batch.log.unwrap().iter().any(|line| line.text.contains("work, code 6")));
    let (numbers, _) = host.regis_take_failure(ContextId::Page).unwrap();
    assert_eq!(numbers[0], i32::from(CommitError::Work.code()));
    // The macrograph is the small one again, here and in the validator;
    // the one made in the step is gone.
    assert_eq!(macrograph_bytes(&host.budget(ContextId::Page)), macrograph_bytes(&budget_before), "the macrographs are counted as before the step");
    host.begin_step(2, true);
    assert!(matches!(call_ints(&mut host, PAGE, Op::RG_DEFINE, &[first], &[P::CALL.number(), extra]), Reply::Type(_)));
    assert!(matches!(host.call(PAGE, Op::RG_SET.number(), &[made[0], Property::X.number(), 1], Payload::None), Reply::Type(_)));
    // Twelve drawings calling the small macrograph commit: with the dear
    // definition still in the validator this step would be refused too.
    for at in 0..11 {
        handle(host.call(PAGE, Op::RG_DRAWING.number(), &[0, first, 1, 20 * (at + 1), 0, 0, -1, 1], Payload::None));
    }
    let batch = host.take_batch(3, &[]);
    assert!(receiver.take(&batch), "the receiver, which never had the dear definition, commits what the validator passed");
    assert_eq!([receiver.pixel(105, 105), receiver.pixel(25, 5), receiver.pixel(225, 5)], [2, 2, 2]);
}

/// Section 6.3, `measure`: the pixels a string takes at the cell the
/// options select (size 0 is 9 by 10, 1 is 9 by 20, n is 9n by 15n), by
/// the engine's count of the string's cells, and never less than the
/// renderer then paints.
#[test]
fn measure_is_the_cells_the_text_takes() {
    let mut host = host();
    let measure = |host: &mut Host, options: [i32; 5], language: &str, text: &str| {
        let payload = format!("{language}\0{text}");
        match host.call(PAGE, Op::RG_MEASURE.number(), &options, Payload::Text(payload.as_bytes())) {
            Reply::Int(packed) => Ok((packed % HANDLE_NUMBERS, packed / HANDLE_NUMBERS)),
            other => Err(other),
        }
    };
    let none = [NONE; 5];
    assert_eq!(measure(&mut host, none, "", "HELLO"), Ok((45, 20)));
    assert_eq!(measure(&mut host, [0, NONE, NONE, NONE, NONE], "", "HELLO"), Ok((45, 10)));
    assert_eq!(measure(&mut host, [1, NONE, NONE, NONE, NONE], "", "HELLO"), Ok((45, 20)));
    assert_eq!(measure(&mut host, [3, NONE, NONE, NONE, NONE], "", "HELLO"), Ok((135, 45)));
    assert_eq!(measure(&mut host, [16, NONE, NONE, NONE, NONE], "", "AB"), Ok((288, 240)));
    assert_eq!(measure(&mut host, [NONE, 12, 30, NONE, NONE], "", "abcd"), Ok((48, 30)));
    assert_eq!(measure(&mut host, [2, NONE, NONE, 7, NONE], "", "abcd"), Ok((72, 70)));
    assert_eq!(measure(&mut host, [2, NONE, NONE, NONE, 90], "", "abcd"), Ok((30, 72)), "a quarter turn: the string runs up");
    assert_eq!(measure(&mut host, [2, NONE, NONE, NONE, 180], "", "abcd"), Ok((72, 30)));
    assert_eq!(measure(&mut host, [2, NONE, NONE, NONE, 45], "", "abcd"), Ok((102, 102)));
    assert_eq!(measure(&mut host, none, "", ""), Ok((0, 20)));
    assert_eq!(measure(&mut host, none, "de", "Gr\u{fc}\u{df}e"), Ok((45, 20)));
    // Persian: the engine's count of the cells, at most one a letter.
    let (persian, _) = measure(&mut host, none, "fa", "\u{633}\u{644}\u{627}\u{645}").unwrap();
    assert!(persian % 9 == 0 && (9..=36).contains(&persian), "{persian}");
    assert!(matches!(measure(&mut host, none, "", "\u{2603}"), Err(Reply::Type(_))));
    assert!(matches!(measure(&mut host, none, "xx", "a"), Err(Reply::Type(_))));
    assert_range(&measure(&mut host, [17, NONE, NONE, NONE, NONE], "", "a").unwrap_err(), "MAX_TEXT_SIZE");
    assert_range(&measure(&mut host, [16, NONE, NONE, NONE, NONE], "", &"x".repeat(300)).unwrap_err(), "MAX_COORDINATE");
    assert_range(&measure(&mut host, none, "", &"x".repeat(MAX_TEXT_CHARS + 1)).unwrap_err(), "MAX_TEXT_CHARS");
    assert!(matches!(host.call(PAGE, Op::RG_MEASURE.number(), &none, Payload::Text(b"no NUL")), Reply::Type(_)));
    // Against the renderer: the rectangle a drawing of the text reaches
    // is inside the measure, and fills most of it.
    for (size, string) in [(1, "HELLO"), (2, "Harbour patrol"), (5, "Wg")] {
        let (width, height) = measure(&mut host, [size, NONE, NONE, NONE, NONE], "", string).unwrap();
        let mut terminal = Terminal::new();
        fonts::give(&mut terminal);
        let mut screen = Screen::new();
        let program = text([size, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE], "", string);
        let _ = terminal.feed_regis(&mut screen, format!("D(B1){}D(E)D(S1(X50,Y60))D(C)", contents(&program)).as_bytes());
        let reach = terminal.bounds(1).unwrap();
        let int = |value: u32| i32::try_from(value).unwrap();
        let inside = int(reach.x) >= 50 && int(reach.y) >= 60 && int(reach.x + reach.width) <= 50 + width && int(reach.y + reach.height) <= 60 + height;
        assert!(inside, "{string} at size {size}: {reach:?} in {width} by {height}");
        assert!(int(reach.width) > width * 3 / 4 && int(reach.height) > height / 2, "{string} at size {size}: {reach:?} in {width} by {height}");
    }
}

/// Section 10.1 rule 3 and the bounds of section 10.6 over the whole
/// program: a program is refused whole on its first fault, and what it
/// writes is at most a drawing's contents.
#[test]
fn a_program_is_refused_whole_and_bounded() {
    assert!(matches!(refused(&[P::MOVE_TO.number(), 1, 1, 99]), Reply::Type(_)));
    assert!(matches!(refused(&[P::CALL.number()]), Reply::Type(_)));
    assert!(matches!(refused(&[&[P::TEXT.number()][..], &[NONE; 10], &[0, 5, 65]].concat()), Reply::Type(_)), "a string shorter than its length");
    assert!(matches!(refused(&[&[P::TEXT.number()][..], &[NONE; 10], &[0, -1]].concat()), Reply::Type(_)));
    let long: Vec<i32> = (0..80).flat_map(|_| [vec![P::MOVE_TO.number(), 0, 0], plain(&"W".repeat(MAX_TEXT_CHARS))].concat()).collect();
    assert_range(&refused(&long), "REGIS_MAX_CONTENTS_BYTES");
    let dots = i32::try_from(MAX_FILL_VERTICES + 1).unwrap();
    let fill: Vec<i32> = [P::FILL.number(), dots].into_iter().chain((0..dots).map(|_| P::DOT.number())).collect();
    assert_range(&refused(&fill), "MAX_FILL_VERTICES");
    assert!(contents(&[P::MOVE_TO.number(), 0, 0]).len() <= REGIS_MAX_CONTENTS_BYTES);
}
