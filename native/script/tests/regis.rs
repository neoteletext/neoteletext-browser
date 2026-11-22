//! The ReGIS API as a script sees it (script-api.md sections 6.1, 6.2,
//! 4.4 and 10.1; work package R1, rdm TODO-7298): numbered drawings made,
//! moved, given looks and frames and watched, each step one commit that a
//! receiver built on the renderer itself takes and paints; a commit the
//! renderer would refuse caught at the host; and the security review's
//! two scripts that attack this API held to their bounds (TODO-7272).
//!
//! The receiver here is the renderer's own `Terminal`, fed exactly the
//! bytes and animation operations each batch carries, as the decoder
//! feeds them. Its pictures are compared with the kept ones in
//! `tests/golden/regis.txt` and written as PNG files to the test's
//! temporary folder for a person to look at.
mod common;
#[path = "regis_page/fonts.rs"]
mod fonts;
#[path = "regis_page/receiver.rs"]
mod receiver;

use std::time::Duration;

use common::{
    Opens as _, all_lines, assert_checks, claim, container, ints_event, limited, lines, opening, script, tick, timed, view, waited_for_a_processor, with_events,
};
use neoteletext_script::Container;
use neoteletext_script::limits::regis::{CHANNEL_SHARE_DRAWINGS, REGIS_MAX_DRAWINGS};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES, WASM_MEMORY_BYTES};
use neoteletext_script::ops::PointerAction;
use protocol::script::{Bounds, ContextId, ContextOpen, EventKind, Renderer, Step};
use receiver::{GOLDEN_FORMAT, Receiver, keep};
use regis::ViewerState;

const GOLDEN: &str = include_str!("golden/regis.txt");
/// How long the decoder waits for a step's answer before it ends the
/// script (script-api.md section 10.6, `STEP_DEADLINE`; the decoder's to
/// hold, work package F4).
const STEP_DEADLINE: Duration = Duration::from_millis(250);

/// A page context's opening on a ReGIS page, everything claimed.
fn regis_opening(context: ContextId) -> ContextOpen {
    let mut open = opening(context);
    open.renderer = Renderer::Regis;
    claim(&mut open, &["Enter", "ArrowLeft", "ArrowRight"]);
    open
}

/// A container with a page context on a ReGIS page running `source`.
fn regis_page(source: &str) -> Container {
    let mut container = container();
    container.open_with(regis_opening(ContextId::Page), source.as_bytes()).expect("the page opens");
    container
}

/// A pointer event on the drawing `handle` (0 for none) at `x`, `y`:
/// the primary mouse pointer's.
fn pointer(action: PointerAction, x: i32, y: i32, handle: i32, local: (i32, i32), others: &[i32]) -> protocol::script::Event {
    const PRIMARY: i32 = 1024;
    let mut payload = vec![1, i32::from(action == PointerAction::DOWN), local.0, local.1];
    payload.extend_from_slice(others);
    ints_event(EventKind::Pointer, [action.number() | PRIMARY, x, y, handle], &payload)
}

/// The page of section 6.4, without its text (text is work package
/// R2's): a background in layer 0, a panel painted gradually and watched,
/// a button with receiver-run looks, and a four-frame sprite.
const PAGE: &str = r##"
const g = nt.regis;
const sky = g.colours.define("sky", { h: 0, l: 25, s: 60 });
const ink = g.colours.define("ink", "#fff");
const hot = g.colours.define("hot", "#f80");
g.drawing({ layer: 0 }).draw((p) => {
  p.ink(sky).fill((q) => q.rect(0, 0, 800, 480));
  p.ink(ink).polyline([[60, 60], [740, 60]]).moveTo(60, 64).lineTo(740, 64);
});
const caption = g.drawing({ x: 60, y: 420, paintRate: 20, label: "Press the button" });
caption.draw((p) => p.ink(ink).rect(0, 0, 400, 30).moveTo(10, 15).lineBy(380, 0));
caption.addEventListener("painted", () => console.log("painted " + nt.now()));
const face = (c) => g.symbol((p) => { p.ink(c).fill((q) => q.rect(0, 0, 160, 40)); p.ink(0).polygon([[60, 10], [100, 20], [60, 30]]); });
const panel = g.group({ x: 300, y: 280, clip: { x: 0, y: 0, width: 200, height: 80 }, detectable: true });
const button = g.drawing({ parent: panel, x: 20, y: 20, source: face(ink), hover: face(hot), pressed: face(sky), detectable: true, label: "Start" });
const heard = [];
button.addEventListener("pointerdown", (e) => heard.push("button " + e.localX + "," + e.localY + " " + (e.target === button) + " " + e.things.length));
panel.addEventListener("pointerdown", (e) => heard.push("panel " + (e.target === button) + " " + (e.currentTarget === panel)));
nt.addEventListener("pointerdown", (e) => { heard.push("nt " + (e.target === button)); console.log(heard.join("; ")); });
button.addEventListener("pointerup", (e) => { e.preventDefault(); console.log("released on " + e.target.label); });
const frames = [1, 2, 3, 4].map((n) => g.symbol((p) => p.ink(hot).fill((q) => q.rect(0, 0, 10 * n, 10 * n))));
const sprite = g.drawing({ x: 600, y: 300, layer: 2, frames, track: true });
nt.addEventListener("keydown", (e) => {
  if (e.key === "Enter") { sprite.frame = 2; button.look = "pressed"; }
  if (e.key === "ArrowRight") { sprite.moveBy(40, -20); sprite.cycle(1, 3, 6); button.look = null; }
  if (e.key === "ArrowLeft") console.log("bounds " + JSON.stringify(sprite.bounds()) + " " + sprite.overlaps(sprite) + " " + caption.overlaps(sprite));
});
"##;

