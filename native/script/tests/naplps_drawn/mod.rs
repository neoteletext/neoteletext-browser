//! What the NAPLPS tests share: painter programs built call by call, and
//! the NAPLPS renderer at the pinned commit set up as the browser's
//! decoder sets it up (`decoder/src/naplps.rs`): the receiver's canvas of
//! 480 by 460, a colour map of sixteen entries, and the engine's 55
//! character sets from its Noto Teletext glyphs with the joining data of
//! the plain Arabic-script sets.
#![allow(dead_code, clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::indexing_slicing, reason = "each test file uses its own part of this; tests")]

use std::sync::OnceLock;

use naplps::limits::{RECEIVER_CANVAS_HEIGHT, RECEIVER_CANVAS_WIDTH};
use std::collections::BTreeMap;

use naplps::{Animation, BitmapSet, Canvas, Event, JoinForm, JoiningType, Look, Object, ObjectKind, Player, Rgb, StrokeFont, Terminal};
use neoteletext_core::charset::CharRef;
use neoteletext_core::glyphs::conformance::FONT_SETS;
use neoteletext_core::glyphs::{GlyphRows, GlyphSet, JOINED_SETS, JoiningClass, joining_in};
use neoteletext_rules::animate::OperationKind;
use neoteletext_script::host::naplps::Fault;
use neoteletext_script::host::naplps::program::Paint;
use neoteletext_script::host::naplps::writer::{NoDefinitions, Painter, Target};
use protocol::script::Batch;
use protocol::script::batch::NaplpsObjectKind;

/// The canvas's size.
pub const WIDTH: u32 = RECEIVER_CANVAS_WIDTH;
/// See [`WIDTH`].
pub const HEIGHT: u32 = RECEIVER_CANVAS_HEIGHT;
/// The colour map's address bits, as the decoder's.
const PALETTE_BITS: u32 = 4;
/// The room a test's painting is given.
const ROOM: usize = 1 << 20;

static GLYPHS: GlyphSet<'static> = GlyphSet::noto_teletext();

/// The glyph of code `code` of set `set`: twenty rows from the top, the
/// most significant of twelve bits the leftmost dot.
pub fn glyph(set: u8, code: u8) -> GlyphRows {
    GLYPHS.glyph(CharRef::new(FONT_SETS[usize::from(set)].1, code))
}

fn sets() -> &'static [BitmapSet] {
    static SETS: OnceLock<Vec<BitmapSet>> = OnceLock::new();
    SETS.get_or_init(|| {
        (0_u8..)
            .zip(FONT_SETS.iter())
            .map(|(number, (_, set))| {
                let mut bitmap = BitmapSet::new(12, 20).unwrap();
                for code in 0x20..=0x7F_u8 {
                    bitmap.set_glyph(code, &GLYPHS.glyph(CharRef::new(*set, code))).unwrap();
                    let Some(joining) = JOINED_SETS.contains(&number).then(|| joining_in(&GLYPHS, number, code)).flatten() else { continue };
                    let class = match joining.class {
                        JoiningClass::None => JoiningType::None,
                        JoiningClass::Right => JoiningType::Right,
                        JoiningClass::Dual => JoiningType::Dual,
                        JoiningClass::Transparent => JoiningType::Transparent,
                    };
                    bitmap.set_joining(code, class).unwrap();
                    for (form, rows) in [(JoinForm::Final, &joining.final_form), (JoinForm::Initial, &joining.initial), (JoinForm::Medial, &joining.medial)] {
                        bitmap.set_form_glyph(code, form, rows).unwrap();
                    }
                }
                bitmap
            })
            .collect()
    })
}

/// A terminal as the decoder makes one for a generation.
pub fn terminal() -> Terminal {
    let mut terminal = Terminal::new(PALETTE_BITS);
    terminal.set_font(StrokeFont::parse(naplps_fonts::NEOTELETEXT).unwrap());
    for (number, set) in (0_u8..).zip(sets()) {
        terminal.set_teletext_set(number, set.clone()).unwrap();
    }
    terminal
}

