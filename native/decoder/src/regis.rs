//! The sub-pages a channel draws in ReGIS (DEC-6335): the bytes its REGIS
//! records carry (the engine's REQ-6336) go to the ReGIS renderer, one
//! terminal for each sub-page, and what it draws is sent to the trusted
//! core as a picture with its pick record.
//!
//! The bytes are untrusted. The renderer bounds what they cost to read and
//! what one repaint costs; here the number of sub-pages kept is bounded,
//! and how much repainting each may cost in a second (REQ-6380): a
//! picture is repainted only while its repaint budget has something in
//! hand, and a repaint held back is made once, of the latest scene, when
//! the budget allows.
//!
//! A sub-page's stream may be painted as a show (DEC-7031 item 5): a PAINT
//! record, read before the generation of the stream it names, gives a
//! default paint rate to that generation's numbered drawings that set
//! none (REQ-7025) and a line speed its bytes are fed to the renderer at
//! (REQ-7024), by the renderer's `Replay`, on the presentation clock. The
//! numbered drawings the open page's rule table names in its `painted`
//! triggers are watched, and each is told when it is whole (REQ-7026).

use std::collections::BTreeMap;

use neoteletext_rules::animate::{AnimationStore, Applied, Operation};
use neoteletext_rules::evaluator::{Bounds, TargetState};
use neoteletext_rules::table::{Target, TargetKind};
use neoteletext_web::records::regis::RegisRecord;
use protocol::regis::{PictureLanguage, REGIS_COLOURS, RegisPick, RegisPicture, RegisViewer};
use regis::limits::{MAX_ANIMATED_DRAWINGS, MAX_CYCLING_OBJECTS, MAX_WATCHED_PAINTINGS};
use regis::{Event, Font, ForcedLook, Pace, Picture, RepaintBudget, Replay, Rgb, Screen, Terminal, ViewerState};

use crate::animation::frames::Paint;
use crate::animation::{Counters, WholePicture, next_cycle_tick};

use crate::parts::{Changed, Figures, Sent, parts_to_send};

/// The most sub-pages whose ReGIS is kept. Each holds two pages of pixels,
/// a picture and its drawings: about a megabyte and a half before any
/// drawing is defined. One more arriving forgets the one fed longest ago.
pub const MAX_REGIS_PAGES: usize = 4;

/// Microseconds in a millisecond: the decoder's clock counts the first,
/// the repaint budget the second.
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;

/// The layer of a top-level drawing that names none (animation.md
/// ANIM-10: the attribute `G` defaults to 1, the scene).
const DEFAULT_LAYER: u8 = 1;

/// One sub-page's ReGIS.
struct Page {
    /// The stream being received.
    generation: u16,
    /// Where the next record of it must begin.
    next: u32,
    /// A record of it was lost: nothing more of it is drawn.
    lost: bool,
    terminal: Terminal,
    screen: Screen,
    picture: Picture,
    /// What it shows changed since its picture was last taken.
    changed: bool,
    /// The colour map as it stood when bytes were last fed: a map changed
    /// outside a batch changes no pixel and makes no commit, and is seen
    /// only here (the renderer's `docs/regis-v2.md`, "Rules for contents",
    /// rule 4).
    colours: [Rgb; REGIS_COLOURS],
    /// When it was last fed, by the count of records read.
    fed: u64,
    /// What the viewer's pointer is doing to it, as the trusted core said.
    viewer: RegisViewer,
    /// The numbered drawing that is its cursor (the engine's REQ-6371), or
    /// 0 for none: blinked while keys go to the channel (REQ-6378).
    cursor: u16,
    /// When the cursor's blinking last began again, by the decoder's
    /// clock; `None` while it does not blink.
    blink_from: Option<u64>,
    /// The cursor is to be shown for a full half period from the next
    /// time the clock is read: it changed, or the picture did.
    blink_again: bool,
    /// What bounds its repainting (the renderer's REQ-6368).
    budget: RepaintBudget,
    /// The time the budget was last credited to, in milliseconds of the
    /// decoder's clock; `None` before its first repaint, when it is full.
    earned_ms: Option<u64>,
    /// The picture last sent, which the next is sent as a part of
    /// (REQ-6399); `None` when the next must be whole.
    sent: Option<Sent>,
    /// The pick record and origins its last pick was built from, and that
    /// pick: built again only when they changed (RULE-65).
    picked: Option<Picked>,
    /// The animation state of its numbered drawings (animation.md ANIM-6),
    /// with the cycles the renderer has no clock to run (ANIM-14). The
    /// renderer is given each state as it changes, and holds the same.
    animation: Box<AnimationStore<MAX_ANIMATED_DRAWINGS>>,
    /// Whether it is painting and whether it is owed a repaint for its
    /// animation.
    motion: Motion,
    /// The presentation tick its paced work was last given the time to:
    /// the replay's bytes at the line speed and the drawings' painting at
    /// their rates. `None` while it has none, when no tick is counted:
    /// the ticks before work arrives are not that work's (REQ-7024,
    /// REQ-7025).
    paced: Option<u32>,
    /// What its animation has cost, not yet reported (ANIM-16).
    counters: Counters,
    /// Feeds the stream's bytes to the terminal: at once, or at the pace
    /// of the line speed its PAINT record stated (DEC-7031 item 5).
    replay: Replay,
    /// The watched drawings that became whole and were not yet told to the
    /// rule table, in ascending order, each once (REQ-7026).
    painted: Vec<u16>,
    /// Whether the whole picture's "painted" event is owed.
    whole: WholePicture,
}

/// What a sub-page's animation is doing.
#[derive(Clone, Copy, Default)]
struct Motion {
    /// A drawing is still being painted gradually (ANIM-15).
    painting: bool,
    /// Its animation state changed since its picture was last taken.
    animated: bool,
    /// The platform asks for reduced motion (REQ-754, renderer parity
    /// PAR-33): the cursor shows steadily and frame cycles do not advance.
    steady: bool,
}

/// A picture's pick record and origins, and the pick built from them.
struct Picked {
    record: Vec<u16>,
    origins: Vec<(u16, i32, i32)>,
    pick: RegisPick,
}

impl Picked {
    /// The pick of `record` and `origins`: the one held when neither
    /// changed, else built again and held. Comparing the record is one
    /// comparison of memory; building the pick reads it entry by entry,
    /// 384,000 of them, which took up to milliseconds at every repaint
    /// (RULE-65). The whole record is compared, not only the rows whose
    /// pixels changed: a drawing can become detectable, or stop being,
    /// without a pixel changing.
    fn pick(held: &mut Option<Self>, record: &[u16], origins: Vec<(u16, i32, i32)>) -> RegisPick {
        if let Some(held) = held.as_ref().filter(|held| held.origins == origins && held.record == record) {
            return held.pick.clone();
        }
        let pick = RegisPick::of(record, origins.iter().copied(), PictureLanguage::Regis);
        *held = Some(Self { record: record.to_vec(), origins, pick: pick.clone() });
        pick
    }
}

impl Page {
    /// A sub-page's ReGIS begun: `viewer` is what the pointer was doing to
    /// the stream before it, which a new stream does not change.
    /// `font` is the one the viewer chose for alphabet 0 (renderer parity
    /// PAR-35), or `None` for the one compiled in.
    fn new(generation: u16, viewer: RegisViewer, font: Option<&Font>) -> Self {
        let mut terminal = Terminal::new();
        if let Some(font) = font {
            terminal.set_font(font.clone());
        // The font compiled in is this program's own data: if it did not
        // parse, text would be drawn as blocks, as the renderer draws a
        // missing glyph.
        } else if let Ok(font) = Font::parse(regis_fonts::VT340) {
            terminal.set_font(font);
        }
        // Renderer parity PAR-12, PAR-13 and PAR-36 item 4: the engine's 55
        // provided sets as alphabets 100 to 154, with the joining data of
        // the plain Arabic-script sets (DEC-6669: a picture's text is drawn
        // in these and no other).
        for (number, set) in (0_u8..).zip(crate::provided::regis_sets()) {
            let _ = terminal.set_provided_set(number, set.clone());
        }
        let colours = terminal.palette().colours();
        Self {
            generation,
            next: 0,
            lost: false,
            terminal,
            screen: Screen::new(),
            picture: Picture::new(),
            changed: true,
            colours,
            fed: 0,
            viewer,
            cursor: 0,
            blink_from: None,
            blink_again: false,
            budget: RepaintBudget::new(),
            earned_ms: None,
            sent: None,
            picked: None,
            animation: Box::new(AnimationStore::new()),
            motion: Motion::default(),
            paced: None,
            counters: Counters::default(),
            replay: Replay::new(),
            painted: Vec::new(),
            whole: WholePicture::default(),
        }
    }

    /// Takes what the terminal did with bytes it was fed, by the stream's
    /// record or by the replay as the clock plays it. A commit, drawing
    /// outside numbered drawings, or a colour map that took effect changes
    /// what the sub-page shows: the map is one for the whole screen, and
    /// outside an open batch a change of it is immediate (the renderer's
    /// `docs/regis-v2.md`, "Rules for contents", rule 4), with no commit
    /// and no pixel written to say so. The other events ask for things a
    /// page does not do: a v1 pause or a reply to the host. With
    /// `at_once`, a drawing given a paint rate is whole at once.
    fn after_feed(&mut self, events: &[Event], at_once: bool) {
        let committed = events.iter().any(|event| matches!(event, Event::Committed(_)));
        if committed {
            // The last writer wins (ANIM-6): what the commit wrote over is
            // forgotten here as the renderer forgot it.
            self.resynchronise();
            // A drawing given a paint rate starts painting (ANIM-15); with
            // the viewer's setting off it is whole at once.
            self.motion.painting = self.terminal.advance_ticks(if at_once { u32::MAX } else { 0 });
        }
        let colours = self.terminal.palette().colours();
        let recoloured = colours != self.colours;
        self.colours = colours;
        self.changed |= committed | recoloured | self.screen.take_changed();
        // The channel moves the cursor by a commit after each key: it is
        // shown again at once, never hidden where the viewer looks for it.
        self.blink_again |= committed;
        self.note_painted();
    }