/// Section 6.4's page, and what "done" means for work package R1:
/// drawings made, moved, given looks and frames, and watched, with kept
/// pictures. Each step is one commit the receiver takes; the painted
/// report raises `painted` (section 6.2); a press goes to the drawing
/// the renderer's pick record names and up its parent chain to `nt`
/// (section 4.4); `frame`, `cycle` and `look` are the receiver's
/// animation state; and a tracked drawing's bounds are the renderer's.
#[test]
fn the_worked_page_is_drawn_watched_pressed_and_animated() {
    let mut container = regis_page(PAGE);
    let mut receiver = Receiver::new();
    let mut wrong = Vec::new();
    let first = container.step(&tick(0, 0)).unwrap();
    assert_eq!(lines(&first), Vec::<String>::new());
    receiver.take(&first);
    assert_eq!(receiver.commits, 1, "one step is one commit");
    let watch = first.watch.clone().unwrap();
    // The caption is drawing 2 and the sprite 12: the host's numbers, in
    // the order the script made its drawings.
    assert_eq!((watch.painted, watch.tracked, watch.wheel), (Some(vec![2]), Some(vec![12]), None));
    assert_eq!(receiver.terminal.label(2).as_deref(), Some("Press the button"));
    assert_eq!(receiver.terminal.take_painted(), Vec::<u16>::new(), "the caption is being painted");
    keep(GOLDEN, "page-at-first", &mut receiver, &mut wrong);
    // The receiver paints the caption at its rate and reports it whole;
    // the step that carries the report raises `painted`.
    let mut painted_at = None;
    for at in 1..200_u32 {
        let _ = receiver.terminal.advance_ticks(1);
        let painted: Vec<i32> = receiver.terminal.take_painted().into_iter().map(i32::from).collect();
        let batch = container.step(&Step { painted: painted.clone(), ..tick(at, u64::from(at)) }).unwrap();
        receiver.take(&batch);
        if !painted.is_empty() {
            assert_eq!(painted, vec![2]);
            assert_eq!(lines(&batch), vec![format!("painted {at}")]);
            painted_at = Some(at);
            break;
        }
        assert_eq!(batch.regis, None, "a step that staged nothing sends nothing");
    }
    // 1,220 work units at 2,000 a second: the caption's 860 pixels of
    // outline and 380 of line take over half a second.
    let painted_at = painted_at.expect("the caption was reported painted");
    assert!((30..60).contains(&painted_at), "painted at tick {painted_at}");
    keep(GOLDEN, "page-painted", &mut receiver, &mut wrong);
    // A press on the button: the renderer's pick record names drawing
    // 7, the button, and the position within it; its parent, the panel,
    // is drawing 3.
    let picture = receiver.picture();
    let pick = picture.pick(330, 310);
    assert_eq!((pick.drawing, pick.x, pick.y), (7, 10, 10));
    assert_eq!(picture.pick(310, 290).drawing, 0, "the panel has no pixels of its own");
    let at = painted_at + 1;
    let press = pointer(PointerAction::DOWN, 330, 310, i32::from(pick.drawing), (pick.x, pick.y), &[3]);
    let batch = container.step(&with_events(at, u64::from(at), vec![press])).unwrap();
    assert_eq!(lines(&batch), vec!["button 10,10 true 2; panel true true; nt true"]);
    assert_eq!(view(&batch).verdicts, vec![2], "heard, and the default not cancelled");
    // The receiver shows the pressed look itself, with no byte sent.
    assert!(receiver.terminal.set_viewer_state(ViewerState { pressed: 7, ..ViewerState::default() }));
    keep(GOLDEN, "page-pressed-by-the-viewer", &mut receiver, &mut wrong);
    assert!(receiver.terminal.set_viewer_state(ViewerState::default()));
    let release = pointer(PointerAction::UP, 330, 310, 7, (10, 10), &[3]);
    let batch = container.step(&with_events(at + 1, u64::from(at + 1), vec![release])).unwrap();
    assert_eq!(lines(&batch), vec!["released on Start"]);
    assert_eq!(view(&batch).verdicts, vec![3], "heard, and the default cancelled");
    // Enter: a frame and a forced look, by the receiver's animation state.
    let key = |name: &str| common::key(EventKind::KeyDown, name, 0);
    let batch = container.step(&with_events(at + 2, u64::from(at + 2), vec![key("Enter")])).unwrap();
    assert_eq!(batch.regis, None, "animation state is not the stream's");
    assert_eq!(batch.animate.as_ref().map(Vec::len), Some(2));
    receiver.take(&batch);
    keep(GOLDEN, "page-frame-2-and-forced-pressed", &mut receiver, &mut wrong);
    // The right arrow: the sprite moved by properties (one commit), a
    // cycle started, the look given back to the pointer.
    let batch = container.step(&with_events(at + 3, u64::from(at + 3), vec![key("ArrowRight")])).unwrap();
    assert_eq!(batch.regis.as_deref(), Some(b"D(S12(X640,Y280))D(C)".as_slice()));
    receiver.take(&batch);
    keep(GOLDEN, "page-sprite-moved", &mut receiver, &mut wrong);
    // The tracked sprite's rectangle is the renderer's, sent with the step.
    let bounds = receiver.bounds(&[12]);
    assert_eq!(bounds, vec![Bounds { handle: 12, x: 640, y: 280, width: 20, height: 20 }], "the sprite's second frame, 20 pixels square");
    let batch = container.step(&Step { bounds, ..with_events(at + 4, u64::from(at + 4), vec![key("ArrowLeft")]) }).unwrap();
    assert_eq!(lines(&batch), vec![r#"bounds {"x":640,"y":280,"width":20,"height":20} true false"#]);
    assert!(receiver.bytes < 1200, "the whole page is {} bytes of ReGIS", receiver.bytes);
    assert!(wrong.is_empty(), "pictures differ from tests/golden/regis.txt (format {GOLDEN_FORMAT}):\n{}", wrong.join("\n"));
}

/// Sections 6.1 and 6.2 as JavaScript: every option is a property that
/// reads back and can be assigned; the functions `symbol`, `group` and
/// `clear`; errors of the kinds section 3.8 gives; a scope's drawings end
/// with it.
#[test]
fn drawings_are_objects_whose_properties_are_regiss_attributes() {
    let body = r##"
const g = nt.regis;
const master = g.symbol((p) => p.rect(0, 0, 4, 4));
equal("a symbol is hidden", master.visible, false);
const top = g.group({ x: 5, y: 6 });
const d = g.drawing({ x: -3, y: 4, depth: 7, visible: false, parent: top, source: master, detectable: true, hover: master, pressed: master, alternative: master,
  clip: { x: 1, y: 2, width: 3, height: 4 }, layer: 2, frames: [master, master], paintRate: 0, label: "Label", wheel: true, track: true });
equal("x", d.x, -3); equal("y", d.y, 4); equal("depth", d.depth, 7); equal("visible", d.visible, false);
check("parent", d.parent === top); check("source", d.source === master); equal("detectable", d.detectable, true);
check("looks", d.hover === master && d.pressed === master && d.alternative === master);
equal("clip", JSON.stringify(d.clip), '{"x":1,"y":2,"width":3,"height":4}'); equal("layer", d.layer, 2);
check("frames", d.frames.length === 2 && d.frames[0] === master && Object.isFrozen(d.frames));
equal("paintRate", d.paintRate, 0); equal("label", d.label, "Label"); equal("wheel", d.wheel, true); equal("track", d.track, true);
const plain = g.drawing();
equal("defaults", JSON.stringify([plain.x, plain.y, plain.depth, plain.visible, plain.parent, plain.source, plain.detectable, plain.hover, plain.pressed,
  plain.alternative, plain.clip, plain.layer, plain.frames, plain.paintRate, plain.label, plain.wheel, plain.track, plain.frame, plain.look]),
  "[0,0,0,true,null,null,false,null,null,null,null,1,null,null,null,false,false,0,null]");
plain.x = 10; plain.y += 5; plain.moveBy(1, 1); plain.moveTo(20, 30); plain.depth = -1; plain.visible = false; plain.parent = top; plain.source = master;
plain.detectable = true; plain.hover = master; plain.clip = { x: 0, y: 0, width: 9, height: 9 }; plain.layer = 3; plain.frames = [master];
plain.paintRate = 100; plain.label = "café"; plain.wheel = true; plain.track = true; plain.frame = 3; plain.look = "alternative";
equal("assigned", JSON.stringify([plain.x, plain.y, plain.depth, plain.visible, plain.parent === top, plain.source === master, plain.detectable,
  plain.hover === master, plain.clip, plain.layer, plain.frames.length, plain.paintRate, plain.label, plain.wheel, plain.track, plain.frame, plain.look]),
  '[20,30,-1,false,true,true,true,true,{"x":0,"y":0,"width":9,"height":9},3,1,100,"café",true,true,3,"alternative"]');
plain.parent = null; plain.source = null; plain.hover = null; plain.clip = null; plain.frames = null; plain.label = null; plain.look = null;
equal("unset", JSON.stringify([plain.parent, plain.source, plain.hover, plain.clip, plain.frames, plain.label, plain.look]), "[null,null,null,null,null,null,null]");
// The kinds of error (section 3.8): a bound is a RangeError beginning
// with its name; a wrong type or a stale handle is a TypeError.
throws("x out of range", RangeError, "MAX_COORDINATE", () => { plain.x = 40000; });
equal("a refused assignment changes nothing", plain.x, 20);
throws("layer", RangeError, "LAYERS", () => { plain.layer = 4; });
throws("paint rate", RangeError, "MAX_PAINT_RATE", () => { plain.paintRate = 32768; });
throws("a rate cannot be unset", TypeError, "a paint rate once given", () => { plain.paintRate = null; });
throws("label", RangeError, "MAX_LABEL_BYTES", () => { plain.label = "x".repeat(65); });
throws("label's characters", TypeError, "a label holds only", () => { plain.label = "ł"; });
throws("frames", RangeError, "MAX_OBJECT_FRAMES", () => { plain.frames = new Array(17).fill(master); });
throws("frame", RangeError, "MAX_OBJECT_FRAMES", () => { plain.frame = 16; });
throws("cycle", RangeError, "MAX_CYCLE_PERIOD", () => plain.cycle(0, 3, 3601));
throws("look", TypeError, "look is", () => { plain.look = "odd"; });
throws("x is whole", TypeError, "x is a whole number", () => { plain.x = 1.5; });
throws("visible is a boolean", TypeError, "visible is true or false", () => { plain.visible = 1; });
throws("parent is a Drawing", TypeError, "parent is a Drawing or null", () => { plain.parent = {}; });
throws("an unknown option", TypeError, "'colour' is not an option", () => g.drawing({ colour: 1 }));
throws("its own parent", TypeError, "a drawing is not its own parent", () => { top.parent = d; });
throws("a Drawing is not constructed", TypeError, "a Drawing is made by", () => new Drawing());
throws("ink", RangeError, "REGIS_COLOURS", () => plain.draw((p) => p.ink(16)));
throws("a painter outside draw", TypeError, "the painter is used only inside draw", () => { let kept; plain.draw((p) => { kept = p; }); kept.dot(); });
throws("a fill in a fill", TypeError, "a fill is not traced", () => plain.draw((p) => p.fill((q) => q.fill(() => {}))));
throws("a colour", TypeError, "a colour is", () => g.colours.define("x", "red"));
throws("a colour entry", RangeError, "REGIS_COLOURS", () => g.colours.set(16, "#000"));
equal("a colour's entry", g.colours.define("one", "#123456"), 1);
// An option the making refuses leaves no drawing behind.
const before = nt.budget().drawings.used;
throws("a refused option", RangeError, "MAX_LABEL_BYTES", () => g.drawing({ label: "x".repeat(65) }));
equal("no drawing is left", nt.budget().drawings.used, before);
// Removing: the drawing and its children; their handles are stale.
const child = g.drawing({ parent: top });
top.remove();
throws("a removed drawing", TypeError, "the drawing was removed", () => { top.x = 1; });
throws("a removed child", TypeError, "the drawing was removed", () => child.x);
throws("a listener on a removed drawing", TypeError, "the drawing was removed", () => child.addEventListener("painted", () => {}));
// A scope's drawings end with it (section 3.3).
let scoped;
const scope = nt.scope(() => { scoped = g.drawing({}); });
equal("in its scope", scoped.x, 0);
scope.end();
throws("ended with its scope", TypeError, "the drawing was removed", () => scoped.x);
// clear removes every drawing of the context.
g.clear();
equal("cleared", nt.budget().drawings.used, 0);
throws("cleared", TypeError, "the drawing was removed", () => master.x);
const budget = nt.budget();
check("the budget's ReGIS entries", ["drawings", "contentBytes", "stagedBytes", "animated", "cycling", "watched", "macrographBytes", "colours", "rebuildWork"].every((name) => typeof budget[name].used === "number" && budget[name].limit > 0));
check("nt.regis is frozen", Object.isFrozen(nt.regis) && Object.isFrozen(nt.regis.colours));
done();
"##;
    let mut container = regis_page(&script(body));
    let batch = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&batch), 53);
    // Made, changed, removed and cleared in one step: one commit, which
    // a receiver takes.
    let mut receiver = Receiver::new();
    receiver.take(&batch);
    assert_eq!(receiver.commits, 1);
    assert!((1..=20).all(|number| receiver.terminal.committed_drawing(number).is_none()), "cleared: no drawing is left at the receiver");
    // A page context has its own page's renderer's API and no other.
    let mut teletext = common::page(&script(r#"equal("a teletext page's context has no ReGIS API", nt.regis, undefined); done();"#));
    assert_checks(&lines(&teletext.step(&tick(0, 0)).unwrap()), 1);
}

/// Section 6.2, "Commit": a commit the renderer would refuse is caught at
/// the host. Nothing of the step is sent; at the next step an `error`
/// event carries a `RangeError` naming the commit error and the drawing;
/// the drawings made in the step are gone, those removed in it are back
/// with their listeners, and every attribute reads as before the step.
#[test]
fn a_refused_commit_discards_the_step_and_raises_an_error() {
    let body = r#"
const g = nt.regis;
const a = g.drawing({ x: 1 });
const b = g.drawing({ parent: a, x: 2 });
const kept = g.drawing({ x: 3, detectable: true });
kept.addEventListener("painted", () => console.log("kept was painted"));
let made = null;
let errors = 0;
addEventListener("error", (e) => {
  errors++;
  e.preventDefault();
  check("a RangeError naming the commit error", e.error instanceof RangeError && e.error.message.startsWith("REGIS_COMMIT_CYCLE: "));
  equal("the commit error", JSON.stringify([e.error.commit.code, e.error.commit.name]), '[4,"cycle"]');
  check("the drawing concerned is one of the cycle", e.error.commit.drawing === a || e.error.commit.drawing === b);
  throws("a drawing made in the step is gone", TypeError, "the drawing was removed", () => made.x);
  equal("a drawing removed in the step is back", kept.x, 3);
  equal("attributes are as before the step", JSON.stringify([a.x, a.source, b.x, b.parent === a]), "[1,null,2,true]");
});
nt.addEventListener("tick", (e) => {
  if (e.tick === 1) {
    made = g.drawing({ x: 9 });
    a.x = 100;
    b.parent = null;
    kept.remove();
    // A drawing drawn in its own place: its child's source is itself.
    b.parent = a;
    b.source = a;
  }
  if (e.tick === 2) { equal("the error was raised before the tick", errors, 1); a.x = 50; }
  if (e.tick === 3) { equal("raised once", errors, 1); done(); }
});
"#;
    let mut container = regis_page(&script(body));
    let mut receiver = Receiver::new();
    let batches = common::run(&mut container, 0, 2);
    let refused = batches[1].as_ref().unwrap();
    assert_eq!(refused.regis, None, "nothing of the refused step is sent");
    assert_eq!(lines(refused), vec!["the step's ReGIS commit would fail (cycle, code 4, drawing 1): its changes were discarded and nothing was sent"]);
    assert_eq!(view(refused).states[0].next_due, Some(2));
    receiver.take(batches[0].as_ref().unwrap());
    receiver.take(refused);
    // The step that tells of it: the listener cancelled the log line; the
    // failure is counted all the same (section 4.2, `error`).
    let told = container.step(&tick(2, 2)).unwrap();
    assert_eq!(view(&told).states[0].failures, 0, "a tick that succeeds follows");
    assert_eq!(told.regis.as_deref(), Some(b"D(S1(X50))D(C)".as_slice()));
    receiver.take(&told);
    assert_eq!(receiver.terminal.drawing_position(1), Some((50, 0)));
    assert!(receiver.terminal.committed_drawing(3).is_some(), "the drawing whose removal was discarded");
    // The drawing that is back is still watched, and its listener hears.
    let painted = container.step(&Step { painted: vec![3], ..tick(3, 3) }).unwrap();
    let log = lines(&painted);
    assert!(log.contains(&"kept was painted".to_owned()), "{log:?}");
    assert_checks(&[all_lines(&batches), lines(&told), log].concat(), 8);
}

/// Section 3.1: the channel context is given the ReGIS API of the page
/// shown, for the overlay layer; its drawings are its own, the page's are
/// the page's, and both leave in one commit.
#[test]
fn the_channel_context_draws_over_the_page_in_layer_3() {
    let channel = r##"
const mark = nt.regis.drawing({ x: 700, y: 10 });
equal("the overlay layer is the channel context's default", mark.layer, 3);
mark.draw((p) => p.ink(2).fill((q) => q.rect(0, 0, 50, 50)));
throws("no other layer", TypeError, "the channel context draws only in the overlay layer", () => nt.regis.drawing({ layer: 1 }));
throws("no colours", TypeError, "only the page context names and sets colours", () => nt.regis.colours.set(1, "#fff"));
done();
"##;
    let page = r#"
const under = nt.regis.drawing({ x: 690, y: 0 });
under.draw((p) => p.ink(3).fill((q) => q.rect(0, 0, 100, 100)));
equal("the page's own layer", under.layer, 1);
done();
"#;
    let mut container = container();
    container.open_with(regis_opening(ContextId::Channel), script(channel).as_bytes()).unwrap();
    container.open_with(regis_opening(ContextId::Page), script(page).as_bytes()).unwrap();
    let batch = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&batch), 4);
    let mut receiver = Receiver::new();
    receiver.take(&batch);
    assert_eq!(receiver.commits, 1);
    let picture = receiver.picture();
    assert_eq!(picture.canvas().pixel(720, 30), Some(2), "the channel's mark lies over the page's drawing");
    assert_eq!(picture.canvas().pixel(695, 80), Some(3));
}

