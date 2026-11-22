//! `nt.naplps` as a script sees it (script-api.md sections 7.1 to 7.3 and
//! 7.6; work package N1): JavaScript run in the container, its batches'
//! NAPLPS played by the NAPLPS renderer at the pinned commit as the
//! browser's decoder plays it, and the pictures compared with those kept.
#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::panic, reason = "tests")]

mod common;
mod naplps_drawn;

use common::{Opens as _, all_lines, assert_checks, container, key, lines, opening, script, tick, with_events};
use naplps_drawn::{Receiver, expect, lit, pixel};
use neoteletext_script::Container;
use neoteletext_script::host::naplps::BACKDROP_HANDLE;
use neoteletext_script::host::naplps::limits::{NAPLPS_MAX_BYTES_PER_CALLBACK, NAPLPS_MAX_BYTES_PER_SECOND};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES};
use protocol::script::{Batch, ContextId, EventKind, Renderer, Step};

/// A container with a page context on a NAPLPS page running `source`,
/// the keys `keys` claimed.
fn naplps_page(source: &str, keys: &[&str]) -> Container {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.renderer = Renderer::Naplps;
    open.keys = keys.iter().map(|key| (*key).to_owned()).collect();
    container.open_with(open, source.as_bytes()).expect("the page opens");
    container
}

/// Steps the container and gives the receiver what the batch appends.
fn step(container: &mut Container, receiver: &mut Receiver, step: &Step) -> Batch {
    let batch = container.step(step).expect("the step is answered");
    if let Some(append) = &batch.naplps {
        receiver.take(append.new_generation, &append.bytes);
    }
    batch
}

/// The worked page example of section 7.6 as far as this work package
/// builds it: its texture, a definition of section 7.4, is work package
/// N2's, so the sea is hatched with one of the standard's own patterns
/// and every other line is the example's. A staged reveal played by the
/// receiver's WAITs, text on the cell grid, a lamp painted dark and lit
/// by the colour map alone, then made to blink: the pictures at each
/// stage are the ones kept.
#[test]
fn the_worked_page_example_draws_its_kept_pictures() {
    let mut container = naplps_page(include_str!("naplps_page/harbour-hatched.js"), &["l", "b"]);
    let mut receiver = Receiver::new();
    let batch = step(&mut container, &mut receiver, &tick(0, 0));
    assert_eq!(lines(&batch), Vec::<String>::new());
    assert!(batch.naplps.as_ref().is_some_and(|append| append.new_generation), "the page's first painting begins a generation");
    // The sky; the receiver holds the rest behind the first WAIT.
    expect("harbour-0-sky", &receiver.canvas, &receiver.colours());
    assert!(!receiver.player.idle());
    assert_eq!(pixel(&receiver.canvas, 240, 380), 0);
    // Half a second later, the sea; half a second after that, the rest.
    receiver.advance(30);
    expect("harbour-1-sea", &receiver.canvas, &receiver.colours());
    receiver.advance(30);
    expect("harbour-2-all", &receiver.canvas, &receiver.colours());
    assert!(receiver.player.idle(), "everything appended is painted and every WAIT has passed");
    let dark = receiver.canvas.pixels().to_vec();
    let (sky, lamp) = (8_usize, 10_usize);
    assert_eq!((receiver.colours()[sky], receiver.colours()[lamp]), ([0x11, 0x33, 0x66], [0, 0, 0]));
    assert_eq!(pixel(&receiver.canvas, 420, 90), 10, "the lamp is painted in its own entry");
    // No tick of the script is needed meanwhile: it asked for none.
    for at in 1..61 {
        let batch = step(&mut container, &mut receiver, &tick(at, u64::from(at)));
        assert_eq!(batch.naplps, None);
    }
    // "l" lights the lamp: the colour map changes and no dot is repainted.
    let batch = step(&mut container, &mut receiver, &with_events(61, 61, vec![key(EventKind::KeyDown, "l", 0)]));
    assert_eq!(batch.verdicts.as_ref().map(|verdicts| verdicts[0].cancelled), Some(true));
    let append = batch.naplps.expect("the colour's bytes");
    assert!(!append.new_generation && append.bytes.len() <= 11, "{:02X?}", append.bytes);
    assert_eq!(receiver.canvas.pixels(), dark);
    assert_eq!(receiver.colours()[lamp], [0xFF, 0xEE, 0x88]);
    expect("harbour-3-lit", &receiver.canvas, &receiver.colours());
    // "b" makes it blink to the sky's colour, half a second each way,
    // played on the colour map alone with the script not running.
    let batch = step(&mut container, &mut receiver, &with_events(62, 62, vec![key(EventKind::KeyDown, "b", 0)]));
    assert!(batch.naplps.is_some());
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(receiver.colours()[lamp]);
        receiver.advance(30);
    }
    assert_eq!(seen, [[0x11, 0x33, 0x66], [0xFF, 0xEE, 0x88], [0x11, 0x33, 0x66], [0xFF, 0xEE, 0x88]]);
    assert_eq!(receiver.canvas.pixels(), dark);
}