    /// Keeps the watched drawings the renderer says became whole since it
    /// was last asked (DEC-7031 item 2), for the rule table: asked after
    /// bytes are fed and after each call that can make a drawing whole.
    fn note_painted(&mut self) {
        for number in self.terminal.take_painted() {
            if !self.painted.contains(&number) && self.painted.len() < MAX_WATCHED_PAINTINGS {
                self.painted.push(number);
            }
        }
        self.painted.sort_unstable();
    }

    /// Watches the numbered drawings `drawings` for their painting
    /// finishing, in place of any watched before, and the whole picture
    /// when `whole` (DEC-7031 item 5): the open page's rule table was
    /// loaded, or dropped. Each drawing is watched anew, so that one whole
    /// already is told to the table just loaded.
    fn watch(&mut self, drawings: &[u16], whole: bool, at_once: bool) {
        self.painted.clear();
        let _ = self.terminal.take_painted();
        // Watching none first: a drawing named again would keep an episode
        // already told to the table before this one.
        let _ = self.terminal.watch_painted(&[]);
        let _ = self.terminal.watch_painted(drawings);
        // One of the calls that close an episode, so that the drawings
        // whole already are reported.
        self.motion.painting = self.terminal.advance_ticks(if at_once { u32::MAX } else { 0 });
        self.note_painted();
        self.whole.watch(whole, self.next != 0);
    }

    /// The viewer's setting "Paint drawings gradually" was turned off, or
    /// the platform asks for reduced motion (DEC-7031 item 5): the stream
    /// is paced no longer, what the replay held is drawn at once, and so
    /// is every drawing being painted.
    fn whole_at_once(&mut self) {
        self.replay.set_pace(None);
        let events = self.replay.advance(&mut self.terminal, &mut self.screen, 0);
        self.after_feed(&events, true);
        self.motion.painting = self.terminal.advance_ticks(u32::MAX);
        self.note_painted();
        self.changed = true;
    }
}

impl Page {
    /// The half period of the cursor's blink on the decoder's clock: 32
    /// ticks of the presentation clock (ANIM-1).
    const BLINK_US: u64 = neoteletext_core::consts::presentation_tick_due_us(interface_core::regis::BLINK_HALF_TICKS);

    /// Whether the cursor blinks: keys go to the channel and the sub-page
    /// has a cursor drawing.
    const fn blinks(&self) -> bool {
        !self.motion.steady && self.viewer.typing && self.cursor != 0
    }

    /// Gives the renderer the viewer state as it stands at `now_us`: the
    /// drawings hovered and pressed (REQ-6374), and the cursor drawing
    /// hidden through every second half period of its blink (REQ-6378).
    /// The blink begins again, with the cursor shown, when keys start
    /// going to the channel, when the cursor drawing changes and when a
    /// commit arrives, as a key typed moves the cursor by one. The picture
    /// is owed a repaint when the renderer says it may differ; a drawing
    /// with no look, and a cursor that does not exist, cost none.
    fn apply_viewer(&mut self, now_us: u64) {
        let mut hidden = 0;
        if self.blinks() {
            // The blink begins on the tick due now, so that the cursor
            // shows and hides on ticks of the presentation clock (ANIM-1).
            let from = match self.blink_from {
                Some(from) if !self.blink_again => from,
                _ => crate::animation::due_at(crate::animation::tick_at(now_us)),
            };
            self.blink_from = Some(from);
            let halves = now_us.saturating_sub(from).checked_div(Self::BLINK_US).unwrap_or(0);
            if !halves.is_multiple_of(2) {
                hidden = self.cursor;
            }
        } else {
            self.blink_from = None;
        }
        self.blink_again = false;
        let state = ViewerState { hovered: self.viewer.hovered, pressed: self.viewer.pressed, hidden };
        self.changed |= self.terminal.set_viewer_state(state);
    }

    /// When the cursor next shows or hides, if it blinks.
    fn next_blink_us(&self, now_us: u64) -> Option<u64> {
        let from = self.blink_from.filter(|_| self.blinks())?;
        let halves = now_us.saturating_sub(from).checked_div(Self::BLINK_US).unwrap_or(0);
        Some(from.saturating_add(halves.saturating_add(1).saturating_mul(Self::BLINK_US)))
    }

    /// Credits the repaint budget for the time since it was last credited.
    fn earn(&mut self, now_us: u64) {
        let milliseconds = now_us.checked_div(MICROSECONDS_PER_MILLISECOND).unwrap_or(0);
        if let Some(earned) = self.earned_ms {
            self.budget.earn(milliseconds.saturating_sub(earned));
        }
        self.earned_ms = Some(milliseconds);
    }
}

impl Page {
    /// Gives the renderer the animation state held for a drawing, or
    /// takes it away when none is held, and keeps what the renderer kept:
    /// it refuses a state for a drawing that does not exist, and the
    /// shown part of one whose showing would cost more than a repaint may
    /// (ANIM-9). The picture is owed a repaint when the renderer's state
    /// differs from before.
    fn give(&mut self, number: u16) {
        let target = Target::object(number);
        let before = self.terminal.animation(number);
        match self.animation.get(target).copied() {
            Some(state) => {
                let given = regis::AnimationState {
                    shown: state.shown,
                    position: state.position,
                    layer: state.layer,
                    order: state.order,
                    frame: state.frame,
                    look: ForcedLook::from_code(state.look).unwrap_or_default(),
                };
                if !self.terminal.set_animation(number, given) {
                    self.reconcile(number);
                }
            }
            None => self.terminal.clear_animation(number),
        }
        if self.terminal.animation(number) != before {
            self.changed = true;
            self.motion.animated = true;
        }
    }

    /// Makes the state held for a drawing what the renderer holds of it
    /// (ANIM-6, one state and the last writer wins): a commit that set
    /// the drawing's `X` or `Y` forgot the position, `V` the shown part,
    /// `L` or `G` the layer and order, and a deletion the whole state.
    /// The frame, the cycle and the forced look are not the stream's to
    /// write and stay.
    fn reconcile(&mut self, number: u16) {
        let target = Target::object(number);
        if self.terminal.drawing_position(number).is_none() {
            self.animation.forget(target);
            return;
        }
        let kept = self.terminal.animation(number);
        let Some(state) = self.animation.get_mut(target) else { return };
        state.shown = kept.and_then(|kept| kept.shown);
        state.position = kept.and_then(|kept| kept.position);
        state.layer = kept.and_then(|kept| kept.layer);
        state.order = kept.and_then(|kept| kept.order);
        if state.is_default() {
            self.animation.forget(target);
        }
    }

    /// After a commit: every state held is made what the renderer kept of
    /// it (the renderer's hand-over note: "the decoder re-reads
    /// `Terminal::animation(n)` after `Event::Committed`").
    fn resynchronise(&mut self) {
        let held: Vec<u16> = self.animation.entries().iter().map(|(target, _)| target.number).collect();
        for number in held {
            self.reconcile(number);
        }
    }

    /// Carries out one animation operation at presentation tick `tick`
    /// (ANIM-6); false when it did nothing, which is counted: its target
    /// is not a numbered drawing of the scene, or one more drawing than
    /// may would hold state, or one more than may would cycle (ANIM-14).
    fn animate(&mut self, operation: &Operation, tick: u32) -> bool {
        let number = operation.target.number;
        let own = (operation.target.kind == TargetKind::Object).then(|| self.terminal.drawing_position(number)).flatten();
        let applied = own.is_some_and(|own| self.animation.apply(operation, own, tick, MAX_CYCLING_OBJECTS) == Applied::Done);
        if applied {
            self.give(number);
        } else {
            self.counters.skipped = self.counters.skipped.saturating_add(1);
        }
        applied
    }

    /// Brings the sub-page to presentation tick `tick`: each cycling
    /// drawing whose frame is due is given its frame (ANIM-14: the caller
    /// sets the frame each tick), and the renderer is told the ticks that
    /// passed, which its drawings with a paint rate earn work by
    /// (ANIM-15). With `at_once`, the viewer's setting "Paint drawings
    /// gradually" off, every drawing is whole at once.
    ///
    /// The ticks that passed are counted from the last step that left a
    /// replay running or a drawing painting. A step that leaves neither
    /// stops the count, and the loop may then sleep for any time: bytes or
    /// a commit that arrive afterwards have their tick 0 at the step that
    /// follows them, as a new sub-page's have at its first, and are not
    /// given the ticks of the sleep (REQ-7024: bytes on an idle line wait
    /// their full time; REQ-7025: a drawing earns the ticks after its
    /// commit).
    fn advance(&mut self, tick: u32, at_once: bool) {
        let mut due = Vec::new();
        // Under reduced motion a frame cycle shows its first frame, the
        // frame in force before it starts, and does not advance (PAR-33).
        self.animation.advance(if self.motion.steady { 0 } else { tick }, &mut |target| due.push(target.number));
        for number in due {
            self.give(number);
        }
        let passed = self.paced.map_or(0, |paced| tick.saturating_sub(paced));
        // The bytes the line has delivered by this tick (REQ-7024), and
        // what their commits changed.
        if !self.replay.idle() {
            let events = self.replay.advance(&mut self.terminal, &mut self.screen, passed);
            self.after_feed(&events, at_once);
        }
        let was = self.motion.painting;
        self.motion.painting = self.terminal.advance_ticks(if at_once { u32::MAX } else { passed });
        // What the ticks bought is painted by the next repaint.
        if was && (passed > 0 || at_once) {
            self.changed = true;
            self.motion.animated = true;
        }
        self.paced = (!self.replay.idle() || self.motion.painting).then_some(tick);
        self.note_painted();
    }