macro_rules! hostile {
    ($name:literal) => {
        include_str!(concat!("hostile/", $name, ".js"))
    };
}

/// What a hostile script's run cost the host.
#[derive(Debug, Default)]
struct Cost {
    /// The most bytes the host held for the container outside its
    /// WebAssembly memory.
    host_bytes: usize,
    /// The most bytes of the WebAssembly memory.
    wasm_bytes: usize,
    /// The largest batch, as the message that leaves.
    batch_bytes: usize,
    /// Every byte of ReGIS sent.
    regis_bytes: usize,
    /// The longest step, on the wall clock.
    longest_step: Duration,
    steps: u32,
    refused_commits: u32,
}

/// Runs a hostile script as a ReGIS page's for `ticks`, feeding a
/// receiver, and gives what it cost: under the limits the browser's
/// script host runs with, or `unhurried`, with no clock to stop it
/// first, so that the bounds on what the host holds are what holds it.
fn run_hostile(source: &str, ticks: u32, unhurried: bool) -> (Cost, Vec<String>) {
    // The start limit is on the wall clock: on a machine with no
    // processor free the prelude alone can wait past it, so an opening
    // stopped for time is tried again, as tests/hostile.rs does.
    let mut opened = None;
    for _ in 0..STARVED_TRIES {
        let mut container = if unhurried { container() } else { limited() };
        if container.open_with(regis_opening(ContextId::Page), source.as_bytes()).is_ok() {
            opened = Some(container);
            break;
        }
    }
    let mut container = opened.expect("the script opens");
    let mut receiver = Receiver::new();
    let mut cost = Cost::default();
    let mut log = Vec::new();
    for at in 0..ticks {
        let (stepped, took) = timed(|| container.step(&tick(at, u64::from(at))));
        let Ok(batch) = stepped else { break };
        cost.longest_step = cost.longest_step.max(took.wall);
        cost.steps += 1;
        let mut written = Vec::new();
        protocol::script::write_to_decoder(&mut written, &protocol::script::FromScriptHost::Batch(Box::new(batch.clone()))).expect("a batch is one frame");
        cost.batch_bytes = cost.batch_bytes.max(written.len());
        cost.regis_bytes += batch.regis.as_ref().map_or(0, Vec::len);
        cost.host_bytes = cost.host_bytes.max(container.host_bytes());
        cost.wasm_bytes = cost.wasm_bytes.max(container.memory().0);
        let lines = lines(&batch);
        cost.refused_commits += u32::try_from(lines.iter().filter(|line| line.contains("ReGIS commit would fail")).count()).unwrap();
        log.extend(lines);
        receiver.take(&batch);
        // Section 10.1 rule 7, at every step: the host's bytes, and one
        // step's output as one frame (its five bytes of framing beside).
        assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes at step {at}", container.host_bytes());
        assert!(written.len() <= MAX_BATCH_BYTES + FRAME_HEADER_BYTES, "a batch of {} bytes at step {at}", written.len());
    }
    assert!(receiver.terminal.held_bytes() <= regis::limits::MAX_HELD_BYTES);
    assert!(cost.wasm_bytes <= WASM_MEMORY_BYTES);
    (cost, log)
}

