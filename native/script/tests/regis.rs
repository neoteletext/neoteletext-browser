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

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Opens as _, all_lines, assert_checks, claim, container, ints_event, limited, lines, opening, script, tick, view, with_events};
use neoteletext_rules::animate::OperationKind;
use neoteletext_script::Container;
use neoteletext_script::limits::regis::{CHANNEL_SHARE_DRAWINGS, REGIS_MAX_DRAWINGS};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES, WASM_MEMORY_BYTES};
use neoteletext_script::ops::PointerAction;
use protocol::script::{Batch, Bounds, ContextId, ContextOpen, EventKind, Renderer, Step};
use regis::{Event as RegisEvent, ForcedLook, Picture, SCREEN_HEIGHT, SCREEN_WIDTH, Screen, Terminal, ViewerState};

/// The version of the kept pictures' format: the digest of a picture's
/// 800 by 480 colour-map addresses followed by its sixteen colours. A
/// change to what is painted raises it and writes the file again.
const GOLDEN_FORMAT: u32 = 1;
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

/// What the decoder and its renderer are to a script's batches: the
/// renderer's own terminal, fed each batch's ReGIS and animation
/// operations, and asked for the picture, the painted report, bounds and
/// picks.
struct Receiver {
    terminal: Terminal,
    screen: Screen,
    /// The commits applied.
    commits: u32,
    /// Every byte of ReGIS taken.
    bytes: usize,
}

impl Receiver {
    fn new() -> Self {
        // The fonts are the receiver's, given as the decoder gives them.
        let mut terminal = Terminal::new();
        fonts::give(&mut terminal);
        Self { terminal, screen: Screen::new(), commits: 0, bytes: 0 }
    }

    /// Takes a batch as the decoder does: its ReGIS appended to the
    /// page's one stream, then its animation operations, then its watch
    /// list. The receiver never refuses a commit: the host's validator is
    /// the renderer's own.
    #[track_caller]
    fn take(&mut self, batch: &Batch) {
        let _ = view(batch);
        if let Some(bytes) = &batch.regis {
            assert!(bytes.len() <= MAX_BATCH_BYTES);
            self.bytes += bytes.len();
            let events = self.terminal.feed_regis(&mut self.screen, bytes);
            assert!(!events.iter().any(|event| matches!(event, RegisEvent::CommitFailed { .. })), "the receiver refused what the host sent: {events:?}");
            self.commits += u32::try_from(events.iter().filter(|event| matches!(event, RegisEvent::Committed(_))).count()).unwrap();
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
            assert!(self.terminal.set_animation(operation.target.number, state), "{operation:?}");
        }
        if let Some(painted) = batch.watch.as_ref().and_then(|watch| watch.painted.as_ref()) {
            let numbers: Vec<u16> = painted.iter().map(|handle| u16::try_from(*handle).unwrap()).collect();
            assert!(self.terminal.watch_painted(&numbers));
        }
    }

    fn picture(&mut self) -> Picture {
        let mut picture = Picture::new();
        self.terminal.compose(&self.screen, &mut picture);
        picture
    }

    /// The bounds of the drawings `numbers`, as a step reports them.
    fn bounds(&self, numbers: &[u16]) -> Vec<Bounds> {
        numbers
            .iter()
            .filter_map(|number| {
                let found = self.terminal.bounds(*number)?;
                let int = |value: u32| i32::try_from(value).unwrap();
                Some(Bounds { handle: i32::from(*number), x: int(found.x), y: int(found.y), width: int(found.width), height: int(found.height) })
            })
            .collect()
    }
}