/// The worked page example of section 7.6 exactly as the specification
/// gives it: work package TODO-7299's "done". It needs `nt.naplps.texture`
/// (section 7.4, operation 307 `NP_TEXTURE`), which work package N2
/// (TODO-7301) builds.
#[test]
#[ignore = "needs nt.naplps.texture, operation 307 NP_TEXTURE, which work package N2 (TODO-7301) builds; run with --ignored once it has"]
fn the_worked_page_example_as_the_specification_gives_it() {
    let mut container = naplps_page(include_str!("naplps_page/harbour.js"), &["l", "b"]);
    let mut receiver = Receiver::new();
    let batch = step(&mut container, &mut receiver, &tick(0, 0));
    assert_eq!(lines(&batch), Vec::<String>::new(), "the script ran without an error");
    receiver.advance(60);
    assert!(receiver.player.idle());
    expect("harbour-waves", &receiver.canvas, &receiver.colours());
}

/// Sections 7.1 to 7.3 from a script: the objects are frozen, calls
/// chain, names and colours are read, and every refusal is the error the
/// specification names, a `RangeError` beginning with its bound's name or
/// a `TypeError`; section 4.2: `painted` is heard on the backdrop.
#[test]
#[expect(clippy::too_many_lines, reason = "one script that makes every call of sections 7.1 to 7.3, and what its batches hold")]
fn a_script_paints_through_nt_naplps() {
    let mut container = naplps_page(
        &script(
            r##"
        const n = nt.naplps;
        check("frozen", Object.isFrozen(n) && Object.isFrozen(n.colours) && Object.isFrozen(n.backdrop) && Object.isFrozen(n.NaplpsPainter.prototype));
        equal("renderer", nt.page.renderer, "naplps");
        equal("define", n.colours.define("sky", "#136"), 8);
        equal("the same name", n.colours.define("sky", "#113366"), 8);
        equal("the next", n.colours.define("sea", "#024"), 9);
        throws("a colour's form", TypeError, "a colour is", () => n.colours.define("x", "red"));
        throws("a colour's form", TypeError, "a colour is", () => n.colours.set(8, "#12345"));
        throws("teletext's names", TypeError, "teletext's eight colours", () => n.colours.define("white", "#fff"));
        throws("an entry not defined", TypeError, "the entry is not a defined colour", () => n.colours.set(12, "#fff"));
        throws("an entry of teletext's", RangeError, "NAPLPS_COLOURS", () => n.colours.set("white", "#fff"));
        let kept;
        n.backdrop.paint((p) => {
          kept = p;
          check("a painter", p instanceof n.NaplpsPainter);
          check("calls chain", p.ink("white").moveTo(0, 0).lineBy(5, 5).ink(sky_entry()).fillRect(10, 10, 4, 4) === p);
        });
        function sky_entry() { return 8; }
        throws("a painter outside its function", TypeError, "a painter is used only", () => kept.moveTo(0, 0));
        throws("a name no colour has", TypeError, "no colour is named", () => n.backdrop.paint((p) => p.ink("mauve")));
        throws("an entry past the map", RangeError, "NAPLPS_COLOURS", () => n.backdrop.paint((p) => p.ink(16)));
        throws("a rectangle too wide", RangeError, "CANVAS_WIDTH", () => n.backdrop.paint((p) => p.fillRect(0, 0, 481, 1)));
        throws("a place off the screen", RangeError, "MAX_X", () => n.backdrop.paint((p) => p.moveTo(480, 0)));
        throws("a number that is not whole", TypeError, "a painter's argument", () => n.backdrop.paint((p) => p.moveTo(1.5, 0)));
        throws("a number that is no number", TypeError, "a painter's argument", () => n.backdrop.paint((p) => p.moveTo({ valueOf() { return 1; } }, 0)));
        throws("points that are no pairs", TypeError, "a polyline is", () => n.backdrop.paint((p) => p.polyline([[1, 2, 3]])));
        throws("too many points", RangeError, "MAX_POINTS", () => n.backdrop.paint((p) => p.polyline(new Array(1025).fill([1, 1]))));
        throws("a line texture", TypeError, "no such line texture", () => n.backdrop.paint((p) => p.stroke("wavy")));
        throws("a fill style", TypeError, "no such fill style", () => n.backdrop.paint((p) => p.fillStyle("tartan")));
        throws("a texture that is none", TypeError, "a fill style is", () => n.backdrop.paint((p) => p.fillStyle({})));
        throws("a glyph that is none", TypeError, "a glyph is", () => n.backdrop.paint((p) => p.glyph({})));
        throws("a stamp that is none", TypeError, "a stamp is", () => n.backdrop.paint((p) => p.stamp({}, 0, 0)));
        throws("a character no set holds", TypeError, "text:", () => n.backdrop.paint((p) => p.text(1, 0, "漢")));
        throws("text past the last column", RangeError, "GRID_COLUMNS", () => n.backdrop.paint((p) => p.text(1, 38, "abc")));
        throws("a text size", TypeError, "no such text size", () => n.backdrop.paint((p) => p.text(1, 0, "a", { size: "huge" })));
        throws("a rotation", TypeError, "no such rotation", () => n.backdrop.paint((p) => p.textAt(0, 0, "a", { rotation: 45 })));
        throws("an image's pixels", TypeError, "an image's pixels", () => n.backdrop.paint((p) => p.image(0, 0, 1, 1, [1])));
        throws("a wait too long", RangeError, "WAIT_BUDGET_TICKS", () => n.backdrop.paint((p) => p.wait(601)));
        throws("a painting that is no function", TypeError, "a painting is", () => n.backdrop.paint(7));
        // A refused painting painted nothing: the calls before its fault are not kept.
        const before = nt.budget().appendedBytesThisSecond.used;
        throws("refused whole", RangeError, "NAPLPS_COLOURS", () => n.backdrop.paint((p) => p.fillRect(0, 0, 9, 9).ink(99)));
        equal("nothing appended", nt.budget().appendedBytesThisSecond.used, before);
        // Every call of section 7.3's table that needs no definition.
        n.backdrop.paint((p) => {
          p.moveTo(10, 100).moveBy(1, 1).pointAt(12, 102).pointBy(1, 1).lineTo(30, 110).lineBy(5, -5);
          p.polyline([[40, 100], [50, 110], [60, 100]]).polylineBy([[5, 5], [5, -5]]);
          p.rect(80, 100, 10, 10).fillRect(95, 100, 10, 10).moveTo(110, 100).rectBy(10, 10).fillRectBy(10, 10);
          p.polygon([[140, 100], [150, 100], [145, 110]]).fillPolygon([[160, 100], [170, 100], [165, 110]]);
          p.moveTo(180, 100).polygonBy([[10, 0], [-5, 10]]).moveTo(200, 100).fillPolygonBy([[10, 0], [-5, 10]]);
          p.moveTo(220, 110).arc(230, 100, 240, 110).moveTo(250, 110).fillArc(260, 100, 270, 110).circle(290, 105, 8).disc(310, 105, 8);
          p.moveTo(330, 100).trace(2, 2, "xxydd<x^y").moveTo(350, 100).fillTrace(4, 4, "xxyy<xx");
          p.image(370, 100, 2, 2, new Uint8Array([1, 2, 3, 4]));
          p.ink("red").inkOn("yellow", "blue").pen(2, 2).stroke("dotdash").fillStyle("cross").highlight(true).mask(8, 8).fillRect(380, 100, 20, 20);
          p.pen(1, 1).stroke("solid").fillStyle("solid").highlight(false);
          p.text(10, 2, "Text on the grid").text(11, 2, "Double", { size: "double-size" }).text(13, 2, "αβγ", { language: "el" });
          p.textAt(200, 300, "Free text", { width: 16, height: 24, rotation: 0, path: "right", spacing: 1.25, rowSpacing: 2, underline: true, reverse: true });
          p.textAt(300, 400, "Up", { rotation: 90, path: "up", spacing: "proportional" });
          p.wait(1);
        });
        equal("measure", JSON.stringify(n.measure("abc")), '{"width":36,"height":20}');
        equal("measure a field", n.measure("abc", { width: 24, height: 40, spacing: 1.5 }).width, 108);
        equal("measure a language", n.measure("αβ", { language: "el" }).width, 24);
        throws("a blink to nothing", TypeError, "a colour-map entry", () => n.colours.blink(8, {}));
        n.colours.blink("sky", { to: "black" });
        n.colours.blink(9, { to: 8, on: 3, off: 4, delay: 2 });
        n.colours.steady("sky");
        throws("a blink too slow", RangeError, "MAX_BLINK_TENTHS", () => n.colours.blink(9, { to: 8, on: 64 }));
        throws("too soon", RangeError, "NAPLPS_REPAINT_TICKS", () => n.backdrop.repaint((p) => p.fillRect(0, 0, 1, 1)));
        const budget = nt.budget();
        equal("budget colours", JSON.stringify(budget.colours), '{"used":2,"limit":8}');
        equal("budget blinks", JSON.stringify(budget.blinks), '{"used":1,"limit":32}');
        equal("budget waits", JSON.stringify(budget.waitTicks), '{"used":6,"limit":3600}');
        check("budget bytes", budget.appendedBytesThisSecond.used > 100 && budget.appendedBytesThisSecond.limit === 262144);
        n.backdrop.addEventListener("painted", (event) => { console.log("painted heard on the backdrop: " + (event.target === n.backdrop)); });
        setTimeout(() => {
          n.backdrop.repaint((p) => p.ink("sea").fillRect(0, 0, 480, 460));
          console.log("repainted");
        }, 500);
        done();
    "##,
        ),
        &[],
    );
    let mut receiver = Receiver::new();
    let first = step(&mut container, &mut receiver, &tick(0, 0));
    assert_checks(&lines(&first), 45);
    assert_eq!(first.watch.as_ref().and_then(|watch| watch.painted.clone()), Some(vec![BACKDROP_HANDLE]), "the decoder is asked to watch the backdrop");
    // What it painted is NAPLPS the renderer reads whole: nothing is
    // passed over or refused, and the picture has every shape's dots.
    receiver.advance(6);
    let diagnostics = receiver.terminal.diagnostics();
    assert_eq!((diagnostics.passed_over, diagnostics.refusals), (0, [0; naplps::REFUSAL_KINDS]), "{diagnostics:?}");
    assert!(lit(&receiver.canvas) > 2_000);
    for (x, y) in [(12, 102), (100, 105), (125, 105), (165, 103), (310, 105), (371, 101)] {
        assert_ne!(pixel(&receiver.canvas, x, y), 0, "dot ({x}, {y})");
    }
    assert_eq!(
        (pixel(&receiver.canvas, 370, 100), pixel(&receiver.canvas, 371, 100), pixel(&receiver.canvas, 370, 101), pixel(&receiver.canvas, 371, 101)),
        (1, 2, 3, 4)
    );
    // The decoder reports the backdrop painted: the listener hears it.
    let painted = step(&mut container, &mut receiver, &Step { painted: vec![BACKDROP_HANDLE], ..tick(1, 1) });
    assert_eq!(lines(&painted), ["painted heard on the backdrop: true"]);
    // Half a second on, the timer repaints: a new generation, the two
    // colours and the blink sent again before the new backdrop.
    let mut batches = Vec::new();
    for at in 2..=31 {
        batches.push(Ok(step(&mut container, &mut receiver, &tick(at, u64::from(at)))));
    }
    assert_eq!(all_lines(&batches), ["repainted"]);
    let repaint = batches.iter().flatten().find_map(|batch| batch.naplps.clone()).expect("the repaint's bytes");
    assert!(repaint.new_generation);
    receiver.advance(1);
    assert_eq!(lit(&receiver.canvas), 480 * 460);
    assert_eq!(pixel(&receiver.canvas, 240, 230), 9);
    assert_eq!(receiver.terminal.palette().blinks().len(), 1);
    assert_eq!((receiver.terminal.palette().colour(8), receiver.terminal.palette().colour(9)), ([0x11, 0x33, 0x66], [0x00, 0x22, 0x44]));
}