/// How often a script that should open is tried again when this machine
/// had no processor for it.
const STARVED_TRIES: usize = 5;
/// A frame's kind byte and length (`native/protocol/src/framing.rs`).
const FRAME_HEADER_BYTES: usize = 5;
/// The review measured the spike's host at 130 MB and one frame at 160 MB
/// under `regismem.js`; everything a run sends, in all its steps
/// together, is held under this.
const MOST_SENT_BY_A_RUN: usize = 4 * 1024 * 1024;

/// The security review's `regismem.js` and `regisstage.js` (finding
/// S-06, TODO-7272; section 10.1 rule 7). `regismem.js`: 2,048 drawings
/// of 1,000 rectangles each made the spike's host hold 130 MB and emit
/// one frame of 160 MB. `regisstage.js`: one drawing redefined by 31
/// timers every tick, each definition staged again. Under the browser's
/// limits and with no clock at all, the host holds no more than
/// `MAX_HOST_STATE_BYTES` at any step and no batch is more than
/// `MAX_BATCH_BYTES`. With no clock to stop them first, what stops them
/// is the host's own bounds: a definition that does not fit the step's
/// output is a `RangeError` naming `MAX_BATCH_BYTES`, and the failures in
/// a row stop the context, whose staged step is then never committed.
/// Under the browser's limits they are stopped for good by time: a step
/// whose drawings would cost more than one repaint may is refused whole
/// by the renderer's validator and never sent, and the validator's time
/// is the script's.
#[test]
fn the_reviews_regis_scripts_are_held_to_the_hosts_bounds() {
    for (name, source) in [("regismem.js", hostile!("regismem")), ("regisstage.js", hostile!("regisstage"))] {
        let (cost_limited, log) = run_hostile(source, 120, false);
        println!("{name} under the browser's limits: {cost_limited:?}");
        assert!(cost_limited.regis_bytes <= MOST_SENT_BY_A_RUN, "{name}: {cost_limited:?}");
        let (cost, log_unhurried) = run_hostile(source, 120, true);
        println!("{name} with no clock: {cost:?}");
        assert!(cost.regis_bytes <= MOST_SENT_BY_A_RUN, "{name}: {cost:?}");
        assert!(log_unhurried.iter().any(|line| line.starts_with("uncaught: RangeError: MAX_BATCH_BYTES: ")), "{name}: {log_unhurried:?}");
        assert!(log_unhurried.iter().any(|line| line.starts_with("MAX_CONSECUTIVE_FAILURES: ")), "{name}: {log_unhurried:?}");
        // Under the limits it is stopped for good: by a callback's time,
        // or by the second's budget once the validator's time to refuse
        // its commit has been charged to it (section 3.2: host time is
        // the script's time).
        assert!(cost_limited.steps < 120, "{name} was not stopped: {log:?}");
    }
}