    /// The next presentation tick after `tick` this sub-page needs the
    /// decoder's loop at: the next while a drawing is painting, else the
    /// earlier of the tick the replay next has a byte to feed (DEC-7031
    /// item 5) and the tick a cycle's frame next changes at (ANIM-1).
    fn next_tick(&self, tick: u32) -> Option<u32> {
        if self.motion.painting {
            return Some(tick.saturating_add(1));
        }
        let replay = self.replay.ticks_to_next().map(|ticks| tick.saturating_add(ticks.max(1)));
        // Under reduced motion no cycle advances (PAR-33).
        let cycle = if self.motion.steady { None } else { next_cycle_tick(self.animation.entries(), tick) };
        [replay, cycle].into_iter().flatten().min()
    }

    /// The state of a numbered drawing for a rule table's conditions
    /// (RULE-26, ANIM-13), or `None` when the scene has no such drawing.
    fn target_state(&self, number: u16) -> Option<TargetState> {
        let (x, y) = self.terminal.drawing_position(number)?;
        let state = self.animation.get(Target::object(number)).copied().unwrap_or_default();
        let cut = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
        let bounds = self.terminal.bounds(number).map(|bounds| Bounds {
            left: cut(bounds.x),
            top: cut(bounds.y),
            right: cut(bounds.x.saturating_add(bounds.width)),
            bottom: cut(bounds.y.saturating_add(bounds.height)),
        });
        // The drawing's committed attributes as the stream left them
        // (page-rules.md RULE-26: the renderer's `committed_drawing`), under
        // the animation state the rules gave it.
        let committed = self.terminal.committed_drawing(number);
        let order = state.order.or_else(|| committed.map(|committed| i8::try_from(committed.order.clamp(i32::from(i8::MIN), i32::from(i8::MAX))).unwrap_or(0)));
        Some(TargetState {
            shown: state.shown.or(committed.map(|committed| committed.shown)).unwrap_or(bounds.is_some()),
            cycling: state.cycle.is_some(),
            frame: state.frame,
            look: state.look,
            layer: state.layer.or(committed.map(|committed| committed.layer)).unwrap_or(DEFAULT_LAYER),
            order: order.unwrap_or(0),
            x,
            y,
            bounds,
        })
    }
}

/// A sub-page's picture, ready to send.
pub struct Drawn {
    /// The page.
    pub page: u16,
    /// The sub-code.
    pub sub: u16,
    /// The picture.
    pub picture: RegisPicture,
    /// Its pick record, when it differs from the last one sent.
    pub pick: Option<RegisPick>,
    /// Further parts of the same repaint, each laid over the one before:
    /// the other areas that changed, when they lie apart.
    pub more: Vec<RegisPicture>,
}

/// A sub-page's drawing links, whole: its page, its sub-code, and each
/// drawing with the target it leads to.
#[cfg(feature = "links")]
pub type DrawingLinks = (u16, u16, Vec<(u16, String)>);

/// A sub-page's drawing links being received: the parts of one sending.
#[cfg(feature = "links")]
struct LinkParts {
    key: (u16, u16),
    /// The part that must come next, and how many there are.
    next: u8,
    parts: u8,
    links: Vec<(u16, String)>,
}

// The trusted core is sent as many links as the engine's record carries.
#[cfg(feature = "links")]
const _: () = assert!(protocol::regis::MAX_REGIS_LINKS == neoteletext_web::records::drawing_links::MAX_DRAWING_LINKS);

/// Every sub-page's ReGIS a channel has sent.
#[derive(Default)]
pub struct RegisPages {
    pages: BTreeMap<(u16, u16), Page>,
    /// The sub-page whose picture was last sent for each page: the
    /// receiver holds one picture for a page, so a part is only ever of
    /// that sub-page's.
    sent_sub: BTreeMap<u16, u16>,
    /// Records read.
    records: u64,
    /// The drawing links being received, of one sub-page at a time.
    #[cfg(feature = "links")]
    link_parts: Option<LinkParts>,
    /// Sub-pages' drawing links received whole, for the trusted core.
    #[cfg(feature = "links")]
    links: Vec<DrawingLinks>,
    /// Every drawing is painted whole at once: the viewer's setting
    /// "Paint drawings gradually" is off (animation.md ANIM-15).
    at_once: bool,
    /// The platform asks for reduced motion (REQ-754, renderer parity
    /// PAR-33): every drawing is whole at once as well, cursors show
    /// steadily and frame cycles do not advance.
    reduced: bool,
    /// Each sub-page's sheet (ANIM-17): the stream of hidden drawings fed
    /// to its renderer before its own stream. A sub-page is a page
    /// (page-rules.md RULE-112): no sheet is fed to another sub-code.
    sheets: BTreeMap<(u16, u16), Sheet>,
    /// The font the viewer chose for alphabet 0 of every new terminal, or
    /// none for the one compiled in (renderer parity PAR-35).
    font: Option<Font>,
    /// The PAINT record waiting for each sub-page's stream: the generation
    /// it names and its values, at most one a sub-page (DEC-7031 item 1).
    /// It is applied when that generation starts and dropped when any
    /// other does.
    pending: BTreeMap<(u16, u16), (u16, Paint)>,
    /// What the open page's rule table watches the painting of (DEC-7031
    /// item 5), which a terminal made for its sub-page is told.
    watch: Option<Watch>,
}

/// The sub-page whose rule table has `painted` triggers, and what they
/// name (DEC-7031 item 4).
struct Watch {
    key: (u16, u16),
    /// The numbered drawings named, at most `MAX_WATCHED_PAINTINGS`.
    drawings: Vec<u16>,
    /// Whether the page's whole picture is named.
    whole: bool,
}

/// The most PAINT records kept waiting for their streams: one for each
/// sub-page kept. One more drops the lowest sub-page's.
const MAX_PENDING_PAINTS: usize = MAX_REGIS_PAGES;

/// The pace of a line of `speed` bits a second (REQ-7024): as many bytes
/// as its speed is bits a second, in the ticks the engine names.
const fn line_pace(speed: u32) -> Pace {
    Pace { bytes: speed, ticks: neoteletext_web::consts::LINE_PACE_TICKS }
}

/// A page's ReGIS sheet as received (ANIM-17).
#[derive(Default)]
struct Sheet {
    generation: u16,
    bytes: Vec<u8>,
    /// A record of it was lost: nothing more of this generation is kept.
    lost: bool,
}

/// The most sub-pages whose sheet is kept: a sheet is kept while its
/// sub-page is, and sub-pages are kept [`MAX_REGIS_PAGES`] at most.
const MAX_SHEETS: usize = MAX_REGIS_PAGES;