/// A blank canvas.
pub fn canvas() -> Canvas {
    Canvas::new(WIDTH, HEIGHT).unwrap()
}

/// `bytes` drawn whole, every WAIT passed over.
pub fn draw(bytes: &[u8]) -> Canvas {
    let mut canvas = canvas();
    let mut terminal = terminal();
    terminal.feed(&mut canvas, bytes);
    terminal.finish(&mut canvas);
    let diagnostics = terminal.diagnostics();
    // Every byte written means something to the renderer: none is passed
    // over and nothing is refused.
    assert_eq!((diagnostics.passed_over, diagnostics.refusals), (0, [0; naplps::REFUSAL_KINDS]), "{diagnostics:?}");
    canvas
}

/// The events the renderer reports for `bytes`.
pub fn events(bytes: &[u8]) -> Vec<Event> {
    let mut canvas = canvas();
    let mut terminal = terminal();
    let mut events = terminal.feed(&mut canvas, bytes);
    events.extend(terminal.finish(&mut canvas));
    events
}

/// The colour-map entry at dot (`x`, `y`).
pub fn pixel(canvas: &Canvas, x: u32, y: u32) -> u8 {
    canvas.pixel(x, y).unwrap()
}

/// How many dots are not entry 0.
pub fn lit(canvas: &Canvas) -> usize {
    canvas.pixels().iter().filter(|entry| **entry != 0).count()
}

/// What opens a generation, ended so that it is drawn.
pub fn opening() -> Vec<u8> {
    let mut bytes = Vec::new();
    Painter::start_generation(&mut bytes);
    Painter::backdrop().end(&mut bytes);
    bytes
}

/// The bytes a painter of `target` writes for `program`, in English.
pub fn paint_as(target: Target, program: &Program) -> Result<Vec<u8>, Fault> {
    let mut painter = if target == Target::Backdrop {
        let mut painter = Painter::backdrop();
        painter.end(&mut Vec::new());
        painter
    } else {
        Painter::definition(target)
    };
    let mut out = Vec::new();
    painter.run(program.as_slice(), "en", &NoDefinitions, &mut out, ROOM, "ROOM")?;
    Ok(out)
}

/// A generation's opening and `program` painted on the backdrop.
pub fn paint(program: &Program) -> Vec<u8> {
    let mut bytes = opening();
    bytes.extend(paint_as(Target::Backdrop, program).unwrap());
    bytes
}

/// A painter program, built as the prelude builds one (section 10.2).
#[derive(Clone, Default)]
pub struct Program(Vec<i32>);

impl Program {
    pub fn new() -> Self {
        Self::default()
    }

    /// A program of exactly `words`.
    pub fn words(words: &[i32]) -> Self {
        Self(words.to_vec())
    }

    /// A call and its whole numbers.
    pub fn call(mut self, call: Paint, ints: &[i32]) -> Self {
        self.0.push(call.number());
        self.0.extend_from_slice(ints);
        self
    }

    /// A call that takes a list of pairs.
    pub fn points(mut self, call: Paint, points: &[(i32, i32)]) -> Self {
        self.0.push(call.number());
        self.0.push(i32::try_from(points.len()).unwrap());
        self.0.extend(points.iter().flat_map(|(x, y)| [*x, *y]));
        self
    }

    /// A string: its length, then its code points.
    pub fn text(mut self, text: &str) -> Self {
        self.0.push(i32::try_from(text.chars().count()).unwrap());
        self.0.extend(text.chars().map(|character| i32::try_from(u32::from(character)).unwrap()));
        self
    }

    /// A list of words: its length, then the words.
    pub fn list(mut self, words: &[i32]) -> Self {
        self.0.push(i32::try_from(words.len()).unwrap());
        self.0.extend_from_slice(words);
        self
    }

    pub fn as_slice(&self) -> &[i32] {
        &self.0
    }