/// Work package R1's own script beside the review's (`regisfill.js`): it
/// catches what is thrown and hides its drawings, so that neither its
/// failures nor the renderer's cap on painting work stop it, and goes on
/// until the page's tables are full. Every bound refuses it by name, the
/// host's bytes stay within `MAX_HOST_STATE_BYTES` with the tables full
/// (the figure is printed), and the receiver takes every commit.
#[test]
fn a_script_that_fills_every_regis_table_is_held_to_the_hosts_bound() {
    let (cost, log) = run_hostile(hostile!("regisfill"), 60, true);
    println!("regisfill.js with no clock: {cost:?}");
    assert_eq!(log.len(), 1, "{log:?}");
    let said = log[0].strip_prefix(&format!("made {} defined ", REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS)).expect("the page's share of drawings was made");
    let (defined, refused) = said.split_once(" refused by ").unwrap();
    // Each drawing's contents are about 54,000 bytes: the page's share holds 18.
    assert_eq!(defined.parse::<usize>().unwrap(), 18);
    assert_eq!(refused, "MAX_BATCH_BYTES,REGIS_MAX_DRAWINGS,REGIS_MAX_TOTAL_CONTENTS_BYTES");
    assert_eq!(cost.refused_commits, 0);
    // The contents alone are held twice by the validator: the tables are full.
    assert!(cost.host_bytes > 2 * 900_000 && cost.host_bytes <= MAX_HOST_STATE_BYTES, "{cost:?}");
}

/// What one way of moving the page's drawings each tick costs. The times
/// are those the test's thread was running, the wall clock less its waits
/// for a processor (`common::timed`), as the library's own budget tests
/// take them and as the script host counts its limits on its own
/// processor time (DEC-7450).
struct Moved {
    /// A step's time on average and at most.
    average: Duration,
    longest: Duration,
    /// A step's time on the wall clock on average, the waits included.
    wall: Duration,
    /// The bytes of a step's commit.
    bytes: usize,
    /// The receiver's time to take and compose a step, on average.
    receiver: Duration,
}

/// Runs a page of 2,048 drawings (the page's 1,984, one of them a hidden
/// master, and the channel context's 64) for a second of ticks, with
/// `each_tick` as what the page's script does each tick once its
/// drawings are made: `all` holds the drawings, `xs` their x as the
/// script last set it, and `plain` as many plain objects with an `x`.
fn moved(each_tick: &str) -> Moved {
    /// The ticks measured: a second.
    const TICKS: u32 = 60;
    let page = format!(
        r#"
const g = nt.regis;
const shape = g.symbol((p) => p.ink(3).fill((q) => q.rect(0, 0, 6, 6)));
const all = [], xs = [], plain = [];
let building = true;
nt.addEventListener("tick", (e) => {{
  // 128 drawings a tick, until the page's share is made.
  if (building) {{
    try {{
      for (let n = 0; n < 128; n++) {{
        const x = (all.length * 13) % 790;
        all.push(g.drawing({{ source: shape, layer: 2, x, y: (all.length * 7) % 470 }}));
        xs.push(x);
        plain.push({{ x }});
      }}
    }} catch (error) {{ building = false; console.log("made " + all.length + ": " + error.message.split(":")[0]); }}
    return;
  }}
  {each_tick}
}});
"#
    );
    let channel = "for (let n = 0; n < 64; n++) nt.regis.drawing({ x: n * 12, y: 470 }).draw((p) => p.ink(2).rect(0, 0, 8, 8));";
    let mut container = container();
    container.open_with(regis_opening(ContextId::Channel), channel.as_bytes()).unwrap();
    container.open_with(regis_opening(ContextId::Page), page.as_bytes()).unwrap();
    let mut receiver = Receiver::new();
    let mut at = 0_u32;
    let mut log = Vec::new();
    while !log.iter().any(|line: &String| line.starts_with("made ")) {
        let batch = container.step(&tick(at, u64::from(at))).unwrap();
        log.extend(lines(&batch));
        receiver.take(&batch);
        at += 1;
        assert!(at < 40, "{log:?}");
    }
    // The page's share, less the one hidden master.
    assert_eq!(log, vec![format!("made {}: REGIS_MAX_DRAWINGS", REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS - 1)]);
    let (mut total, mut longest, mut wall, mut bytes, mut compose) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, 0_usize, Duration::ZERO);
    for _ in 0..TICKS {
        let (batch, took) = timed(|| container.step(&tick(at, u64::from(at))).unwrap());
        total += took.running;
        wall += took.wall;
        longest = longest.max(took.running);
        bytes = bytes.max(batch.regis.as_ref().map_or(0, Vec::len));
        assert_eq!(lines(&batch), Vec::<String>::new());
        let ((), took) = timed(|| {
            receiver.take(&batch);
            let _ = receiver.picture();
        });
        compose += took.running;
        at += 1;
    }
    Moved { average: total / TICKS, longest, wall: wall / TICKS, bytes, receiver: compose / TICKS }
}