impl RegisPages {
    /// Reads one REGIS record: its bytes go to its sub-page's terminal.
    pub fn read(&mut self, record: &RegisRecord<'_>) {
        let key = (record.page.mpp(), record.subcode.value());
        self.records = self.records.saturating_add(1);
        // A first byte again, of the same generation, is the stream sent
        // again whole, as a channel does when the page is selected again
        // after it was drawn from its kept source (PAR-34): it starts over.
        let fresh = self.pages.get(&key).is_none_or(|page| page.generation != record.generation || (record.offset == 0 && page.next != 0));
        if fresh {
            // A stream is followed from its first byte: one joined part
            // way through waits for the next generation.
            if record.offset != 0 {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.lost = true;
                }
                return;
            }
            if !self.pages.contains_key(&key) && self.pages.len() >= MAX_REGIS_PAGES {
                let oldest = self.pages.iter().min_by_key(|(_, page)| page.fed).map(|(key, _)| *key);
                if let Some(oldest) = oldest {
                    self.pages.remove(&oldest);
                }
            }
            // What the pointer was doing, and which drawing is the cursor,
            // are not the stream's to change.
            let (viewer, cursor) = self.pages.get(&key).map(|page| (page.viewer, page.cursor)).unwrap_or_default();
            let mut page = Page::new(record.generation, viewer, self.font.as_ref());
            page.cursor = cursor;
            page.motion.steady = self.reduced;
            // The PAINT record waiting is this generation's or no stream's
            // (DEC-7031 item 1): one that names another generation is
            // dropped, and this stream is painted as one with none.
            let paint = self.pending.remove(&key).filter(|(generation, _)| *generation == record.generation).map(|(_, paint)| paint);
            // The default paint rate is in force from the first commit,
            // the sheet's among them (REQ-7025).
            if let Some(rate) = paint.and_then(|paint| paint.default_rate) {
                page.terminal.set_default_paint_rate(rate);
            }
            // The open page's table watches this sub-page's drawings.
            if let Some(watch) = self.watch.as_ref().filter(|watch| watch.key == key) {
                let _ = page.terminal.watch_painted(&watch.drawings);
                page.whole.watch(watch.whole, false);
            }
            // The page's sheet goes first (ANIM-17): the stream names its
            // hidden drawings as sources, looks and frames. It is fed at
            // once whatever the line speed: it draws nothing.
            if let Some(sheet) = self.sheets.get(&key).filter(|sheet| !sheet.bytes.is_empty()) {
                let _ = page.terminal.feed_regis(&mut page.screen, &sheet.bytes);
            }
            // The stream's bytes are fed at the line's speed (REQ-7024),
            // unless the viewer's setting is off or motion is reduced.
            if let Some(speed) = paint.and_then(|paint| paint.line_speed).filter(|_| !self.at_once && !self.reduced) {
                page.replay.set_pace(Some(line_pace(speed)));
            }
            self.pages.insert(key, page);
        }
        let Some(page) = self.pages.get_mut(&key) else { return };
        page.fed = self.records;
        if page.lost || record.offset != page.next {
            page.lost = true;
            return;
        }
        page.next = page.next.saturating_add(u32::try_from(record.data.len()).unwrap_or(u32::MAX));
        // Fed at once, or held by the replay for the clock to play.
        let events = page.replay.push(&mut page.terminal, &mut page.screen, record.data);
        page.after_feed(&events, self.at_once || self.reduced);
        page.whole.drew();
    }

    /// Takes a PAINT record for a sub-page's ReGIS stream (DEC-7031 item
    /// 1): kept, in place of any kept for the sub-page before, until the
    /// generation it names starts. The engine's reader has checked its
    /// values.
    pub fn read_paint(&mut self, page: u16, sub: u16, generation: u16, paint: Paint) {
        let key = (page, sub);
        if !self.pending.contains_key(&key) && self.pending.len() >= MAX_PENDING_PAINTS {
            self.pending.pop_first();
        }
        self.pending.insert(key, (generation, paint));
    }

    /// The open page's rule table names these numbered drawings of the
    /// sub-page `key`, and with `whole` its whole picture, in its
    /// `painted` triggers (DEC-7031 items 4 and 5); `None` when no table
    /// names any. The sub-page watched before is watched no longer.
    pub fn set_watch(&mut self, watch: Option<((u16, u16), Vec<u16>, bool)>) {
        let at_once = self.at_once || self.reduced;
        if let Some(held) = self.watch.take().and_then(|before| self.pages.get_mut(&before.key)) {
            held.watch(&[], false, at_once);
        }
        self.watch = watch.map(|(key, drawings, whole)| Watch { key, drawings, whole });
        if let Some(watch) = &self.watch
            && let Some(held) = self.pages.get_mut(&watch.key)
        {
            held.watch(&watch.drawings, watch.whole, at_once);
        }
    }

    /// The watched drawings of the sub-page `sub` of `page` that became
    /// whole since the last call, in ascending order (REQ-7026).
    pub fn take_painted(&mut self, page: u16, sub: u16) -> Vec<u16> {
        self.pages.get_mut(&(page, sub)).map(|held| core::mem::take(&mut held.painted)).unwrap_or_default()
    }

    /// Takes the "painted" event of the whole picture of the sub-page
    /// `sub` of `page` at presentation tick `tick`, if it is owed: its
    /// table names the page, bytes were drawn and the replay holds none
    /// (DEC-7031 item 5). At most once a tick.
    pub fn take_whole(&mut self, page: u16, sub: u16, tick: u32) -> bool {
        self.pages.get_mut(&(page, sub)).is_some_and(|held| held.whole.take(held.replay.idle(), tick))
    }

    /// Whether the sub-page `sub` of `page` has a "painted" event to give
    /// its rule table: a drawing's, or the whole picture's.
    #[must_use]
    pub fn painted_due(&self, page: u16, sub: u16) -> bool {
        self.pages.get(&(page, sub)).is_some_and(|held| !held.painted.is_empty() || held.whole.due(held.replay.idle()))
    }

    /// The trusted core said what the viewer's pointer is doing to a
    /// sub-page's picture (REQ-6374). A sub-page with no ReGIS has no
    /// picture for it to change.
    pub fn set_viewer(&mut self, page: u16, sub: u16, viewer: RegisViewer) {
        if let Some(held) = self.pages.get_mut(&(page, sub)) {
            held.viewer = viewer;
        }
    }

    /// The channel named the drawing that is a sub-page's cursor, or 0
    /// for none (the engine's REQ-6371). A sub-page with no ReGIS yet has
    /// nothing to blink; its channel names the cursor again with its ReGIS.
    pub fn set_cursor(&mut self, page: u16, sub: u16, drawing: u16) {
        if let Some(held) = self.pages.get_mut(&(page, sub)).filter(|held| held.cursor != drawing) {
            held.cursor = drawing;
            held.blink_again = true;
        }
    }

    /// Reads one `DRAWING_LINKS` record (the engine's REQ-6370). Part 0
    /// begins a sub-page's links again and later parts add to them, in
    /// order; when the last has come the links are kept for the trusted
    /// core. The engine's reader has checked each record; across the parts
    /// a drawing named twice, or a link past the most a sub-page has,
    /// drops the whole sending, as does a part out of its place.
    #[cfg(feature = "links")]
    pub fn read_links(&mut self, record: &neoteletext_web::records::drawing_links::DrawingLinksRecord<'_>) -> Option<&DrawingLinks> {
        let key = (record.page.mpp(), record.subcode.value());
        if record.part == 0 {
            self.link_parts = Some(LinkParts { key, next: 0, parts: record.parts, links: Vec::new() });
        }
        let mut parts = self.link_parts.take().filter(|parts| parts.key == key && parts.next == record.part && parts.parts == record.parts)?;
        for link in record.links() {
            let named_twice = parts.links.iter().any(|(drawing, _)| *drawing == link.drawing);
            // A target is printable ASCII, as the reader checked.
            let target = core::str::from_utf8(link.target).ok()?;
            if named_twice || parts.links.len() >= protocol::regis::MAX_REGIS_LINKS {
                return None;
            }
            parts.links.push((link.drawing, target.to_owned()));
        }
        parts.next = parts.next.saturating_add(1);
        if parts.next == parts.parts {
            // Only the latest of a sub-page's links is waiting to be sent.
            self.links.retain(|(page, sub, _)| (*page, *sub) != key);
            self.links.push((key.0, key.1, parts.links));
            self.links.last()
        } else {
            self.link_parts = Some(parts);
            None
        }
    }

    /// A sub-page's drawing links, kept with its picture's source, for the
    /// trusted core again as the picture is drawn again (PAR-34).
    #[cfg(feature = "links")]
    pub(crate) fn give_links(&mut self, links: DrawingLinks) {
        self.links.retain(|(page, sub, _)| (*page, *sub) != (links.0, links.1));
        self.links.push(links);
    }

    /// The drawing that is a sub-page's cursor, 0 for none.
    #[cfg(all(test, feature = "links"))]
    pub(crate) fn cursor(&self, page: u16, sub: u16) -> u16 {
        self.pages.get(&(page, sub)).map_or(0, |held| held.cursor)
    }

    /// The drawing links received whole since they were last taken.
    #[cfg(feature = "links")]
    pub fn take_links(&mut self) -> Vec<DrawingLinks> {
        core::mem::take(&mut self.links)
    }

    /// The generation of the ReGIS stream the sub-page `sub` of `page`
    /// holds and the offset its next byte has, or none when it holds no
    /// stream: where a page's script appends (the script API's
    /// specification section 13 item 3; TODO-7293).
    #[must_use]
    pub fn stream_end(&self, page: u16, sub: u16) -> Option<(u16, u32)> {
        self.pages.get(&(page, sub)).filter(|held| !held.lost).map(|held| (held.generation, held.next))
    }

    /// Whether the sub-page of `page` on screen (see [`Self::shown`]) is
    /// drawn in ReGIS.
    #[must_use]
    pub fn holds(&self, page: u16, sub: Option<u16>) -> bool {
        self.shown(page, sub).is_some_and(|key| self.pages.contains_key(&key))
    }

    /// Whether the channel has sent ReGIS for a page.
    #[must_use]
    pub fn has(&self, page: u16) -> bool {
        self.pages.keys().any(|(held, _)| *held == page)
    }

    /// Marks a page's pictures as not yet sent: the page came on screen.
    pub fn show_again(&mut self, page: u16) {
        for (_, held) in self.pages.iter_mut().filter(|((held, _), _)| *held == page) {
            held.changed = true;
            // The receiver may hold nothing of it: the next is whole.
            held.sent = None;
        }
        self.sent_sub.remove(&page);
    }

    /// The sub-page of `page` on screen: `sub`, or with none, or the
    /// sub-code that names none, the page's first with any ReGIS.
    fn shown(&self, page: u16, sub: Option<u16>) -> Option<(u16, u16)> {
        match sub {
            // The sub-page on screen, and no other: a viewer stepping
            // through a page's sub-pages must never be shown the picture
            // of one that is not on screen while this one's is on its way.
            Some(sub) if sub != neoteletext_core::limits::SUBCODE_ANY => Some((page, sub)),
            _ => self.pages.keys().find(|(held, _)| *held == page).copied(),
        }
    }

    /// The picture of a page's sub-page on screen (see [`Self::shown`]),
    /// if what it shows changed since it was last taken and its repaint
    /// budget allows a repaint at `now_us` of the decoder's clock. A
    /// repaint the budget holds back stays owed: [`Self::owed_us`] says
    /// when to ask again, and the picture then made is of the latest
    /// scene, however much changed meanwhile (REQ-6380).
    pub fn take(&mut self, page: u16, sub: Option<u16>, now_us: u64) -> Option<Drawn> {
        let key = self.shown(page, sub)?;
        let same_sub = self.sent_sub.get(&key.0) == Some(&key.1);
        let held = self.pages.get_mut(&key)?;
        held.apply_viewer(now_us);
        if !held.changed {
            return None;
        }
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("decoder", "compose-begin", std::time::Instant::now(), format_args!(""));
        held.earn(now_us);
        if !held.budget.may_repaint() {
            // RULE-66's hops: a repaint the budget held back.
            #[cfg(feature = "timed-test")]
            protocol::timed_test::mark("decoder", "regis-held", std::time::Instant::now(), format_args!("{}", held.budget.milliseconds_until_repaint()));
            // Over the budget the animation slows and nothing else does
            // (ANIM-16): the state has changed already, the painting
            // waits, and the damage of every change meanwhile is painted
            // by one repaint when the budget allows.
            if core::mem::take(&mut held.motion.animated) {
                held.counters.slowed = held.counters.slowed.saturating_add(1);
            }
            return None;
        }
        held.changed = false;
        held.motion.animated = false;
        held.terminal.compose(&held.screen, &mut held.picture);
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("decoder", "composed", std::time::Instant::now(), format_args!("{}", held.picture.work()));
        held.budget.spend(held.picture.work());
        let pixels = held.picture.canvas().pixels();
        let palette = held.terminal.palette().colours();
        // The rows whose pixels this compose changed since the one before,
        // which was sent: only they are compared and kept.
        let rows = match held.picture.changed_rows() {
            None => Changed::Nothing,
            Some((first, last)) => Changed::Rows(usize::try_from(first).unwrap_or(usize::MAX), usize::try_from(last).unwrap_or(usize::MAX)),
        };
        let pick = Picked::pick(&mut held.picked, held.picture.pick_record(), held.picture.origins().collect());
        // What the sub-page's drawing has cost: on the base since its
        // stream began, and to paint its numbered drawings this time.
        let work = held.terminal.base_work().saturating_add(held.picture.work());
        let figures = Figures { commits: held.picture.commits(), work, language: PictureLanguage::Regis };
        let (mut parts, pick_changed) = parts_to_send(&mut held.sent, same_sub, pixels, palette, &pick, rows, figures)?;
        self.sent_sub.insert(key.0, key.1);
        let more = parts.split_off(1);
        Some(Drawn { page: key.0, sub: key.1, picture: parts.pop()?, pick: pick_changed.then_some(pick), more })
    }

    /// Carries out one animation operation on the sub-page of `page` on
    /// screen (see [`Self::shown`]) at presentation tick `tick`
    /// (animation.md ANIM-6); false when it did nothing.
    pub fn animate(&mut self, page: u16, sub: Option<u16>, operation: &Operation, tick: u32) -> bool {
        self.shown(page, sub).and_then(|key| self.pages.get_mut(&key)).is_some_and(|held| held.animate(operation, tick))
    }

    /// Brings the sub-page of `page` on screen to presentation tick
    /// `tick`: its cycles' frames and its drawings' painting (ANIM-14,
    /// ANIM-15).
    pub fn advance(&mut self, page: u16, sub: Option<u16>, tick: u32) {
        let at_once = self.at_once || self.reduced;
        if let Some(held) = self.shown(page, sub).and_then(|key| self.pages.get_mut(&key)) {
            held.advance(tick, at_once);
        }
    }

    /// The next presentation tick after `tick` the sub-page of `page` on
    /// screen needs the loop at for its animation, if any (ANIM-1).
    #[must_use]
    pub fn next_tick(&self, page: u16, sub: Option<u16>, tick: u32) -> Option<u32> {
        self.shown(page, sub).and_then(|key| self.pages.get(&key))?.next_tick(tick)
    }

    /// The state of a numbered drawing of the sub-page of `page` on
    /// screen, as a rule table's conditions read it (RULE-26, ANIM-13).
    #[must_use]
    pub fn target_state(&self, page: u16, sub: Option<u16>, number: u16) -> Option<TargetState> {
        self.shown(page, sub).and_then(|key| self.pages.get(&key))?.target_state(number)
    }

    /// The numbered drawing the pick record of the sub-page of `page` on
    /// screen names at a pixel of its picture as it was last composed, the
    /// picture the viewer points at; 0 for none (page-rules.md RULE-12;
    /// the renderer's `docs/regis-v2.md`, "Picking").
    #[must_use]
    pub fn drawing_at(&self, page: u16, sub: Option<u16>, x: u16, y: u16) -> u16 {
        self.shown(page, sub).and_then(|key| self.pages.get(&key)).map_or(0, |held| held.picture.pick(u32::from(x), u32::from(y)).drawing)
    }

    /// The numbered drawing that drawing `number` of the sub-page of
    /// `page` on screen is a child of, as the stream committed it; 0 for
    /// a top-level drawing and for one that does not exist (RULE-12: a
    /// pointer on a drawing is inside each of its ancestors).
    #[must_use]
    pub fn parent(&self, page: u16, sub: Option<u16>, number: u16) -> u16 {
        self.shown(page, sub).and_then(|key| self.pages.get(&key)).and_then(|held| held.terminal.parent(number)).unwrap_or(0)
    }

    /// What the animation of the sub-page of `page` on screen has cost
    /// since it was last asked, for the rule table's host to report
    /// (ANIM-16, RULE-48).
    pub fn take_counters(&mut self, page: u16, sub: Option<u16>) -> Counters {
        self.shown(page, sub).and_then(|key| self.pages.get_mut(&key)).map(|held| core::mem::take(&mut held.counters)).unwrap_or_default()
    }

    /// The font the viewer chose for alphabet 0, or `None` for the one
    /// compiled in (renderer parity PAR-35): every terminal begun after
    /// this draws its text in it.
    pub fn set_font(&mut self, font: Option<Font>) {
        self.font = font;
    }

    /// The platform's preference for reduced motion (REQ-754, renderer
    /// parity PAR-33): on, the paint rate draws at once (ANIM-15), a frame
    /// cycle shows its first frame and does not advance (ANIM-14), and the
    /// cursor drawing shows steadily (REQ-6378); the channel's commits and
    /// the rules' state changes still apply.
    pub fn set_reduced(&mut self, reduced: bool) {
        self.reduced = reduced;
        for held in self.pages.values_mut() {
            held.motion.steady = reduced;
            if reduced {
                held.whole_at_once();
            }
            held.changed = true;
        }
    }

    /// The viewer's setting "Paint drawings gradually" (ANIM-15): off,
    /// every drawing of every sub-page is whole at once, now and at each
    /// commit, and no stream is fed at a line's speed: what a replay held
    /// is drawn at once (DEC-7031 item 5).
    pub fn set_gradual(&mut self, gradual: bool) {
        self.at_once = !gradual;
        if self.at_once {
            for held in self.pages.values_mut() {
                held.whole_at_once();
            }
        }
    }

    /// Reads one SHEET record of ReGIS (ANIM-17): the bytes are kept, in
    /// order, as the sub-page's sheet (RULE-112: a sheet is one page and
    /// sub-code's). A new generation starts the sheet again, and the
    /// sub-page's renderer with it: it is forgotten, and its channel sends
    /// its stream again after the sheet (a renderer cannot be started
    /// again from bytes the decoder did not keep). A record out of its
    /// place loses the generation.
    pub fn read_sheet(&mut self, page: u16, sub: u16, generation: u16, offset: u32, data: &[u8]) {
        let key = (page, sub);
        let fresh = self.sheets.get(&key).is_none_or(|sheet| sheet.generation != generation);
        if fresh {
            if !self.sheets.contains_key(&key) && self.sheets.len() >= MAX_SHEETS {
                // The sheet of a sub-page that is not kept goes first; else
                // the lowest one's.
                let unused = self.sheets.keys().copied().find(|held| !self.pages.contains_key(held)).or_else(|| self.sheets.keys().next().copied());
                if let Some(unused) = unused {
                    self.sheets.remove(&unused);
                }
            }
            let renewed = self.sheets.contains_key(&key);
            self.sheets.insert(key, Sheet { generation, bytes: Vec::new(), lost: offset != 0 });
            if renewed {
                self.pages.remove(&key);
                if self.sent_sub.get(&page) == Some(&sub) {
                    self.sent_sub.remove(&page);
                }
            }
        }
        let Some(sheet) = self.sheets.get_mut(&key) else { return };
        let end = usize::try_from(offset).ok().and_then(|offset| offset.checked_add(data.len()));
        if sheet.lost || usize::try_from(offset).ok() != Some(sheet.bytes.len()) || end.is_none_or(|end| end > neoteletext_web::consts::MAX_SHEET_BYTES) {
            sheet.lost = true;
            return;
        }
        sheet.bytes.extend_from_slice(data);
    }

    /// Forgets everything held of one sub-page: its renderer, its sheet and
    /// its animation state (page-rules.md RULE-111: a page closed drops its
    /// local state; its channel sends it afresh on the next selection).
    pub fn forget(&mut self, page: u16, sub: u16) {
        self.pages.remove(&(page, sub));
        self.sheets.remove(&(page, sub));
        self.pending.remove(&(page, sub));
        if self.sent_sub.get(&page) == Some(&sub) {
            self.sent_sub.remove(&page);
        }
    }

    /// When a page's sub-page on screen next needs the decoder's loop, in
    /// microseconds of its clock: when the repaint it is owed may be made,
    /// or when its cursor next shows or hides; `None` when neither. The
    /// loop waits on the earliest of these.
    pub fn owed_us(&mut self, page: u16, sub: Option<u16>, now_us: u64) -> Option<u64> {
        let key = self.shown(page, sub)?;
        let held = self.pages.get_mut(&key)?;
        held.apply_viewer(now_us);
        let blink = held.next_blink_us(now_us);
        if !held.changed {
            return blink;
        }
        held.earn(now_us);
        let wait = held.budget.milliseconds_until_repaint().saturating_mul(MICROSECONDS_PER_MILLISECOND);
        Some(blink.map_or(now_us.saturating_add(wait), |blink| blink.min(now_us.saturating_add(wait))))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes data it built to a known size")]

    use neoteletext_core::types::{PageAddress, SubCode};
    use protocol::regis::{REGIS_HEIGHT, REGIS_WIDTH, RegisArea, unpack};

    use crate::parts::changed_areas;

    use super::*;

    fn record(page: u16, generation: u16, offset: u32, data: &[u8]) -> RegisRecord<'_> {
        RegisRecord { page: PageAddress::from_mpp(page).unwrap(), subcode: SubCode::new(0), generation, offset, data }
    }

    /// DEC-6335: a sub-page's records, in order, draw its picture; a
    /// numbered drawing is in the picture and its pick record.
    #[test]
    fn records_in_order_draw_a_picture_with_its_pick_record() {
        let mut pages = RegisPages::default();
        let first = b"P[10,10]V[30,10]D(B1)W(I3)V[0,0][19,0]D(E)";
        pages.read(&record(0x100, 1, 0, first));
        pages.read(&record(0x100, 1, u32::try_from(first.len()).unwrap(), b"D(S1(X100,Y50,I1),C)"));
        let drawn = pages.take(0x100, Some(0), 0).unwrap();
        let pixels = unpack(&drawn.picture.packed);
        assert_eq!(pixels[10 * REGIS_WIDTH + 20], 7, "the base");
        assert!(drawn.picture.work >= 21 + 20, "the base's line and the drawing's are both counted");
        assert_eq!(pixels[50 * REGIS_WIDTH + 110], 3, "the numbered drawing");
        assert_eq!(drawn.picture.commits, 1);
        assert!(drawn.picture.work > 0);
        let picked = drawn.pick.unwrap().at(PictureLanguage::Regis, 110, 50);
        assert_eq!((picked.drawing, picked.x, picked.y), (1, 10, 0));
        assert!(pages.take(0x100, Some(0), 0).is_none(), "nothing changed since");
        pages.show_again(0x100);
        assert!(pages.take(0x100, None, 0).is_some(), "sent again when the page comes on screen");
    }

    /// Renderer parity PAR-36 item 4 (PAR-12): every ReGIS terminal has the
    /// engine's 55 provided sets, so alphabet 117, set 17 (Greek), draws the
    /// engine's Noto Teletext capital delta for its code 4/4, exactly as a
    /// terminal given that glyph alone draws it, and not the built-in D.
    #[test]
    fn alphabet_117_draws_the_engines_greek_glyphs() {
        // In a numbered drawing, so that the picture composed holds it.
        let text = |alphabet: u8| format!("D(B1)W(I7)T(A{alphabet})'D'D(E)D(S1(X100,Y100),C)");
        let drawn = |bytes: &str| {
            let mut pages = RegisPages::default();
            pages.read(&record(0x100, 1, 0, bytes.as_bytes()));
            let _ = pages.take(0x100, Some(0), 0).unwrap();
            pages.pages.get(&(0x100, 0)).unwrap().picture.canvas().pixels().to_vec()
        };
        let greek = drawn(&text(117));
        let delta = *crate::engine::GLYPHS.lookup('\u{0394}').unwrap();
        let mut set = regis::ProvidedSet::new(12, 20).unwrap();
        set.set_glyph(0x44, &delta).unwrap();
        let mut terminal = Terminal::new();
        terminal.set_provided_set(17, set).unwrap();
        let mut screen = Screen::new();
        let _ = terminal.feed_regis(&mut screen, text(117).as_bytes());
        let mut picture = Picture::new();
        terminal.compose(&screen, &mut picture);
        assert!(picture.canvas().pixels().iter().any(|&pixel| pixel != 0), "the delta draws");
        assert!(greek == picture.canvas().pixels(), "the engine's Greek glyph, as the renderer draws it");
        assert!(greek != drawn(&text(0)), "not the built-in alphabet's D");
    }

    /// Renderer parity PAR-35: a font the viewer chose, which parsed, is
    /// alphabet 0 of every terminal begun after it, so its glyphs are drawn
    /// in place of those compiled in; going back to the font compiled in
    /// draws those again.
    #[test]
    fn a_chosen_font_changes_the_glyphs_drawn() {
        let text = b"D(B1)W(I7)P[100,100]T'A'D(E)D(S1(X100,Y100),C)";
        let drawn = |font: Option<Font>| {
            let mut pages = RegisPages::default();
            pages.set_font(font);
            pages.read(&record(0x100, 1, 0, text));
            let _ = pages.take(0x100, Some(0), 0).unwrap();
            pages.pages.get(&(0x100, 0)).unwrap().picture.canvas().pixels().to_vec()
        };
        let compiled_in = drawn(None);
        let chosen = Font::parse(crate::fonts::BLOCK_A).unwrap();
        let block = drawn(Some(chosen));
        assert!(block != compiled_in, "the chosen font's A is drawn");
        let lit = |pixels: &[u8]| pixels.iter().filter(|&&pixel| pixel != 0).count();
        assert!(lit(&block) > lit(&compiled_in), "a solid block lights more than the VT340's A");
        assert!(drawn(None) == compiled_in, "back to the font compiled in");
    }

    /// A record lost shows nothing more of its generation; the next
    /// generation starts the sub-page again.
    #[test]
    fn a_lost_record_stops_a_generation_and_the_next_starts_again() {
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, b"P[10,10]"));
        let _ = pages.take(0x100, None, 0);
        pages.read(&record(0x100, 1, 99, b"V[30,10]"));
        assert!(pages.take(0x100, None, 0).is_none(), "the record after a gap is not drawn");
        pages.read(&record(0x100, 2, 5, b"V[30,10]"));
        assert!(pages.take(0x100, None, 0).is_none(), "a generation joined part way through is not drawn");
        pages.read(&record(0x100, 3, 0, b"P[10,10]V[30,10]"));
        let drawn = pages.take(0x100, None, 0).unwrap();
        assert_eq!(unpack(&drawn.picture.packed)[10 * REGIS_WIDTH + 20], 7);
    }

    /// A page with several sub-pages drawn in ReGIS shows the one on
    /// screen and no other, even while that one's ReGIS has not come.
    #[test]
    fn only_the_sub_page_on_screen_is_taken() {
        let sub = |code: u16, data: &'static [u8]| RegisRecord {
            page: PageAddress::from_mpp(0x101).unwrap(),
            subcode: SubCode::new(code),
            generation: 1,
            offset: 0,
            data,
        };
        let mut pages = RegisPages::default();
        pages.read(&sub(1, b"P[10,10]V[30,10]"));
        assert!(pages.take(0x101, Some(2), 0).is_none(), "sub-page 2 is on screen and has no ReGIS yet");
        pages.read(&sub(2, b"P[10,20]V[30,20]"));
        let drawn = pages.take(0x101, Some(2), 0).unwrap();
        assert_eq!(drawn.sub, 2);
        assert_eq!(unpack(&drawn.picture.packed)[20 * REGIS_WIDTH + 20], 7);
        assert_eq!(pages.take(0x101, Some(1), 0).unwrap().sub, 1);
    }

    /// The sub-pages kept are bounded: one more forgets the one fed longest ago.
    #[test]
    fn the_sub_pages_kept_are_bounded() {
        let mut pages = RegisPages::default();
        for page in 0..=u16::try_from(MAX_REGIS_PAGES).unwrap() {
            pages.read(&record(0x100 + page, 1, 0, b"V[5,5]"));
        }
        assert!(!pages.has(0x100));
        assert!(pages.has(0x101) && pages.has(0x100 + u16::try_from(MAX_REGIS_PAGES).unwrap()));
    }

    /// REQ-6380: a stream of minimal commits to a picture that is dear to
    /// repaint, each followed by a wish to draw it, buys no more repaint
    /// work than the budget allows for the time passed; the repaint held
    /// back is made when the budget allows, and shows the last scene.
    #[test]
    fn repaints_are_held_to_the_budget_and_the_last_scene_is_shown() {
        use regis::limits::{MAX_REBUILD_WORK, REPAINT_WORK_HELD, REPAINT_WORK_PER_SECOND};

        /// The commits sent, a millisecond of the decoder's clock apart.
        const COMMITS: u32 = 200;
        // A filled screen used 100 times: about 77 million work units a repaint.
        let uses = (2..102).map(|use_of| format!("D(S{use_of}(R1))")).collect::<Vec<_>>().concat();
        let scene = format!("D(B1)F(P[0,0]V[799,0][799,479][0,479])D(E)D(S1(V0)){uses}D(C)");
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        let mut offset = u32::try_from(scene.len()).unwrap();
        let (mut work, mut frames, mut last) = (0_u64, 0_u32, None);
        for commit in 0..COMMITS {
            let moved = format!("D(S2(X{commit}),C)");
            pages.read(&record(0x100, 1, offset, moved.as_bytes()));
            offset += u32::try_from(moved.len()).unwrap();
            let now_us = u64::from(commit) * MICROSECONDS_PER_MILLISECOND;
            if let Some(drawn) = pages.take(0x100, Some(0), now_us) {
                // The work reported includes the base's, which is none here.
                work += drawn.picture.work;
                frames += 1;
                last = Some(drawn.picture.commits);
            } else {
                let owed = pages.owed_us(0x100, Some(0), now_us).unwrap();
                assert!(owed > now_us, "a repaint held back is owed at a later time");
            }
        }
        let seconds_thousandths = u64::from(COMMITS);
        let bound = REPAINT_WORK_HELD + REPAINT_WORK_PER_SECOND * seconds_thousandths / 1000 + MAX_REBUILD_WORK;
        assert!(work <= bound, "{frames} repaints cost {work}, over the bound of {bound}");
        assert!((2..COMMITS / 10).contains(&frames), "some repaints are made and most are held back: {frames}");
        assert_ne!(last, Some(COMMITS + 1), "the last commit's repaint was held back");
        // The repaint owed is made at the time given, and is of the last scene.
        let now_us = u64::from(COMMITS) * MICROSECONDS_PER_MILLISECOND;
        let owed = pages.owed_us(0x100, Some(0), now_us).unwrap();
        assert!(pages.take(0x100, Some(0), owed - MICROSECONDS_PER_MILLISECOND).is_none(), "not a millisecond sooner");
        let drawn = pages.take(0x100, Some(0), owed).unwrap();
        assert_eq!(drawn.picture.commits, COMMITS + 1);
        assert_eq!(pages.owed_us(0x100, Some(0), owed), None, "nothing is owed once it is made");
    }

    /// With the budget in hand, each commit's picture is made at once.
    #[test]
    fn a_cheap_picture_is_repainted_at_every_commit() {
        let mut pages = RegisPages::default();
        let scene = "D(B1)W(I3)V[0,0][19,0]D(E)D(S1(X100,Y50),C)";
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        let mut offset = u32::try_from(scene.len()).unwrap();
        for commit in 0..50_u32 {
            let drawn = pages.take(0x100, Some(0), 0).unwrap();
            assert_eq!(drawn.picture.commits, commit + 1);
            let moved = format!("D(S1(X{commit}),C)");
            pages.read(&record(0x100, 1, offset, moved.as_bytes()));
            offset += u32::try_from(moved.len()).unwrap();
        }
    }

    /// REQ-6374: the hovered drawing is painted with its hover look and the
    /// pressed one with its pressed look, with no bytes from the channel;
    /// a drawing with no look, and a viewer state that changes nothing,
    /// cost no repaint.
    #[test]
    fn the_viewer_state_paints_looks_and_costs_nothing_where_there_are_none() {
        let square = |colour: u8| format!("W(I{colour})V[0,0][19,0][19,19][0,19][0,0]");
        let scene = format!(
            "D(B1){}D(E)D(B2){}D(E)D(B3){}D(E)D(B4){}D(E)D(S1(X100,Y50,I1,H2,A3),S2(V0),S3(V0),S4(X300,Y50,I1),C)",
            square(3),
            square(5),
            square(6),
            square(7)
        );
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        let at = |pages: &mut RegisPages| {
            pages.take(0x100, Some(0), 0).map(|_| pages.pages.get(&(0x100, 0)).unwrap().picture.canvas().pixels()[50 * REGIS_WIDTH + 100])
        };
        assert_eq!(at(&mut pages), Some(3));
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 1, pressed: 0, typing: true });
        assert_eq!(at(&mut pages), Some(5), "the hover look");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 1, pressed: 1, typing: true });
        assert_eq!(at(&mut pages), Some(6), "the pressed look wins");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 1, pressed: 1, typing: true });
        assert_eq!(at(&mut pages), None, "the same state again repaints nothing");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 4, pressed: 0, typing: true });
        assert_eq!(at(&mut pages), Some(3), "drawing 1 is itself again");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 0, pressed: 4, typing: true });
        assert_eq!(at(&mut pages), None, "drawing 4 has no looks: nothing to repaint");
        // Another sub-page's viewer state is not this one's.
        pages.set_viewer(0x100, 9, RegisViewer { hovered: 1, pressed: 0, typing: true });
        assert_eq!(at(&mut pages), None);
    }

    /// REQ-6378: while keys go to the channel the cursor drawing is shown
    /// and hidden in turn, each for the half period of the typing cursor's
    /// blink; a commit, as a key typed sends, shows it again for a full
    /// half period; when keys stop it is left as the channel drew it.
    #[test]
    fn the_cursor_drawing_blinks_while_keys_go_to_the_channel() {
        /// The half period, on the decoder's clock.
        const HALF: u64 = Page::BLINK_US;
        let scene = "D(B1)W(I7)T'Hi'D(E)D(B2)W(I3)V[0,0][8,0]D(E)D(S1(X60,Y200),S2(X78,Y200),C)";
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        // The pixel of the cursor's row at a column, in the picture taken at a time, if one was made.
        let cursor_pixel = |pages: &mut RegisPages, now: u64, column: usize| {
            pages.take(0x100, Some(0), now).map(|_| pages.pages.get(&(0x100, 0)).unwrap().picture.canvas().pixels()[200 * REGIS_WIDTH + column])
        };
        // The cursor's pixel in the picture taken at a time, if one was made.
        let cursor_at = |pages: &mut RegisPages, now: u64| {
            pages.take(0x100, Some(0), now).map(|_| pages.pages.get(&(0x100, 0)).unwrap().picture.canvas().pixels()[200 * REGIS_WIDTH + 80])
        };
        assert_eq!(cursor_at(&mut pages, 0), Some(3));
        pages.set_cursor(0x100, 0, 2);
        assert_eq!(cursor_at(&mut pages, 0), None, "keys do not go to the channel: it does not blink");
        assert_eq!(pages.owed_us(0x100, Some(0), 0), None);
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 0, pressed: 0, typing: true });
        // Keys begin going to the channel a little after tick 3: the blink
        // begins on that tick (ANIM-1).
        let start = crate::animation::due_at(3);
        assert_eq!(cursor_at(&mut pages, start + 1000), None, "shown for the first half period, as it already is");
        assert_eq!(pages.owed_us(0x100, Some(0), start + 2000), Some(start + HALF), "the loop wakes when it is to hide, 32 ticks on");
        assert_eq!(cursor_at(&mut pages, start + HALF), Some(0), "hidden for the second");
        assert_eq!(cursor_at(&mut pages, start + 2 * HALF), Some(3), "and shown again");
        assert_eq!(cursor_at(&mut pages, start + 3 * HALF), Some(0));
        // A key typed: the channel moves the cursor by a commit, and it is
        // shown at once at its new place, for a full half period from the
        // tick the commit is drawn in.
        let moved = "D(S2(X87),C)";
        pages.read(&record(0x100, 1, u32::try_from(scene.len()).unwrap(), moved.as_bytes()));
        let typed_at = start + 3 * HALF + 5000;
        let shown_from = crate::animation::due_at(crate::animation::tick_at(typed_at));
        assert!(shown_from <= typed_at && typed_at - shown_from < crate::animation::due_at(1));
        assert_eq!(cursor_pixel(&mut pages, typed_at, 89), Some(3));
        assert_eq!(pages.owed_us(0x100, Some(0), typed_at), Some(shown_from + HALF));
        assert_eq!(cursor_at(&mut pages, shown_from + HALF - 1), None, "still shown just before the half period ends");
        // Keys stop going to the channel while it is hidden: it comes back and stays.
        assert!(cursor_at(&mut pages, shown_from + HALF).is_some());
        pages.set_viewer(0x100, 0, RegisViewer::default());
        assert_eq!(cursor_pixel(&mut pages, shown_from + HALF + 1, 89), Some(3));
        assert_eq!(pages.owed_us(0x100, Some(0), typed_at + 5 * HALF), None);
        // A cursor drawing that does not exist blinks nothing and costs no repaint.
        pages.set_cursor(0x100, 0, 99);
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 0, pressed: 0, typing: true });
        assert_eq!(cursor_at(&mut pages, typed_at + 9 * HALF), None);
        assert_eq!(cursor_at(&mut pages, typed_at + 10 * HALF), None);
    }

    /// REQ-6376: a sub-page's drawing links, in one record or several in
    /// order, are kept whole for the trusted core; a part out of its
    /// place, or a drawing named in two parts, drops the sending.
    #[cfg(feature = "links")]
    #[test]
    fn drawing_links_are_gathered_from_their_parts() {
        use neoteletext_web::records::drawing_links::{DrawingLink, encode};
        use neoteletext_web::records::{ChannelRecord, Read, RecordReader};

        // Sends `links` as records and reads each into `pages`, leaving
        // out the parts `skip` names.
        let send = |pages: &mut RegisPages, links: &[DrawingLink<'_>], skip: Option<u8>| {
            let mut reader = RecordReader::new();
            encode(PageAddress::from_mpp(0x105).unwrap(), SubCode::new(0), links, 3, 0, &mut |packet| {
                if let Read::Record(ChannelRecord::DrawingLinks(record)) = reader.push(&packet)
                    && skip != Some(record.part)
                {
                    pages.read_links(&record);
                }
            })
            .unwrap();
        };
        let mut pages = RegisPages::default();
        send(&mut pages, &[DrawingLink { drawing: 11, target: b"100" }, DrawingLink { drawing: 13, target: b"web:https://example.invalid/" }], None);
        assert_eq!(pages.take_links(), [(0x105, 0, vec![(11, "100".to_owned()), (13, "web:https://example.invalid/".to_owned())])]);
        assert_eq!(pages.take_links(), []);
        // Three long links take two records: both parts, then the first lost.
        let long = format!("web:https://example.invalid/{}", "a".repeat(3000));
        let three: Vec<DrawingLink<'_>> = (1..=3).map(|drawing| DrawingLink { drawing, target: long.as_bytes() }).collect();
        send(&mut pages, &three, None);
        assert_eq!(pages.take_links().first().map(|(_, _, links)| links.len()), Some(3));
        send(&mut pages, &three, Some(0));
        assert_eq!(pages.take_links(), [], "the first part was lost: the rest is not a sub-page's links");
        // No links at all are a sub-page's links: none.
        send(&mut pages, &[], None);
        assert_eq!(pages.take_links(), [(0x105, 0, Vec::new())]);
    }

    /// A receiver's copy of a picture: whole pictures replace it and parts
    /// are laid over it, as the interface does.
    #[derive(Default)]
    struct Receiver {
        pixels: Vec<u8>,
        number: u32,
    }

    impl Receiver {
        #[allow(clippy::arithmetic_side_effects, reason = "a test's own places in a picture, far inside usize")]
        fn take(&mut self, picture: &RegisPicture) {
            let area = picture.area;
            let part = unpack(&picture.packed);
            if picture.whole() {
                assert_eq!(area, RegisArea::whole(PictureLanguage::Regis));
                self.pixels = part;
            } else {
                assert_eq!(picture.base, self.number, "a part of the picture held");
                let (left, across) = (usize::from(area.x), usize::from(area.width));
                for (row, line) in part.chunks_exact(across.max(1)).enumerate().take(usize::from(area.height)) {
                    let start = (usize::from(area.y) + row) * REGIS_WIDTH + left;
                    self.pixels[start..start + across].copy_from_slice(line);
                }
            }
            self.number = picture.number;
        }
    }

    /// TODO-7118 (the renderer's `docs/regis-v2.md`, "Rules for contents",
    /// rule 4): a colour map sent alone, with no batch of D operations
    /// open, takes effect at once. It writes no pixel and makes no commit,
    /// yet the sub-page owes a picture: a part of no pixels that carries
    /// the new colours.
    #[test]
    fn a_colour_map_sent_alone_is_shown_at_once() {
        let mut pages = RegisPages::default();
        let scene = b"P[10,10]V[30,10]D(B1)W(I1)V[0,0][19,0]D(E)D(S1(X100,Y50),C)";
        pages.read(&record(0x100, 1, 0, scene));
        let first = pages.take(0x100, Some(0), 0).unwrap();
        assert!(first.picture.whole());
        let before = first.picture.palette[1];
        assert!(pages.take(0x100, Some(0), 0).is_none(), "nothing changed since");
        let map = b"S(M1(R))";
        pages.read(&record(0x100, 1, u32::try_from(scene.len()).unwrap(), map));
        let recoloured = pages.take(0x100, Some(0), 0).expect("a map sent alone owes the sub-page a picture");
        let [red, green, blue] = recoloured.picture.palette[1];
        assert!(red > 0 && green == 0 && blue == 0, "location 1 is red: {:?}", recoloured.picture.palette[1]);
        assert_ne!(recoloured.picture.palette[1], before, "the map changed");
        assert!(
            !recoloured.picture.whole() && recoloured.picture.area == RegisArea::default() && recoloured.more.is_empty(),
            "no pixel changed: a part of no pixels"
        );
        assert_eq!(recoloured.picture.commits, first.picture.commits, "no commit was made");
        assert!(pages.take(0x100, Some(0), 0).is_none(), "nothing changed since the map");
    }

    /// TODO-7118: a colour map sent inside an open batch is held with the
    /// batch, and shows with its commit, not before.
    #[test]
    fn a_colour_map_inside_an_open_batch_shows_with_its_commit() {
        let mut pages = RegisPages::default();
        let scene = b"P[10,10]V[30,10]D(B1)W(I1)V[0,0][19,0]D(E)D(S1(X100,Y50),C)";
        pages.read(&record(0x100, 1, 0, scene));
        let before = pages.take(0x100, Some(0), 0).unwrap().picture.palette[1];
        let mut offset = u32::try_from(scene.len()).unwrap();
        for bytes in [&b"D(S1(X200))"[..], b"S(M1(R))"] {
            pages.read(&record(0x100, 1, offset, bytes));
            offset += u32::try_from(bytes.len()).unwrap();
            let held = pages.take(0x100, Some(0), 0);
            assert!(held.is_none_or(|drawn| drawn.picture.palette[1] == before), "the batch is open: the old colour stands");
        }
        pages.read(&record(0x100, 1, offset, b"D(C)"));
        let committed = pages.take(0x100, Some(0), 0).expect("the commit shows the batch");
        let [red, green, blue] = committed.picture.palette[1];
        assert!(red > 0 && green == 0 && blue == 0, "location 1 is red with the commit: {:?}", committed.picture.palette[1]);
    }

    /// REQ-6399 items 1, 4 and 5: after the first whole picture, each
    /// change is sent as the rectangle that changed, far smaller than the
    /// picture; laid over one another in turn the parts give exactly the
    /// decoder's picture; and the pick record goes only when it changed.
    #[test]
    fn a_change_is_sent_as_the_part_that_changed_and_the_parts_rebuild_the_picture() {
        let mut pages = RegisPages::default();
        let scene = b"P[10,10]V[700,400]D(B1)W(I3)F(V[0,0][19,0][19,19][0,19])D(E)D(S1(X100,Y50,I1))D(B2)W(I5)V[0,0][30,0]D(E)D(S2(X400,Y300),C)";
        pages.read(&record(0x100, 1, 0, scene));
        let mut offset = u32::try_from(scene.len()).unwrap();
        let mut receiver = Receiver::default();
        let first = pages.take(0x100, Some(0), 0).unwrap();
        assert!(first.picture.whole() && first.pick.is_some(), "the first is whole, with its pick record");
        receiver.take(&first.picture);
        // A small generator: the same moves every run.
        let mut seed = 12_345_u32;
        let mut next = |limit: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) % limit
        };
        for step in 0..60 {
            // The detectable square or the plain line moves.
            let detectable = step % 3 != 0;
            let command = format!("D(S{}(X{},Y{}),C)", if detectable { 1 } else { 2 }, next(760), next(450));
            pages.read(&record(0x100, 1, offset, command.as_bytes()));
            offset += u32::try_from(command.len()).unwrap();
            let drawn = pages.take(0x100, Some(0), 0).unwrap();
            assert!(!drawn.picture.whole(), "step {step}: a part");
            // Where the drawing was and where it is, each no larger than the drawing and a margin.
            let sent: usize = core::iter::once(&drawn.picture).chain(&drawn.more).map(|part| part.area.pixels()).sum();
            assert!(sent <= 2 * 40 * 40, "step {step}: {sent} pixels sent for a move of a drawing of at most 31 by 20");
            assert_eq!(drawn.pick.is_some(), detectable, "step {step}: the pick record goes when a detectable drawing moved, and only then");
            receiver.take(&drawn.picture);
            for part in &drawn.more {
                receiver.take(part);
            }
            let held = pages.pages.get(&(0x100, 0)).unwrap().picture.canvas().pixels();
            assert!(receiver.pixels == held, "step {step}: the parts give the decoder's picture");
        }
    }

    /// REQ-6399 item 1: the next picture is whole when the page comes on
    /// screen again, when the sub-page shown is another than the one last
    /// sent, and when the colours changed.
    #[test]
    fn the_picture_is_sent_whole_when_the_receiver_may_not_hold_its_base() {
        let sub = |code: u16, data: &'static [u8]| RegisRecord {
            page: PageAddress::from_mpp(0x101).unwrap(),
            subcode: SubCode::new(code),
            generation: 1,
            offset: 0,
            data,
        };
        let mut pages = RegisPages::default();
        pages.read(&sub(1, b"P[10,10]V[30,10]"));
        pages.read(&sub(2, b"P[10,40]V[30,40]"));
        assert!(pages.take(0x101, Some(1), 0).unwrap().picture.whole());
        assert!(pages.take(0x101, Some(2), 0).unwrap().picture.whole(), "another sub-page of the page");
        pages.pages.get_mut(&(0x101, 1)).unwrap().changed = true;
        assert!(pages.take(0x101, Some(1), 0).unwrap().picture.whole(), "back to the first: the receiver holds the second's");
        pages.pages.get_mut(&(0x101, 1)).unwrap().changed = true;
        let part = pages.take(0x101, Some(1), 0).unwrap();
        assert!(
            !part.picture.whole() && part.picture.area == RegisArea::default() && part.pick.is_none() && part.more.is_empty(),
            "nothing changed: a part of no pixels"
        );
        pages.show_again(0x101);
        assert!(pages.take(0x101, Some(1), 0).unwrap().picture.whole(), "the page came on screen again");
    }

    /// The areas that changed hold every differing pixel on even columns;
    /// changes far apart down the picture are separate areas, and too many
    /// to send apart are one.
    #[test]
    fn the_changed_areas_hold_every_differing_pixel_and_lie_apart() {
        let old = vec![0_u8; REGIS_WIDTH * REGIS_HEIGHT];
        assert_eq!(changed_areas(&old, &old, REGIS_WIDTH), []);
        let mut new = old.clone();
        new[7 * REGIS_WIDTH + 13] = 1;
        new[9 * REGIS_WIDTH + 20] = 1;
        assert_eq!(changed_areas(&old, &new, REGIS_WIDTH), [RegisArea { x: 12, y: 7, width: 10, height: 3 }], "rows near each other are one area");
        new[400 * REGIS_WIDTH + 799] = 2;
        assert_eq!(
            changed_areas(&old, &new, REGIS_WIDTH),
            [RegisArea { x: 12, y: 7, width: 10, height: 3 }, RegisArea { x: 798, y: 400, width: 2, height: 1 }]
        );
        // A changed pixel every twenty rows: more areas than are sent apart.
        let mut striped = old.clone();
        for row in (0..REGIS_HEIGHT).step_by(20) {
            striped[row * REGIS_WIDTH + 100] = 3;
        }
        assert_eq!(changed_areas(&old, &striped, REGIS_WIDTH), [RegisArea { x: 100, y: 0, width: 2, height: 461 }]);
    }
}