/// The digest of a picture: its pixels, then its colours.
fn digest(picture: &Picture, terminal: &Terminal) -> String {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    context.update(picture.canvas().pixels());
    for colour in terminal.palette().colours() {
        context.update(&colour);
    }
    context.finish().as_ref().iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// A PNG chunk: its length, type, data and CRC-32 (the PNG
/// specification, section 5.3; the CRC is ISO 3309's, polynomial
/// 0xEDB88320 reflected).
fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    let start = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let mut crc = u32::MAX;
    for byte in &out[start..] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

/// Writes a picture as a PNG with a palette, its pixel data stored and
/// not compressed (RFC 1950 and RFC 1951 stored blocks), where a person
/// can look at it; gives the file's path.
fn write_png(name: &str, picture: &Picture, terminal: &Terminal) -> PathBuf {
    let (width, height) = (usize::try_from(SCREEN_WIDTH).unwrap(), usize::try_from(SCREEN_HEIGHT).unwrap());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&u32::try_from(width).unwrap().to_be_bytes());
    header.extend_from_slice(&u32::try_from(height).unwrap().to_be_bytes());
    // Eight bits a pixel, colour type 3 (a palette), no interlace.
    header.extend_from_slice(&[8, 3, 0, 0, 0]);
    chunk(&mut png, *b"IHDR", &header);
    chunk(&mut png, *b"PLTE", &terminal.palette().colours().concat());
    let mut raw = Vec::new();
    for row in picture.canvas().pixels().chunks(width) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut data = vec![0x78, 0x01];
    let (mut low, mut high) = (1_u32, 0_u32);
    let blocks = raw.chunks(65_535).count();
    for (index, block) in raw.chunks(65_535).enumerate() {
        let length = u16::try_from(block.len()).unwrap();
        data.push(u8::from(index + 1 == blocks));
        data.extend_from_slice(&length.to_le_bytes());
        data.extend_from_slice(&(!length).to_le_bytes());
        data.extend_from_slice(block);
        for byte in block {
            low = (low + u32::from(*byte)) % 65_521;
            high = (high + low) % 65_521;
        }
    }
    data.extend_from_slice(&((high << 16) | low).to_be_bytes());
    chunk(&mut png, *b"IDAT", &data);
    chunk(&mut png, *b"IEND", &[]);
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("regis-{name}.png"));
    std::fs::write(&path, png).expect("the picture is written");
    path
}