/// Measures `each_tick` (see [`moved`]) and holds its average step to
/// `limit`. A measurement over the limit that was made while this thread
/// waited for a processor is made again, as `common::unless_starved`
/// tries a test again: the time a thread is counted as running still
/// grows a little on a machine with no processor free (two threads on one
/// core, a cache shared with strangers), and that is the machine's and
/// not the step's. A measurement over the limit with no such wait fails
/// at once, and one over it at every try fails: what a quiet machine is
/// held to is unchanged.
#[track_caller]
fn moved_within(name: &str, each_tick: &str, limit: Duration) -> Moved {
    for attempt in 1..=STARVED_TRIES {
        let waited_before = waited_for_a_processor();
        let cost = moved(each_tick);
        let waited = waited_for_a_processor().saturating_sub(waited_before);
        if cost.average < limit {
            return cost;
        }
        assert!(
            waited > neoteletext_script::limits::TIME_CHECK_MARGIN && attempt < STARVED_TRIES,
            "2,048 drawings, each tick {name}: a step takes {:?} on average, over {limit:?} (try {attempt}; the thread waited {waited:?} for a processor)",
            cost.average
        );
        eprintln!("{name}: try {attempt} took {:?} a step after its thread waited {waited:?} for a processor: measuring again", cost.average);
    }
    unreachable!("the last try returns or fails")
}

/// The ways a page of 2,048 drawings moves them each tick, as [`moved`]
/// runs them: what each does once the drawings are made.
const WAYS: [(&str, &str); 8] = [
    ("nothing", ""),
    ("the script's loop alone, over plain objects", "for (const o of plain) o.x = (o.x + 1) % 790;"),
    ("two reads a drawing, nothing written", "let sum = 0; for (const d of all) sum += d.x + d.x;"),
    ("one write a drawing of the value it has", "for (let i = 0; i < all.length; i++) all[i].x = xs[i];"),
    ("one write a drawing of a new value", "for (let i = 0; i < all.length; i++) { xs[i] = (xs[i] + 1) % 790; all[i].x = xs[i]; }"),
    ("a read and a write a drawing: d.x = (d.x + 1) % 790", "for (const d of all) d.x = (d.x + 1) % 790;"),
    ("x and y of every drawing: d.moveBy(1, 1)", "for (const d of all) d.moveBy(1, 1);"),
    (
        "a list's set of x from an Int32Array the script computed",
        "if (!globalThis.list) { globalThis.list = g.many(all); globalThis.values = Int32Array.from(xs); } \
         for (let i = 0; i < values.length; i++) values[i] = (values[i] + 1) % 790; list.set('x', values);",
    ),
];

/// TODO-7298's and TODO-7373's measurement: a commit a tick on a scene of
/// 2,048 drawings, every one of the page's 1,983 shown drawings moved by
/// its properties each tick, with where a step's time goes: the script's
/// own loop over as many plain objects, the reads alone, the writes
/// alone, the whole by assignments one by one, and a list's `set`.
///
/// The times are printed and none is held here (rdm TODO-7451): this
/// test runs beside the rest of the workspace's, and a step's time is
/// the machine's as much as the step's. Measured with every processor
/// taken twice over, the time a thread is counted as running was itself
/// twice a quiet machine's. What is held is what no load changes: each
/// way sends what it should, and a list's `set` sends byte for byte what
/// the same assignments one by one send. The budget is held by the timed
/// test below.
#[test]
fn a_commit_a_tick_on_2048_drawings_is_measured() {
    let mut bytes = Vec::new();
    for (name, each_tick) in WAYS {
        let cost = moved(each_tick);
        println!(
            "2,048 drawings, each tick {name}: a step takes {:?} on average and {:?} at most ({:?} on the wall clock); its commit is {} bytes; the receiver takes and composes it in {:?}",
            cost.average, cost.longest, cost.wall, cost.bytes, cost.receiver
        );
        bytes.push(cost.bytes);
    }
    let [nothing, plain, reads, same, new, read_and_write, both, list] = bytes[..] else { panic!("eight ways") };
    assert_eq!([nothing, plain, reads, same], [0; 4], "a tick that changes no drawing commits nothing");
    assert!(new > 0 && read_and_write == new && list == new, "one x of every drawing, however it is set, is one commit of {new} bytes: {bytes:?}");
    assert!(both > new, "x and y of every drawing is a longer commit: {bytes:?}");
}

/// Section 3.2's budget for a step (`STEP_TIME_BUDGET`, 12 ms) held for
/// the way to move very many drawings each tick, a list's `set`, on the
/// page of 2,048 drawings; and every other way held to the decoder's
/// deadline for a step's answer (TODO-7373 records what the assignments
/// one by one cost and why).
///
/// A timed test (DEVELOPMENT.md section 8; rdm TODO-7451): the figure is
/// of an optimised build on a machine with processors free, and holds on
/// no other. The times are those the thread was running, and a
/// measurement made while the thread waited for a processor is made
/// again, which is enough for the passing load of a gate's own steps and
/// not for a machine with every processor taken.
#[test]
#[ignore = "a timed test: a step's time on a page of 2,048 drawings against STEP_TIME_BUDGET, which needs a release build and a quiet machine; run it there with --release -- --ignored"]
fn a_lists_set_of_2048_drawings_keeps_to_the_steps_budget() {
    // Asked for in a build it cannot hold in, it fails and says so.
    let optimised = !cfg!(debug_assertions);
    assert!(optimised, "the budget is for an optimised build: run with --release");
    for (place, (name, each_tick)) in WAYS.iter().enumerate() {
        let limit = if place + 1 == WAYS.len() { neoteletext_script::limits::STEP_TIME_BUDGET } else { STEP_DEADLINE };
        let cost = moved_within(name, each_tick, limit);
        println!("2,048 drawings, each tick {name}: a step takes {:?} on average and {:?} at most, within {limit:?}", cost.average, cost.longest);
    }
}