    /// The program as a host call's `Int32Array` payload.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(|word| word.to_le_bytes()).collect()
    }
}

/// A frame cycle a receiver runs (animation.md ANIM-14): first, last,
/// period, once, and the tick it started at.
type Cycle = (u8, u8, u16, bool, u64);

/// A receiver playing a page's NAPLPS on the presentation clock, as the
/// decoder does: batches' bytes pushed, WAIT and BLINK played by the
/// renderer's `Player`, the objects of a batch given to the renderer,
/// the animation operations applied to their state (ANIM-6), cycles run
/// on the clock (ANIM-14), and the shown objects composed over the
/// stream's picture.
pub struct Receiver {
    pub terminal: Terminal,
    pub canvas: Canvas,
    pub player: Player,
    /// The picture presented: the stream's with the objects over it.
    presented: Canvas,
    state: BTreeMap<u16, (Animation, Option<Cycle>)>,
    pub tick: u64,
    /// The title the objects' record last gave.
    pub title: String,
}

impl Receiver {
    pub fn new() -> Self {
        Self { terminal: terminal(), canvas: canvas(), player: Player::new(), presented: canvas(), state: BTreeMap::new(), tick: 0, title: String::new() }
    }

    /// Takes a batch's NAPLPS: a new generation is a new terminal, canvas
    /// and player.
    pub fn take(&mut self, new_generation: bool, bytes: &[u8]) {
        if new_generation {
            let (tick, title) = (self.tick, core::mem::take(&mut self.title));
            *self = Self { tick, title, ..Self::new() };
        }
        self.player.push(&mut self.terminal, &mut self.canvas, bytes);
    }

    /// Takes everything of a batch that is NAPLPS's: the stream's bytes,
    /// the objects, then the animation operations.
    pub fn take_batch(&mut self, batch: &Batch) {
        if let Some(append) = &batch.naplps {
            self.take(append.new_generation, &append.bytes);
        }
        if let Some(record) = &batch.naplps_objects {
            let objects: Vec<Object> = record
                .objects
                .iter()
                .map(|object| Object {
                    number: object.number,
                    kind: if object.kind == NaplpsObjectKind::Region { ObjectKind::Region } else { ObjectKind::Macro },
                    shown: object.shown,
                    detectable: object.detectable,
                    layer: object.layer,
                    order: object.order,
                    x: object.x,
                    y: object.y,
                    width: object.width,
                    height: object.height,
                    origin_x: object.origin.0,
                    origin_y: object.origin.1,
                    cover: object.cover,
                    hover: object.looks[0],
                    pressed: object.looks[1],
                    alternative: object.looks[2],
                    frames: object.frames.clone(),
                    label: object.label.clone(),
                })
                .collect();
            if record.one {
                self.state.remove(&objects[0].number);
                self.terminal.replace_object(objects[0].clone()).unwrap();
            } else {
                self.state.clear();
                self.title.clone_from(&record.title);
                self.terminal.set_objects(&objects).unwrap();
            }
        }
        for operation in batch.animate.iter().flatten() {
            let number = operation.target.number;
            let Some(object) = self.terminal.object(number).cloned() else { continue };
            let (animation, cycle) = self.state.entry(number).or_default();
            match operation.kind {
                OperationKind::Show => animation.shown = Some(true),
                OperationKind::Hide => animation.shown = Some(false),
                OperationKind::Look(look) => animation.look = [Look::Normal, Look::Hover, Look::Pressed, Look::Alternative][usize::from(look)],
                OperationKind::Move { relative, x, y } => {
                    let (at_x, at_y) = animation.position.unwrap_or((object.x, object.y));
                    animation.position = Some(if relative { (at_x.saturating_add(x), at_y.saturating_add(y)) } else { (x, y) });
                }
                OperationKind::Order { layer, order } => {
                    if layer != 255 {
                        animation.layer = Some(layer);
                    }
                    animation.order = Some(order);
                }
                OperationKind::Frame(frame) => (animation.frame, *cycle) = (Some(frame), None),
                OperationKind::Cycle { first, last, period, once } => {
                    (animation.frame, *cycle) = (Some(first), Some((first, last, period, once, self.tick + 1)));
                }
                OperationKind::StopCycle => *cycle = None,
                OperationKind::Reset => (*animation, *cycle) = (Animation::default(), None),
            }
        }
    }