/// Section 7 and section 3.1: `nt.naplps` is the page context's, on a
/// page drawn in NAPLPS; elsewhere it is not there.
#[test]
fn nt_naplps_is_only_on_a_naplps_page() {
    let source = script(r#"equal("naplps", typeof nt.naplps, "undefined"); done();"#);
    let mut teletext = container();
    teletext.open_with(opening(ContextId::Page), source.as_bytes()).unwrap();
    assert_checks(&lines(&teletext.step(&tick(0, 0)).unwrap()), 1);
    let mut channel = container();
    let mut open = opening(ContextId::Channel);
    open.renderer = Renderer::Naplps;
    channel.open_with(open, source.as_bytes()).unwrap();
    assert_checks(&lines(&channel.step(&tick(0, 0)).unwrap()), 1);
}

/// Section 10.1 rule 7 and the security review's finding S-06
/// (TODO-7272, TODO-7336 item 1), for NAPLPS: a script that paints as
/// much as it can, every way it can, on every tick, makes the host hold
/// and send no more than the named bounds, and is not stopped for it:
/// each call past a bound throws and the page goes on.
#[test]
fn a_script_that_paints_without_end_stays_inside_the_bounds() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.renderer = Renderer::Naplps;
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("naplps_page/hostile-paint.js")).expect("the page opens");
    let (mut second, mut most, mut floods) = (0, 0, 0);
    for at in 0..8_u32 {
        let batch = container.step(&tick(at, u64::from(at))).expect("the step is answered");
        let lines = lines(&batch);
        assert!(lines.iter().all(|line| line.starts_with("flood ")), "{lines:?}");
        floods += lines.len();
        let bytes = batch.naplps.as_ref().map_or(0, |append| append.bytes.len());
        assert!(bytes <= MAX_BATCH_BYTES, "tick {at}: {bytes} bytes in one batch");
        second += bytes;
        most = most.max(bytes);
        assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES, "tick {at}: the host holds {}", container.host_bytes());
        assert_eq!(container.stop_reason(), None, "tick {at}");
    }
    // The floods ran, at the top level and on the ticks; the bytes of
    // eight ticks are within one second's, and the script did reach the
    // bound on one callback's.
    assert!(floods >= 8, "{floods} floods");
    assert!(second <= NAPLPS_MAX_BYTES_PER_SECOND, "{second}");
    assert!(most > NAPLPS_MAX_BYTES_PER_CALLBACK / 2, "{most}");
}