/// Section 6.4's first worked example, as the specification has it: a
/// painted background with a title, a caption painted gradually, and a
/// button with receiver-run looks. Its text is drawn by the receiver in
/// the engine's provided character sets; the caption's `painted` plays a
/// sound; the button's release navigates.
#[test]
fn the_specifications_page_example_runs_and_matches_its_kept_pictures() {
    let mut open = regis_opening(ContextId::Page);
    open.keys = vec!["Enter".to_owned()];
    open.flags.pointer_move = false;
    let mut container = container();
    container.open_with(open, include_bytes!("regis_page/example-page.js")).expect("the page opens");
    let mut receiver = Receiver::new();
    let mut wrong = Vec::new();
    let first = container.step(&tick(0, 0)).unwrap();
    assert_eq!(lines(&first), Vec::<String>::new());
    receiver.take(&first);
    assert_eq!(receiver.commits, 1);
    // The caption is drawing 2; the button is drawing 6, after its three faces.
    assert_eq!(first.watch.as_ref().and_then(|watch| watch.painted.clone()), Some(vec![2]));
    assert_eq!(receiver.terminal.label(6).as_deref(), Some("Start"));
    keep(GOLDEN, "example-page-at-first", &mut receiver, &mut wrong);
    let mut painted_at = None;
    for at in 1..200_u32 {
        let _ = receiver.terminal.advance_ticks(1);
        let painted: Vec<i32> = receiver.terminal.take_painted().into_iter().map(i32::from).collect();
        let batch = container.step(&Step { painted: painted.clone(), ..tick(at, u64::from(at)) }).unwrap();
        receiver.take(&batch);
        if !painted.is_empty() {
            assert_eq!(
                view(&batch).sound,
                vec![protocol::script::batch::SoundOperation::Play { voice: 0, sound: 1, gain: 100 }],
                "the caption's painted plays sound 1"
            );
            painted_at = Some(at);
            break;
        }
    }
    // Twenty-six character cells of 9 by 20 at 2,000 work units a second.
    let painted_at = painted_at.expect("the caption was reported painted");
    assert!((100..180).contains(&painted_at), "painted at tick {painted_at}");
    keep(GOLDEN, "example-page-painted", &mut receiver, &mut wrong);
    // The pointer over the button: the receiver shows its hover look.
    let picture = receiver.picture();
    let pick = picture.pick(400, 320);
    assert_eq!(pick.drawing, 6);
    assert!(receiver.terminal.set_viewer_state(ViewerState { hovered: 6, ..ViewerState::default() }));
    keep(GOLDEN, "example-page-hovered", &mut receiver, &mut wrong);
    let at = painted_at + 1;
    let release = pointer(PointerAction::UP, 400, 320, 6, (pick.x, pick.y), &[]);
    let batch = container.step(&with_events(at, u64::from(at), vec![release])).unwrap();
    assert_eq!(batch.navigate.as_deref(), Some("106"), "the button's release starts");
    assert!(wrong.is_empty(), "pictures differ from tests/golden/regis.txt (format {GOLDEN_FORMAT}):\n{}", wrong.join("\n"));
}

/// Section 6.4's second worked example, as the specification has it: a
/// game of drawings moved by their properties, one commit a step, its
/// score drawn as text again whenever it changes.
#[test]
fn the_specifications_game_example_runs_and_matches_its_kept_picture() {
    let mut open = regis_opening(ContextId::Page);
    open.keys = vec!["ArrowLeft".to_owned(), "ArrowRight".to_owned()];
    open.tick_divisor = 2;
    let mut container = container();
    container.open_with(open, include_bytes!("regis_page/example-game.js")).expect("the game opens");
    let mut receiver = Receiver::new();
    let mut wrong = Vec::new();
    let mut log = Vec::new();
    let mut notes = 0;
    for at in 0..400_u32 {
        // The right arrow is held from the second step: the boat sails
        // right to its stop, under the crates that fall there.
        let events = if at == 1 { vec![common::key(EventKind::KeyDown, "ArrowRight", 0)] } else { Vec::new() };
        let bounds = receiver.bounds(&[4]);
        let batch = container.step(&Step { bounds, ..with_events(at, u64::from(at), events) }).unwrap();
        log.extend(lines(&batch));
        notes += view(&batch).sound.iter().filter(|operation| matches!(operation, protocol::script::batch::SoundOperation::Midi { status: 0x90, .. })).count();
        if at > 0 && at % 2 == 0 {
            assert!(batch.regis.as_ref().is_some_and(|bytes| bytes.ends_with(b"D(C)")), "a tick's moves are one commit (step {at})");
        }
        receiver.take(&batch);
    }
    assert_eq!(log, Vec::<String>::new());
    // The boat is drawing 4, tracked, and at its stop.
    assert_eq!(receiver.terminal.drawing_position(4), Some((720, 380)));
    assert_eq!(receiver.bounds(&[4]), vec![Bounds { handle: 4, x: 720, y: 380, width: 80, height: 21 }]);
    println!("the game: {} commits, {} bytes of ReGIS, {notes} crates caught in 400 steps", receiver.commits, receiver.bytes);
    keep(GOLDEN, "example-game-after-400-steps", &mut receiver, &mut wrong);
    assert!(wrong.is_empty(), "pictures differ from tests/golden/regis.txt (format {GOLDEN_FORMAT}):\n{}", wrong.join("\n"));
}