    /// Lets `ticks` pass.
    pub fn advance(&mut self, ticks: u32) {
        self.player.advance(&mut self.terminal, &mut self.canvas, ticks);
        self.tick += u64::from(ticks);
        for (animation, cycle) in self.state.values_mut() {
            let Some((first, last, period, once, start)) = *cycle else { continue };
            let Some(passed) = self.tick.checked_sub(start) else { continue };
            let (steps, count) = (passed / u64::from(period), u64::from(last - first) + 1);
            let frame = if once { (u64::from(first) + steps).min(u64::from(last)) } else { u64::from(first) + steps % count };
            animation.frame = Some(u8::try_from(frame).unwrap());
        }
    }

    /// The picture presented now: the objects, as their state stands,
    /// composed over the stream's picture.
    pub fn presented(&mut self) -> &Canvas {
        for (number, (animation, _)) in &self.state {
            self.terminal.set_animation(*number, *animation);
        }
        self.terminal.compose(&self.canvas, &mut self.presented).unwrap();
        &self.presented
    }

    /// The colour map as it shows now, blinks played.
    pub fn colours(&self) -> Vec<Rgb> {
        self.player.colours(&self.terminal)
    }
}

/// The version of the kept pictures' format: raised, with the pictures
/// regenerated in the same commit, by any change to what is drawn
/// (DEVELOPMENT.md section 8).
pub const PICTURE_FORMAT: u32 = 2;

/// A picture as the text a kept picture is: the format's version, the
/// size, the colour map as it shows, and each line of the canvas as runs
/// of `entry*count` (an entry in hexadecimal).
pub fn picture(canvas: &Canvas, colours: &[Rgb]) -> String {
    use std::fmt::Write as _;
    let mut text = format!("naplps-picture {PICTURE_FORMAT}\nsize {WIDTH} {HEIGHT}\ncolours");
    for [red, green, blue] in colours {
        write!(text, " {red:02x}{green:02x}{blue:02x}").unwrap();
    }
    text.push('\n');
    for line in canvas.pixels().chunks(WIDTH as usize) {
        let mut runs: Vec<(u8, usize)> = Vec::new();
        for entry in line {
            match runs.last_mut() {
                Some((last, count)) if last == entry => *count += 1,
                _ => runs.push((*entry, 1)),
            }
        }
        let line: Vec<String> = runs.iter().map(|(entry, count)| format!("{entry:x}*{count}")).collect();
        text.push_str(&line.join(" "));
        text.push('\n');
    }
    text
}

/// Compares a picture with the one kept as `name` beside the tests, or,
/// with `NAPLPS_BLESS` set, keeps it. With `NAPLPS_KEEP` naming a folder
/// it also writes the picture there as a PPM image, to be looked at.
#[track_caller]
pub fn expect(name: &str, canvas: &Canvas, colours: &[Rgb]) {
    let text = picture(canvas, colours);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/naplps_page/expect").join(format!("{name}.txt"));
    if let Some(folder) = std::env::var_os("NAPLPS_KEEP") {
        let mut image = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
        image.extend(canvas.pixels().iter().flat_map(|entry| colours[usize::from(*entry)]));
        std::fs::write(std::path::Path::new(&folder).join(format!("{name}.ppm")), image).unwrap();
    }
    if std::env::var_os("NAPLPS_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("no kept picture {}: {error}", path.display()));
    if kept != text {
        let line = kept.lines().zip(text.lines()).position(|(kept, drawn)| kept != drawn);
        panic!("the picture {name} differs from the one kept, first at line {line:?} of the file");
    }
}