/// Compares the receiver's picture with the kept one named `name`, and
/// writes it as a PNG. The digests that differ are gathered in `wrong`,
/// so that one run shows every picture that changed.
fn keep(name: &str, receiver: &mut Receiver, wrong: &mut Vec<String>) {
    let picture = receiver.picture();
    let found = digest(&picture, &receiver.terminal);
    let path = write_png(name, &picture, &receiver.terminal);
    let mut kept = GOLDEN.lines().filter(|line| !line.starts_with('#'));
    assert_eq!(kept.next(), Some(format!("format {GOLDEN_FORMAT}").as_str()), "the kept pictures' format");
    let line = format!("{name} {found}");
    if !kept.any(|kept| kept == line) {
        wrong.push(format!("{line}    (see {})", path.display()));
    }
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
    keep("page-at-first", &mut receiver, &mut wrong);
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
    keep("page-painted", &mut receiver, &mut wrong);
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
    keep("page-pressed-by-the-viewer", &mut receiver, &mut wrong);
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
    keep("page-frame-2-and-forced-pressed", &mut receiver, &mut wrong);
    // The right arrow: the sprite moved by properties (one commit), a
    // cycle started, the look given back to the pointer.
    let batch = container.step(&with_events(at + 3, u64::from(at + 3), vec![key("ArrowRight")])).unwrap();
    assert_eq!(batch.regis.as_deref(), Some(b"D(S12(X640,Y280))D(C)".as_slice()));
    receiver.take(&batch);
    keep("page-sprite-moved", &mut receiver, &mut wrong);
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
        let began = Instant::now();
        let Ok(batch) = container.step(&tick(at, u64::from(at))) else { break };
        cost.longest_step = cost.longest_step.max(began.elapsed());
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

/// TODO-7298's measurement: a commit a tick on a scene of 2,048 drawings
/// (the page's 1,984 and the channel context's 64), every one of the
/// page's moved by its properties each tick. Printed: the time of a
/// step, of which the host's validation of the commit is a part, and the
/// bytes of a step's commit. Held only to the decoder's deadline for a
/// step's answer, since the figure depends on the machine.
#[test]
fn a_commit_a_tick_on_2048_drawings_is_measured() {
    /// The ticks measured: a second.
    const TICKS: u32 = 60;
    let page = r#"
const g = nt.regis;
const shape = g.symbol((p) => p.ink(3).fill((q) => q.rect(0, 0, 6, 6)));
const all = [];
let building = true;
nt.addEventListener("tick", (e) => {
  // 128 drawings a tick, until the page's share is made.
  if (building) {
    try { for (let n = 0; n < 128; n++) all.push(g.drawing({ source: shape, layer: 2, x: (all.length * 13) % 790, y: (all.length * 7) % 470 })); }
    catch (error) { building = false; console.log("made " + all.length + ": " + error.message.split(":")[0]); }
    return;
  }
  for (const d of all) d.x = (d.x + 1) % 790;
});
"#;
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
    let (mut total, mut longest, mut bytes) = (Duration::ZERO, Duration::ZERO, 0_usize);
    let mut compose = Duration::ZERO;
    for _ in 0..TICKS {
        let began = Instant::now();
        let batch = container.step(&tick(at, u64::from(at))).unwrap();
        let took = began.elapsed();
        total += took;
        longest = longest.max(took);
        bytes = bytes.max(batch.regis.as_ref().map_or(0, Vec::len));
        assert_eq!(lines(&batch), Vec::<String>::new());
        let began = Instant::now();
        receiver.take(&batch);
        let _ = receiver.picture();
        compose += began.elapsed();
        at += 1;
    }
    assert_eq!(receiver.commits, 16 + TICKS, "a commit a tick");
    println!(
        "a commit a tick on {REGIS_MAX_DRAWINGS} drawings, {} moved each tick: a step takes {:?} on average and {longest:?} at most; a step's commit is {bytes} bytes; \
         the host holds {} bytes; the receiver takes and composes it in {:?} on average",
        REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS,
        total / TICKS,
        container.host_bytes(),
        compose / TICKS
    );
    assert!(longest < STEP_DEADLINE, "a step took {longest:?}");
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
    keep("example-page-at-first", &mut receiver, &mut wrong);
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
    keep("example-page-painted", &mut receiver, &mut wrong);
    // The pointer over the button: the receiver shows its hover look.
    let picture = receiver.picture();
    let pick = picture.pick(400, 320);
    assert_eq!(pick.drawing, 6);
    assert!(receiver.terminal.set_viewer_state(ViewerState { hovered: 6, ..ViewerState::default() }));
    keep("example-page-hovered", &mut receiver, &mut wrong);
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
    keep("example-game-after-400-steps", &mut receiver, &mut wrong);
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
throws("after an arc", TypeError, "the position is where an arc", paint((p) => p.moveTo(40, 0).arc(0, 0, 90).lineTo(-5, 0)));
throws("a macrograph's text", TypeError, "a macrograph's text does not hold", () => g.macrograph((p) => p.text("a@;b")));
throws("measure", TypeError, "measure takes a string", () => g.measure(5));
equal("a refused draw leaves the contents", nt.budget().contentBytes.used > 100, true);
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
    assert!(sent.ends_with("@AD(E)@:AW(I2)C[+6]@;D(C)"), "{sent}");
    for written in [
        "W(C)W(V)W(N1)W(N0)W(P4(M1))W(P11110000(M3))W(P1(M1))W(F15)W(S1)W(S[,50])W(S(X)[50])W(S'#')W(S0)",
        "T(A101)\"Score 12\"T(A0)",
        "T(S2)T(A102)\"Gr",
        "T(S[12,24],U[8,20],H3,D90,I22)[+0,-14]T(A101)\"up\"T(A0)",
    ] {
        assert!(sent.contains(written), "{written} is not in {sent}");
    }
}