/// Section 6.3 as JavaScript: the painter's calls, each ReGIS's own;
/// macrographs; measuring; and the kinds of error section 3.8 gives.
#[test]
fn the_painter_macrographs_and_measure_as_a_script_sees_them() {
    let body = r##"
const g = nt.regis;
const d = g.drawing({ x: 100, y: 100 });
const star = g.macrograph((p) => p.ink(3).vector("0246", { multiplier: 8 }));
check("a macrograph", star instanceof Macrograph && Object.isFrozen(star));
d.draw((p) => {
  const same = p.moveTo(0, 0).lineBy(5, 5).vector("7").circle(20).circleThrough(30, 0).arc(0, 0, 90).moveTo(0, 0)
    .curve([[0, 0], [10, 20], [20, 0]]).curve([[0, 0], [10, 20], [20, 0]], { closed: true })
    .mode("complement").mode("overlay").negative(true).negative(false).pattern(4).pattern("11110000", { multiplier: 3 }).pattern(1)
    .planes(15).shade(true).shade({ toY: 50 }).shade({ toX: 50 }).shade({ char: "#" }).shade(false)
    .text("Score 12").text("Grüße", { size: 2, language: "de" }).text("up", { cell: [12, 24], unit: [8, 20], height: 3, tilt: 90, italic: 22, spacing: [0, -14] })
    .call(star);
  check("each call returns the painter", same === p);
});
star.redefine((p) => p.ink(2).circle(6));
equal("measure", JSON.stringify(g.measure("HELLO")), '{"width":45,"height":20}');
equal("measure at a size", JSON.stringify(g.measure("HELLO", { size: 3 })), '{"width":135,"height":45}');
equal("measure a cell", JSON.stringify(g.measure("ab", { cell: [12, 30], height: 4 })), '{"width":24,"height":40}');
equal("measure in a language", g.measure("Grüße", { language: "de" }).width, 45);
const used = nt.budget().macrographBytes;
check("the macrographs' bytes are counted", used.used > 0 && used.limit === 14336);
const paint = (fn) => () => d.draw(fn);
throws("a vector's digits", TypeError, "a pixel vector is a string", paint((p) => p.vector("089")));
throws("a mode", TypeError, "a writing mode is", paint((p) => p.mode("xor")));
throws("a pattern", TypeError, "a pattern is 0 to 9", paint((p) => p.pattern("101")));
throws("a pattern's multiplier", RangeError, "MAX_PATTERN_MULTIPLIER", paint((p) => p.pattern(1, { multiplier: 17 })));
throws("planes", RangeError, "MAX_PLANES", paint((p) => p.planes(16)));
throws("shade", TypeError, "shade takes", paint((p) => p.shade("yes")));
throws("an arc", TypeError, "an arc is", paint((p) => p.arc(0, 0, 400)));
throws("text is a string", TypeError, "text takes a string", paint((p) => p.text(5)));
throws("a text option", TypeError, "'colour' is not an option of text", paint((p) => p.text("x", { colour: 1 })));
throws("a text size", RangeError, "MAX_TEXT_SIZE", paint((p) => p.text("x", { size: 17 })));
throws("a text cell", TypeError, "cell is [width, height]", paint((p) => p.text("x", { cell: 12 })));
throws("a tilt", TypeError, "a text tilt is", paint((p) => p.text("x", { tilt: 30 })));
throws("a character no set shows", TypeError, "no provided character set", paint((p) => p.text("☃")));
throws("a language", TypeError, "'xx' is not a language", paint((p) => p.text("x", { language: "xx" })));
throws("a long string", RangeError, "MAX_TEXT_CHARS", paint((p) => p.text("x".repeat(1001))));
throws("call takes a macrograph", TypeError, "call takes a Macrograph", paint((p) => p.call(d)));
throws("a Macrograph is not constructed", TypeError, "a Macrograph is made by", () => new Macrograph());
d.draw((p) => p.moveTo(40, 0).arc(0, 0, 90).lineTo(-5, 0).call(star).lineTo(-9, 0).text("ab").lineTo(-1, -1));
throws("a macrograph's body has no position", TypeError, "a macrograph is drawn wherever it is called", () => g.macrograph((p) => p.lineTo(-5, 0)));
throws("a macrograph's text", TypeError, "a macrograph's text does not hold", () => g.macrograph((p) => p.text("a@;b")));
throws("measure", TypeError, "measure takes a string", () => g.measure(5));
equal("a refused draw leaves the contents", nt.budget().contentBytes.used > 30, true);
done();
"##;
    let mut container = regis_page(&script(body));
    let batch = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&batch), 28);
    let mut receiver = Receiver::new();
    receiver.take(&batch);
    assert_eq!(receiver.commits, 1);
    let sent = String::from_utf8_lossy(batch.regis.as_ref().unwrap()).into_owned();
    assert!(sent.starts_with("D(S1(X100,Y100))@:AW(I3)V(W(M8))0246@;D(B1)P[0,0]V[+5,+5]V(W(M1))7C[+20]C[30,0]C(C,A+90)[0,0]P[0,0]"), "{sent}");
    assert!(sent.contains("@AD(E)@:AW(I2)C[+6]@;"), "{sent}");
    // After an arc, a macrograph's call and text, a line to a place with
    // a negative coordinate is written from where the renderer says the
    // position is (TODO-7389).
    assert!(sent.ends_with("D(B1)P[40,0]C(C,A+90)[0,0]V[-5,+40]@AV[-4,+0]T(A101)\"ab\"T(A0)V[-10,-1]D(E)D(C)"), "{sent}");
    for written in [
        "W(C)W(V)W(N1)W(N0)W(P4(M1))W(P11110000(M3))W(P1(M1))W(F15)W(S1)W(S[,50])W(S(X)[50])W(S'#')W(S0)",
        "T(A101)\"Score 12\"T(A0)",
        "T(S2)T(A102)\"Gr",
        "T(S[12,24],U[8,20],H3,D90,I22)[+0,-14]T(A101)\"up\"T(A0)",
    ] {
        assert!(sent.contains(written), "{written} is not in {sent}");
    }
}

/// TODO-7373: a callback's assignments to x, y, depth, visible and
/// detectable are gathered on the script's side and given to the host
/// together, before any other ReGIS call and before the step's commit; a
/// read is answered from what was assigned; and `nt.regis.many` sets one
/// attribute of a list of drawings from a typed array. What the receiver
/// then holds is what assignments one by one would have given it.
#[test]
fn assignments_are_gathered_and_lists_set_many_drawings_at_once() {
    let body = r#"
const g = nt.regis;
const a = g.drawing({ x: 1, y: 2 });
const b = g.drawing({ visible: false });
a.x = 5; a.x = 6; a.y = a.x + 10; a.depth = -3; b.visible = true; b.detectable = true;
equal("a read is what was assigned", JSON.stringify([a.x, a.y, a.depth, b.visible, b.detectable]), "[6,16,-3,true,true]");
// An assignment is in its place among the other calls: the label is
// staged after the x before it and before the x after it.
a.label = "between";
a.x = 7;
throws("a value out of range is refused where it is assigned", RangeError, "MAX_COORDINATE", () => { a.x = 40000; });
throws("a value of the wrong kind", TypeError, "visible is true or false", () => { b.visible = 1; });
equal("and changes nothing", a.x, 7);
// What a promise's job assigns leaves with the same step.
Promise.resolve().then(() => { b.y = 99; });
const all = [a, b, g.drawing({})];
const list = g.many(all);
check("a list", list instanceof DrawingList && list.length === 3 && Object.isFrozen(list));
nt.addEventListener("tick", (e) => {
  if (e.tick === 1) {
    equal("the job's assignment", b.y, 99);
    list.set("x", Int32Array.of(100, 200, 300));
    list.moveTo(Int32Array.of(10, 20, 30), Int32Array.of(11, 21, 31));
    list.set("visible", Int32Array.of(1, 0, 1));
    equal("after a list's set a drawing reads what the host holds", JSON.stringify([a.x, a.y, b.x, b.visible, all[2].y]), "[10,11,20,false,31]");
    a.x = a.x + 1;
    throws("a list's attributes", TypeError, "a list sets", () => list.set("layer", Int32Array.of(1, 1, 1)));
    throws("as many values as drawings", TypeError, "values is an Int32Array", () => list.set("x", Int32Array.of(1, 2)));
    throws("values are an Int32Array", TypeError, "values is an Int32Array", () => list.set("x", [1, 2, 3]));
    throws("a value out of range names its entry", RangeError, "MAX_COORDINATE", () => list.set("y", Int32Array.of(1, 40000, 3)));
    equal("a refused set changes nothing", b.y, 21);
    throws("a list is of drawings", TypeError, "a drawing of the list is a Drawing", () => g.many([a, null]));
    throws("a DrawingList is not constructed", TypeError, "a DrawingList is made by", () => new DrawingList());
    all[2].remove();
    throws("a removed drawing's assignment", TypeError, "the drawing was removed", () => { all[2].x = 1; });
    throws("a list with a removed drawing", TypeError, "the drawing was removed", () => list.set("x", Int32Array.of(1, 2, 3)));
    done();
  }
});
"#;
    let mut container = regis_page(&script(body));
    let mut receiver = Receiver::new();
    let first = container.step(&tick(0, 0)).unwrap();
    assert_eq!(
        String::from_utf8_lossy(first.regis.as_ref().unwrap()),
        "D(S1(X1,Y2))D(S2(V0))D(S1(X6,Y16,L-3))D(S2(V1,I1))D(S1(U'between',X7))D(S3(X0))D(S2(Y99))D(C)"
    );
    receiver.take(&first);
    let second = container.step(&tick(1, 1)).unwrap();
    assert_checks(&[lines(&first), lines(&second)].concat(), 16);
    receiver.take(&second);
    assert_eq!(receiver.commits, 2, "one commit a step");
    assert_eq!([receiver.terminal.drawing_position(1), receiver.terminal.drawing_position(2)], [Some((11, 11)), Some((20, 21))]);
    assert!(receiver.terminal.committed_drawing(3).is_none());
}
