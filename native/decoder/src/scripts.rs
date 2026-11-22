//! The decoder's side of a channel's scripts (the script API's
//! specification sections 3.1, 3.2, 4.4, 10.5, 11.2 and 13; DEC-7266,
//! TODO-7293). A channel's scripts run in a confined process of their
//! own, the script host, which this process never trusts: it reads each
//! script's manifest itself, opens and closes the script host's contexts
//! as the page shown changes, asks it for one step at a time on its own
//! presentation clock, and turns each step's batch into the open page's
//! packets and picture changes.
//!
//! - **The decoder is the authority.** What a script claims is what its
//!   manifest says, read here ([`manifest`]). The rates and bounds of the
//!   specification's section 10.5 are held here again ([`rates`]); a
//!   script host that passes one, answers out of turn, or does not answer
//!   by its deadline is failed for good, and the trusted core is told to
//!   end it.
//! - **A batch reaches only the open page**, as it was when the step was
//!   asked ([`crate::Engine::apply_script`]). A batch for a page the
//!   viewer has left draws nothing.
//! - **Fail open** (section 1 principle 5). An input event whose step was
//!   never answered is reported to the trusted core as not cancelled, so
//!   that its default action is taken.
//! - **Nothing polls.** A step is asked at a tick a context asked for (its
//!   manifest's tick divisor, or the tick its earliest timer is due at,
//!   which each batch says), when input arrives, and when the page has
//!   something to report; with none of these the loop sleeps.

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::time::Instant;

use neoteletext_rules::limits::{MAX_EVENTS_PER_TICK, MAX_KEYS_DOWN};
use protocol::ProtocolError;
use protocol::decoder::script::{ChannelFacts, PointerClaim, PointerKind, ScriptClaims, ScriptOrder, ScriptReport, SourcePiece};
use protocol::decoder::{FromDecoder, write_to_core};
use protocol::rules::{KeyAction, PageSub, PointerAction, RuleInput};
use protocol::script::link::{Asked, HostLink, Received, Stopped};
use protocol::script::relay::{START_DEADLINE, STEP_DEADLINE};
use protocol::script::{
    Batch, ContextFlags, ContextId, ContextOpen, Event, EventKind, FailReason, FromScriptHost, Level, MAX_EVENT_PAYLOAD_BYTES, MAX_SCRIPT_BYTES, Renderer,
    Step, StepFlags,
};

use crate::Engine;
use crate::animation::{due_after, tick_at};
use crate::engine::script::ScriptPage;
use limits::{MAX_NAMED_PAGES, MAX_QUEUED_EVENTS, MAX_SCRIPTS_HELD};
use manifest::Manifest;

pub mod events;
pub mod limits;
pub mod manifest;
pub mod rates;
#[cfg(test)]
mod tests;

/// The language text is encoded for when neither a manifest nor a page
/// names one the engine has a set for: the script host encodes such text
/// as Latin English, and the decoder must choose the same (TODO-7362
/// item 2(c)).
const DEFAULT_LANGUAGE: &str = "en";
/// The most steps and closes that wait for their batch before this side
/// asks no more: the link's own bound less the closes that may be asked
/// beside the steps, since the trusted core counts both against
/// `MAX_STEPS_UNANSWERED` and ends a script host that passes it.
const MAX_UNANSWERED: usize = protocol::script::relay::MAX_STEPS_UNANSWERED - 2 * protocol::script::CONTEXTS;

/// A script held whole, as it arrived.
#[derive(Debug, PartialEq, Eq)]
struct Script {
    name: String,
    source: Vec<u8>,
}

/// A script arriving in pieces.
#[derive(Debug)]
struct Arriving {
    level: ContextId,
    page: PageSub,
    name: String,
    total: usize,
    bytes: Vec<u8>,
}

/// A context that is open in the script host.
#[derive(Debug)]
struct Context {
    manifest: Manifest,
    /// The container tick its first step was, or is to be, asked at.
    first_tick: Option<u64>,
    /// The tick of its earliest timer, as its last batch said.
    next_due: Option<u64>,
    /// It stopped, as a batch said: nothing more is asked for it.
    stopped: bool,
}

/// The page context, and the page it draws on.
#[derive(Debug)]
struct PageContext {
    context: Context,
    at: ScriptPage,
    /// The bytes of the script it runs, to tell the same script arriving
    /// again from another.
    script: PageSub,
    /// The page's packet X/28/0 has been written with the context's
    /// language.
    designated: bool,
    /// Another script has arrived for the page since: the context ends
    /// and the new script's begins.
    stale: bool,
}

/// An input event that waits for a step.
#[derive(Debug)]
struct Queued {
    event: Event,
    /// The trusted core's number for it, which its verdict names; none
    /// for an event the decoder made itself.
    sequence: Option<u32>,
    /// A pointer's move, which a later move replaces (section 4.7).
    moving: bool,
}

/// A step asked and not yet answered.
#[derive(Debug)]
struct Waiting {
    asked: Asked,
    /// The container tick it was asked at.
    tick: u64,
    at: Instant,
    /// Which open page it was asked for.
    epoch: u64,
    /// The trusted core's numbers of its events, in order.
    events: Vec<Option<u32>>,
}

/// What the decoder's loop does after the scripts handled something.
#[derive(Debug, Default)]
pub struct Outcome {
    /// The pages that arrived or changed: reported and drawn as after a
    /// feed.
    pub arrived: Vec<(u16, String)>,
    /// A batch was applied: the pages on screen are looked at again.
    pub applied: bool,
}

/// A channel's scripts, as its decoder holds them.
#[derive(Debug)]
#[expect(clippy::struct_excessive_bools, reason = "independent states of the tab and the clock, each named where it is read")]
pub struct Scripts {
    facts: ChannelFacts,
    /// When the facts were given, to count the epoch on from.
    facts_at: Instant,
    channel_script: Option<Script>,
    pages: BTreeMap<PageSub, Script>,
    /// The order the page scripts arrived in, oldest first.
    arrived: VecDeque<PageSub>,
    /// The script each page's own file names (section 12.4), which wins
    /// over the channel file's for that page (section 12.6).
    named: BTreeMap<PageSub, String>,
    /// The scripts asked of the trusted core and not yet given.
    wanted: BTreeMap<PageSub, String>,
    /// The JavaScript pages (section 12.3): pages with a script and no
    /// page file, which this decoder opened blank. Such a page's renderer
    /// is the one its script's manifest names.
    blank: std::collections::BTreeSet<PageSub>,
    arriving: Option<Arriving>,
    link: HostLink,
    /// The highest API version the script host has, once it said so.
    api_max: Option<u16>,
    /// The page and sub-code shown, of the active tab.
    open: Option<PageSub>,
    /// The page the channel context was last told is shown.
    shown: Option<PageSub>,
    /// The tab is not the active one: no step is asked (section 13 item
    /// 6).
    paused: bool,
    /// The next step says the tab is shown again.
    resumed: bool,
    channel: Option<Context>,
    page: Option<PageContext>,
    /// Counts every change of the open page or its context: a batch
    /// asked under another draws nothing.
    epoch: u64,
    sound_allowed: bool,
    sequence: u32,
    waiting: VecDeque<Waiting>,
    /// When the last piece of an opening was written, while anything
    /// waits ([`START_DEADLINE`]).
    opening: Option<Instant>,
    queue: VecDeque<Queued>,
    /// The keys held down, by name, for the releases a script is owed
    /// when the page loses the keyboard (section 4.7).
    down: Vec<String>,
    /// The decoder tick the container's tick 0 is at, moved on by every
    /// pause so that a script never sees a jump (section 3.2).
    base: Option<u32>,
    /// The decoder tick the pause began at.
    paused_at: Option<u32>,
    /// The last container tick a tick step was asked at.
    last_tick: Option<u64>,
    /// Ticks due whose step was not asked.
    missed: u16,
    /// A step is owed at the next tick whatever the contexts asked: an
    /// opening's first, or something to report.
    owed: bool,
    rates: rates::Rates,
    /// The drawings painted since the last step.
    painted: Vec<i32>,
    claims: Option<ScriptClaims>,
}

impl Default for Scripts {
    fn default() -> Self {
        Self::new()
    }
}

/// Writes a report for the trusted core.
fn report(out: &mut dyn Write, report: ScriptReport) -> Result<(), ProtocolError> {
    write_to_core(out, &FromDecoder::Script(report))
}

/// `text` as a line of the developer log: plain, within the log's bound.
fn log_line(text: &str) -> String {
    text.chars().filter(|character| !character.is_control()).take(protocol::script::MAX_LOG_CHARS).collect()
}

impl Scripts {
    /// No script, and nothing said to a script host.
    #[must_use]
    pub fn new() -> Self {
        Self {
            facts: ChannelFacts::default(),
            facts_at: Instant::now(),
            channel_script: None,
            pages: BTreeMap::new(),
            arrived: VecDeque::new(),
            named: BTreeMap::new(),
            wanted: BTreeMap::new(),
            blank: std::collections::BTreeSet::new(),
            arriving: None,
            link: HostLink::new(),
            api_max: None,
            open: None,
            shown: None,
            paused: false,
            resumed: false,
            channel: None,
            page: None,
            epoch: 0,
            sound_allowed: false,
            sequence: 0,
            waiting: VecDeque::new(),
            opening: None,
            queue: VecDeque::new(),
            down: Vec::new(),
            base: None,
            paused_at: None,
            last_tick: None,
            missed: 0,
            owed: false,
            rates: rates::Rates::default(),
            painted: Vec::new(),
            claims: None,
        }
    }

    /// Whether any context is open.
    #[must_use]
    pub const fn running(&self) -> bool {
        self.channel.is_some() || self.page.is_some()
    }

    /// Why the channel's scripts stopped for good, once they have.
    #[must_use]
    pub const fn stopped(&self) -> Option<FailReason> {
        self.link.stopped()
    }

    /// The page the page context draws on, when one is open.
    #[must_use]
    pub fn page_context(&self) -> Option<ScriptPage> {
        self.page.as_ref().map(|page| page.at)
    }

    /// An order of the trusted core's.
    ///
    /// # Errors
    /// When the decoder's output cannot be written.
    pub fn order(&mut self, order: ScriptOrder, engine: &mut Engine, out: &mut dyn Write) -> Result<Outcome, ProtocolError> {
        match order {
            ScriptOrder::Channel(facts) => {
                self.facts = facts;
                self.facts_at = Instant::now();
            }
            ScriptOrder::Source(piece) => {
                if let Some((level, page, script)) = self.piece(&piece) {
                    // Section 12.6: when a page's own file names a script,
                    // the channel file's script for that page is not used,
                    // and the difference is said for the channel's
                    // developer. Only one page-level script runs for a page.
                    let other = self.named.get(&page).filter(|named| level == ContextId::Page && **named != script.name);
                    if let Some(named) = other {
                        let text = format!("page {:03X}: its page file names the script {named}, so {} is not run for it", page.page, script.name);
                        report(out, ScriptReport::Log { level: 1, context: ContextId::Page, text: log_line(&text) })?;
                    } else {
                        self.keep(level, page, script);
                        // Section 12.3: a page that has a script and no page
                        // file is a JavaScript page, and opens blank. The
                        // decoder holds nothing of it, so it makes the page
                        // here, as soon as its script is whole: the page can
                        // then be shown and opened, which is what starts its
                        // context. A page of any renderer is carried by a
                        // teletext page of its number, as a channel's is.
                        if level == ContextId::Page && self.pages.contains_key(&page) && engine.script_renderer(page.page, page.sub).is_none() {
                            let carrier = ScriptPage { page: page.page, sub: page.sub, renderer: Renderer::Teletext, designation: None };
                            let arrived = engine.script_blank_page(carrier);
                            while self.blank.len() >= MAX_NAMED_PAGES {
                                self.blank.pop_first();
                            }
                            self.blank.insert(page);
                            let mut outcome = self.settle(engine, out)?;
                            outcome.arrived.extend(arrived);
                            return Ok(outcome);
                        }
                    }
                }
            }
            ScriptOrder::NoScript(page) => {
                self.wanted.remove(&page);
                self.pages.remove(&page);
                self.arrived.retain(|held| *held != page);
            }
            ScriptOrder::SoundAllowed => {
                self.sound_allowed = true;
                self.owed |= self.running();
            }
            ScriptOrder::Text { sequence, text } => {
                let claimed = self.claimed(|manifest| manifest.text) && text.len() <= MAX_EVENT_PAYLOAD_BYTES;
                let event = Event { kind: EventKind::Text, numbers: [0; 4], payload: text.into_bytes() };
                let _ = sequence;
                self.enqueue(claimed, event, None, false, out)?;
            }
            ScriptOrder::Wheel { sequence, x, y, delta_x, delta_y } => {
                let claimed = self.claimed(|manifest| manifest.wheel);
                let thing = self.target(engine).and_then(|at| engine.script_pick(at, x, y).first().copied()).unwrap_or(0);
                let event = Event { kind: EventKind::Wheel, numbers: [i32::from(delta_x), i32::from(delta_y), thing, 0], payload: Vec::new() };
                let _ = sequence;
                self.enqueue(claimed, event, None, false, out)?;
            }
            // Section 4.2: a pointer with its number and kind, the mouse's
            // or a contact's. A press and a release are answered, claimed
            // or not, as the rule table's input is.
            ScriptOrder::Pointer(pointer) => {
                let moving = pointer.action == PointerAction::Move;
                let answered = matches!(pointer.action, PointerAction::Press | PointerAction::Release).then_some(pointer.sequence);
                let claim = if moving { PointerClaim::Move } else { PointerClaim::Press };
                let claimed = self.claimed(|manifest| manifest.pointer >= claim);
                let at = self.target(engine);
                let things = at.map(|at| engine.script_pick(at, pointer.x, pointer.y)).unwrap_or_default();
                let teletext = at.is_some_and(|at| at.renderer == Renderer::Teletext);
                let which = events::Which { id: i32::from(pointer.pointer), kind: pointer.kind, primary: pointer.primary };
                let event = events::pointer(pointer.action, pointer.button, (pointer.x, pointer.y), teletext, &things, which);
                // A move of one pointer never replaces another's (section
                // 4.7 merges the waiting moves "for one target"): only the
                // mouse's moves are merged, a contact's are each kept.
                self.enqueue(claimed, event, answered, moving && pointer.kind == PointerKind::Mouse, out)?;
            }
            // Section 4.2: `navigate`, to the page context then the
            // channel's, before the tab goes. Only a cause the browser's
            // own controls made is told today, which no script can cancel,
            // so no verdict is waited for.
            ScriptOrder::TabShown(shown) => self.tab_shown(shown, engine, out)?,
            ScriptOrder::Navigating { cause, target } => {
                let event = Event { kind: EventKind::Navigate, numbers: [i32::from(cause.code()), 0, 0, 0], payload: target.into_bytes() };
                self.enqueue(true, event, None, false, out)?;
            }
        }
        self.settle(engine, out)
    }

    /// Puts a piece of a script with those before it: the script, when
    /// this piece completes it. Pieces come in order from offset 0; one
    /// out of order, or of another script than the one arriving, drops
    /// what arrived.
    fn piece(&mut self, piece: &SourcePiece) -> Option<(ContextId, PageSub, Script)> {
        let total = usize::try_from(piece.total).ok().filter(|total| *total <= MAX_SCRIPT_BYTES)?;
        let offset = usize::try_from(piece.offset).ok()?;
        if offset == 0 {
            self.arriving = Some(Arriving { level: piece.level, page: piece.page, name: piece.name.clone(), total, bytes: Vec::new() });
        }
        let arriving = self.arriving.as_mut()?;
        let same = arriving.level == piece.level && arriving.page == piece.page && arriving.name == piece.name && arriving.total == total;
        if !same || arriving.bytes.len() != offset || offset.saturating_add(piece.bytes.len()) > total {
            self.arriving = None;
            return None;
        }
        arriving.bytes.extend_from_slice(&piece.bytes);
        if arriving.bytes.len() < total {
            return None;
        }
        let whole = self.arriving.take()?;
        Some((whole.level, whole.page, Script { name: whole.name, source: whole.bytes }))
    }

    /// Keeps a script that arrived whole: at most [`MAX_SCRIPTS_HELD`],
    /// the page script that arrived first forgotten for one more.
    fn keep(&mut self, level: ContextId, page: PageSub, script: Script) {
        match level {
            ContextId::Channel => {
                // The channel's script arrives again with each page asked
                // for (TODO-7331): the one running is not started again.
                if self.channel_script.is_none() {
                    self.channel_script = Some(script);
                }
            }
            ContextId::Page => {
                self.wanted.remove(&page);
                // The same bytes again, as each refresh of the page brings
                // them (TODO-7331), leave a running script running.
                if self.pages.get(&page) == Some(&script) {
                    return;
                }
                if let Some(running) = self.page.as_mut().filter(|running| running.script == page) {
                    running.stale = true;
                }
                self.arrived.retain(|held| *held != page);
                while self.arrived.len().saturating_add(1) >= MAX_SCRIPTS_HELD {
                    let oldest = self.arrived.iter().position(|held| Some(*held) != self.open).and_then(|place| self.arrived.remove(place));
                    let Some(oldest) = oldest else { break };
                    self.pages.remove(&oldest);
                }
                self.arrived.push_back(page);
                self.pages.insert(page, script);
            }
        }
    }

    /// A fetched page file was read: `name` is the script its `JS` line or
    /// `SCRIPT_REF` record names for `page`, or none (sections 12.4 to
    /// 12.6). The page file's reference wins: a script held for the page
    /// under another name is dropped, and one the decoder does not hold
    /// is asked of the trusted core by its name, once.
    ///
    /// # Errors
    /// When the decoder's output cannot be written.
    pub fn page_file_names(&mut self, page: PageSub, name: Option<String>, engine: &mut Engine, out: &mut dyn Write) -> Result<Outcome, ProtocolError> {
        match name {
            Some(name) => {
                if self.pages.get(&page).is_some_and(|held| held.name != name) {
                    self.pages.remove(&page);
                    self.arrived.retain(|held| *held != page);
                }
                if !self.pages.contains_key(&page) && self.wanted.get(&page) != Some(&name) && self.link.stopped().is_none() {
                    report(out, ScriptReport::Wanted { page, name: name.clone() })?;
                    while self.wanted.len() >= MAX_NAMED_PAGES {
                        self.wanted.pop_first();
                    }
                    self.wanted.insert(page, name.clone());
                }
                while self.named.len() >= MAX_NAMED_PAGES && !self.named.contains_key(&page) {
                    self.named.pop_first();
                }
                self.named.insert(page, name);
            }
            // The file names none any more: the script it named goes, and
            // the channel file's, if it names one, comes with the page's
            // next fetch.
            None => {
                if self.named.remove(&page).is_some() {
                    self.wanted.remove(&page);
                    self.pages.remove(&page);
                    self.arrived.retain(|held| *held != page);
                }
            }
        }
        self.settle(engine, out)
    }

    /// The page and sub-code of the active tab that is open, or none
    /// while none is (the rule table's `RulesPage`, which the trusted
    /// core already sends): a page's context starts and ends with the
    /// page. None pauses nothing: the trusted core says it between two
    /// pages of one tab, while the next is fetched and drawn, and the
    /// contexts of the page left live until the next one is named
    /// (section 3.1; TODO-7405). Whether the tab is shown is told apart
    /// ([`ScriptOrder::TabShown`]).
    ///
    /// # Errors
    /// When the decoder's output cannot be written.
    pub fn page_opened(&mut self, page: Option<PageSub>, engine: &mut Engine, out: &mut dyn Write) -> Result<Outcome, ProtocolError> {
        if let Some(page) = page.filter(|page| self.open != Some(*page)) {
            self.open = Some(page);
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.settle(engine, out)
    }

    /// The channel's tab is hidden, or shown again (section 3.1's table):
    /// hidden, the contexts are paused after one last step, in which the
    /// script host itself tells them `visibilitychange` and releases what
    /// sounds; shown, the next step says so and the tick goes on from
    /// where it stopped (section 13 item 6). The decoder makes no
    /// visibility event: a step's flags say it, and the script host
    /// refuses the event from anyone else.
    fn tab_shown(&mut self, shown: bool, engine: &Engine, out: &mut dyn Write) -> Result<(), ProtocolError> {
        if shown && self.paused {
            self.paused = false;
            self.resumed = self.running();
            // The ticks of the pause never happened (section 3.2).
            let now = tick_at(engine.now_us());
            if let (Some(base), Some(since)) = (self.base.as_mut(), self.paused_at.take()) {
                *base = base.saturating_add(now.saturating_sub(since));
            }
        } else if !shown && !self.paused {
            self.paused = true;
            self.paused_at = Some(tick_at(engine.now_us()));
            if self.running() && self.link.stopped().is_none() {
                // The release of every key held rides in the last step
                // (section 4.7).
                self.lose_keys();
                self.step(engine, false, StepFlags { pausing: true, ..StepFlags::default() }, out)?;
            }
        }
        Ok(())
    }

    /// Brings the script host's contexts to what the scripts held and the
    /// page shown ask for (section 3.1's table), then asks whatever step
    /// is due.
    fn settle(&mut self, engine: &mut Engine, out: &mut dyn Write) -> Result<Outcome, ProtocolError> {
        let mut outcome = Outcome::default();
        if self.link.stopped().is_some() || self.paused {
            return Ok(outcome);
        }
        let Some(open) = self.open else { return Ok(outcome) };
        let mut changed = false;
        // The page context of a page that is no longer shown, or whose
        // script is no longer the one held, ends first.
        let wanted = self.pages.contains_key(&open).then_some(open);
        if self.page.as_ref().is_some_and(|page| page.stale || Some(page.script) != wanted) {
            self.close_page(out)?;
            changed = true;
        }
        // The channel context is created first, so that its top level
        // returns before the page's begins.
        if self.channel.is_none()
            && let Some(script) = self.channel_script.take()
        {
            // One that is refused is forgotten; it is given again with
            // the next page asked for.
            if let Some((context, _)) = self.open_context(ContextId::Channel, open, &script, engine, out)? {
                self.channel_script = Some(script);
                self.channel = Some(context);
                changed = true;
            }
        }
        if self.page.is_none()
            && wanted.is_some()
            && let Some(script) = self.pages.remove(&open)
        {
            let opened = self.open_context(ContextId::Page, open, &script, engine, out)?;
            if let Some((context, at)) = opened {
                outcome.arrived = engine.script_blank_page(at);
                self.page = Some(PageContext { context, at, script: open, designated: false, stale: false });
                self.pages.insert(open, script);
                self.epoch = self.epoch.wrapping_add(1);
                changed = true;
            }
            // A script that was refused is forgotten, so that it is not
            // read again at every turn; it is given again with its page.
        }
        // The channel context hears of the page shown (section 4.2).
        if self.channel.is_some() && self.shown != Some(open) {
            if let Some(before) = self.shown {
                self.push(
                    Event { kind: EventKind::PageHide, numbers: [i32::from(before.page), i32::from(before.sub), 0, 0], payload: Vec::new() },
                    None,
                    false,
                );
            }
            let renderer = self.target(engine).map_or(Renderer::Teletext, |at| at.renderer);
            let shown = Event {
                kind: EventKind::PageShow,
                numbers: [i32::from(open.page), i32::from(open.sub), i32::from(renderer.code()), 0],
                payload: self.page_url(open).into_bytes(),
            };
            self.push(shown, None, false);
            self.shown = Some(open);
        }
        if changed {
            self.say_claims(out)?;
            self.owed = true;
        }
        self.wake(engine, out)?;
        Ok(outcome)
    }

    /// The address of a page of the channel: its folder and the page's
    /// number as the channel's file names spell it (section 3.3;
    /// REQ-508), with its sub-code when it has one. The decoder is not
    /// told which file gave the page, so the address has no extension.
    fn page_url(&self, page: PageSub) -> String {
        if page.sub == 0 {
            format!("{}{:03X}", self.facts.channel_url, page.page)
        } else {
            format!("{}{:03X}/{:04X}", self.facts.channel_url, page.page, page.sub)
        }
    }

    /// The page a batch draws on now: the page context's, or with none
    /// the page shown as the engine holds it, for the channel context
    /// (section 3.1: "what the channel context may draw").
    fn target(&self, engine: &Engine) -> Option<ScriptPage> {
        if let Some(page) = &self.page {
            return Some(page.at);
        }
        let open = self.open?;
        let renderer = engine.script_renderer(open.page, open.sub).unwrap_or(Renderer::Teletext);
        Some(ScriptPage { page: open.page, sub: open.sub, renderer, designation: None })
    }

    /// Opens a context for `script` on `page`: reads its manifest, the
    /// one place a script's claims come from, and writes the opening.
    /// `None` when the script is refused, with the reason logged.
    fn open_context(
        &mut self,
        level: ContextId,
        page: PageSub,
        script: &Script,
        engine: &Engine,
        out: &mut dyn Write,
    ) -> Result<Option<(Context, ScriptPage)>, ProtocolError> {
        let refuse = |out: &mut dyn Write, why: &str| {
            report(out, ScriptReport::Log { level: 0, context: level, text: log_line(&format!("the script {} was not run: {why}", script.name)) })
                .map(|()| None)
        };
        let manifest = match manifest::read(&script.source) {
            Ok(manifest) => manifest,
            Err(error) => return refuse(out, &error.to_string()),
        };
        if self.api_max.is_some_and(|highest| manifest.api > highest) {
            return refuse(out, "its manifest names a later version of the API than this browser has");
        }
        // A JavaScript page was made blank by this decoder: it has no
        // renderer of its own until its script draws.
        let held = if self.blank.contains(&page) { None } else { engine.script_renderer(page.page, page.sub) };
        // Section 3.1: for a script attached to a page file the manifest's
        // renderer equals the page file's or is absent; a JavaScript page
        // draws on the renderer its manifest names.
        let renderer = match (level, manifest.renderer, held) {
            (ContextId::Page, Some(named), Some(held)) if named != held => return refuse(out, "its manifest names another renderer than its page file's"),
            (ContextId::Page, named, held) => held.or(named).unwrap_or(Renderer::Teletext),
            (ContextId::Channel, _, held) => held.unwrap_or(Renderer::Teletext),
        };
        let language = manifest.language.clone().unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned());
        let designation = neoteletext_core::charset::language::language_value(&language, true)
            .or_else(|| neoteletext_core::charset::language::language_value(DEFAULT_LANGUAGE, true));
        let source_bytes = u32::try_from(script.source.len()).map_err(|_| ProtocolError::TooLong(script.source.len()))?;
        let open = ContextOpen {
            context: level,
            api: manifest.api,
            renderer,
            level: level_of(engine),
            page: page.page,
            subcode: page.sub,
            flags: ContextFlags {
                pointer_press: manifest.pointer >= PointerClaim::Press,
                pointer_move: manifest.pointer >= PointerClaim::Move,
                wheel: manifest.wheel,
                text: manifest.text,
                // A file opened from disk has the network, by the owner's ruling
                // on local files (DEC-7315 version 2): its origin is `null` and its
                // requests leave through a web client started in the
                // loopback address space, which is the trusted core's to
                // arrange; the decoder only says what the page is.
                network: manifest.network,
                from_disk: self.facts.from_disk,
                sound_allowed: self.sound_allowed,
            },
            tick_divisor: manifest.tick,
            language,
            channel_url: self.facts.channel_url.clone(),
            page_url: self.page_url(page),
            script_url: format!("{}{}", self.facts.channel_url, script.name),
            channel_title: self.facts.channel_title.clone(),
            // The script host makes the seed fresh itself (DEC-7357 item 6).
            seed: [0; protocol::script::SEED_BYTES],
            epoch_seconds: self.facts.epoch_seconds.saturating_add(self.facts_at.elapsed().as_secs()),
            zone_minutes: self.facts.zone_minutes,
            keys: manifest.keys.clone(),
            sounds: Vec::new(),
            source_bytes,
        };
        match self.link.open(out, &open, &script.source) {
            Ok(()) => {}
            Err(protocol::script::link::Refusal::Message(ProtocolError::Io(error))) => return Err(ProtocolError::Io(error)),
            Err(refusal) => return refuse(out, &refusal.to_string()),
        }
        self.opening = Some(Instant::now());
        if self.base.is_none() {
            self.base = Some(tick_at(engine.now_us()));
        }
        let at = ScriptPage {
            page: page.page,
            sub: page.sub,
            renderer,
            designation: (level == ContextId::Page && renderer == Renderer::Teletext).then_some(designation).flatten(),
        };
        Ok(Some((Context { manifest, first_tick: None, next_due: None, stopped: false }, at)))
    }

    /// Ends the page context (section 3.1): its one handling of `close`
    /// is the script host's, whose batch for it draws nothing.
    fn close_page(&mut self, out: &mut dyn Write) -> Result<(), ProtocolError> {
        if self.page.take().is_none() {
            return Ok(());
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.rates.page_ended();
        match self.link.close(out, ContextId::Page) {
            Ok(()) => {
                self.waiting.push_back(Waiting {
                    asked: Asked::Close(ContextId::Page),
                    tick: self.last_tick.unwrap_or(0),
                    at: Instant::now(),
                    epoch: self.epoch,
                    events: Vec::new(),
                });
                Ok(())
            }
            Err(protocol::script::link::Refusal::Message(ProtocolError::Io(error))) => Err(ProtocolError::Io(error)),
            Err(_) => Ok(()),
        }
    }

    /// Whether an open context's manifest claims something.
    fn claimed(&self, claims: impl Fn(&Manifest) -> bool) -> bool {
        self.channel.iter().chain(self.page.iter().map(|page| &page.context)).any(|context| !context.stopped && claims(&context.manifest))
    }

    /// Tells the trusted core what the open contexts claim together
    /// (section 4.6), when it differs from what was last told.
    fn say_claims(&mut self, out: &mut dyn Write) -> Result<(), ProtocolError> {
        let Some(page) = self.open else { return Ok(()) };
        let mut claims = ScriptClaims::none(page);
        for context in self.channel.iter().chain(self.page.iter().map(|page| &page.context)).filter(|context| !context.stopped) {
            let manifest = &context.manifest;
            claims.running = true;
            claims.network |= manifest.network;
            claims.text |= manifest.text;
            claims.wheel |= manifest.wheel;
            claims.pointer = claims.pointer.max(manifest.pointer);
            for key in &manifest.keys {
                if !claims.keys.contains(key) && claims.keys.len() < protocol::script::MAX_KEY_ASKS {
                    claims.keys.push(key.clone());
                }
            }
        }
        if self.link.stopped().is_some() {
            claims = ScriptClaims::none(page);
        }
        if self.claims.as_ref() == Some(&claims) {
            return Ok(());
        }
        report(out, ScriptReport::Claims(claims.clone()))?;
        self.claims = Some(claims);
        Ok(())
    }

    /// An input event of the viewer for the open page, as the trusted
    /// core gives it to the page's rule table too (`ToDecoder::RuleInput`).
    /// One a script claims waits for the next step, which is asked at
    /// once; any other is answered at once as not cancelled.
    ///
    /// # Errors
    /// When the decoder's output cannot be written.
    pub fn input(&mut self, page: PageSub, sequence: u32, input: &RuleInput, engine: &mut Engine, out: &mut dyn Write) -> Result<(), ProtocolError> {
        // Only an event with a default a script may cancel is answered
        // (section 4.2's table: a key down, and the pointer's press and
        // release); no other is waited for.
        let answered = matches!(
            input,
            RuleInput::Key { action: KeyAction::Press | KeyAction::Repeat, .. }
                | RuleInput::Pointer { action: PointerAction::Press | PointerAction::Release, .. }
        );
        let sequence = answered.then_some(sequence);
        let default =
            |out: &mut dyn Write| sequence.map_or(Ok(()), |sequence| report(out, ScriptReport::Verdict { sequence, cancelled: false, handled: false }));
        if !self.running() || self.link.stopped().is_some() || self.open != Some(page) || self.paused {
            return default(out);
        }
        let at = self.target(engine);
        match *input {
            RuleInput::Key { action, named, code, mods, .. } => {
                let name = events::key_name(named, code);
                let claimed = name.as_ref().is_some_and(|name| self.claimed(|manifest| manifest.keys.contains(name)));
                let Some(name) = name.filter(|_| claimed) else { return default(out) };
                let down = action != KeyAction::Release;
                if down && !self.down.contains(&name) && self.down.len() < MAX_KEYS_DOWN {
                    self.down.push(name.clone());
                } else if !down {
                    self.down.retain(|held| *held != name);
                }
                // A repeat that waits behind another of its key is
                // dropped (section 4.7).
                if action == KeyAction::Repeat
                    && self.queue.iter().any(|queued| queued.event.kind == EventKind::KeyDown && queued.event.payload == name.as_bytes())
                {
                    return default(out);
                }
                self.enqueue(true, events::key(action, mods, &name), sequence, false, out)?;
            }
            RuleInput::Pointer { action, button, x, y, .. } => {
                let claim = if action == PointerAction::Move { PointerClaim::Move } else { PointerClaim::Press };
                let claimed = self.claimed(|manifest| manifest.pointer >= claim);
                let things = at.map(|at| engine.script_pick(at, x, y)).unwrap_or_default();
                let teletext = at.is_some_and(|at| at.renderer == Renderer::Teletext);
                let event = events::pointer(action, button, (x, y), teletext, &things, events::Which::MOUSE);
                self.enqueue(claimed, event, sequence, action == PointerAction::Move, out)?;
            }
            RuleInput::Drawing { leave, drawing } => {
                let claimed = self.claimed(|manifest| manifest.pointer >= PointerClaim::Press);
                self.enqueue(claimed, events::hover(leave, i32::from(drawing)), sequence, false, out)?;
            }
            RuleInput::Focus => self.enqueue(true, Event { kind: EventKind::Focus, numbers: [0; 4], payload: Vec::new() }, sequence, false, out)?,
            RuleInput::Blur => {
                self.lose_keys();
                self.enqueue(true, Event { kind: EventKind::Blur, numbers: [0; 4], payload: Vec::new() }, sequence, false, out)?;
            }
            // Text and the wheel come with what a script needs of them, as
            // orders of their own.
            RuleInput::Text | RuleInput::Wheel { .. } => {}
        }
        self.wake(engine, out)
    }

    /// Queues the release of every key held, marked as not seen (section
    /// 4.7: "the script is given lost releases for everything down").
    fn lose_keys(&mut self) {
        for name in core::mem::take(&mut self.down) {
            self.queue.push_front(Queued { event: events::lost_release(&name), sequence: None, moving: false });
        }
    }

    /// Queues an event a script claims, or answers one none claims.
    fn enqueue(&mut self, claimed: bool, event: Event, sequence: Option<u32>, moving: bool, out: &mut dyn Write) -> Result<(), ProtocolError> {
        if !claimed || !self.running() || self.link.stopped().is_some() || self.paused {
            return sequence.map_or(Ok(()), |sequence| report(out, ScriptReport::Verdict { sequence, cancelled: false, handled: false }));
        }
        for sequence in self.push(event, sequence, moving) {
            report(out, ScriptReport::Verdict { sequence, cancelled: false, handled: false })?;
        }
        Ok(())
    }

    /// Queues an event within the queue's bound (section 4.7): a waiting
    /// move is replaced by a later one, and when the queue is full the
    /// oldest move goes first, then the oldest event. The trusted core's
    /// numbers of the events that will never be given, for their verdicts.
    fn push(&mut self, event: Event, sequence: Option<u32>, moving: bool) -> Vec<u32> {
        let mut lost = Vec::new();
        if moving && let Some(place) = self.queue.iter().position(|queued| queued.moving) {
            lost.extend(self.queue.remove(place).and_then(|queued| queued.sequence));
        }
        while self.queue.len() >= MAX_QUEUED_EVENTS {
            let place = self.queue.iter().position(|queued| queued.moving).unwrap_or(0);
            lost.extend(self.queue.remove(place).and_then(|queued| queued.sequence));
        }
        self.queue.push_back(Queued { event, sequence, moving });
        lost
    }

    /// The container tick now.
    fn tick(&self, engine: &Engine) -> u64 {
        u64::from(tick_at(engine.now_us()).saturating_sub(self.base.unwrap_or(0)))
    }

    /// The next container tick a context asked a step at, after the last
    /// one asked: its manifest's tick, counted from its first step, or
    /// the tick of its earliest timer.
    fn next_tick(&self) -> Option<u64> {
        let after = self.last_tick.map_or(0, |last| last.saturating_add(1));
        let due = |context: &Context| {
            if context.stopped {
                return None;
            }
            let divisor = u64::from(context.manifest.tick);
            let first = context.first_tick.unwrap_or(after);
            let ticked = (divisor > 0).then(|| {
                let since = after.saturating_sub(first);
                first.saturating_add(since.div_ceil(divisor).saturating_mul(divisor))
            });
            [ticked, context.next_due.map(|due| due.max(after))].into_iter().flatten().min()
        };
        let asked = self.channel.iter().chain(self.page.iter().map(|page| &page.context)).filter_map(due).min();
        if self.owed { Some(asked.map_or(after, |asked| asked.min(after))) } else { asked }
    }

    /// When the loop must wake for the scripts by itself, on the engine's
    /// clock: the next tick a step is due at, and the deadline of the
    /// oldest step that waits.
    #[must_use]
    pub fn due_us(&self, engine: &Engine) -> Option<u64> {
        if self.link.stopped().is_some() || !self.running() {
            return None;
        }
        let now = engine.now_us();
        let deadline =
            self.deadline().map(|at| now.saturating_add(u64::try_from(at.saturating_duration_since(Instant::now()).as_micros()).unwrap_or(u64::MAX)));
        let step = if self.paused {
            None
        } else {
            self.next_tick().map(|tick| {
                let decoder_tick = u32::try_from(tick).unwrap_or(u32::MAX).saturating_add(self.base.unwrap_or(0));
                // `due_after` gives when the tick after its argument is due.
                decoder_tick.checked_sub(1).map_or(0, due_after)
            })
        };
        [deadline, step].into_iter().flatten().min()
    }

    /// When the oldest thing asked must have been answered (section 3.2
    /// rule 5(b)): [`STEP_DEADLINE`] after it was asked, or
    /// [`START_DEADLINE`] after the last piece of an opening when that is
    /// later, as the trusted core counts it.
    fn deadline(&self) -> Option<Instant> {
        let asked = self.waiting.front()?.at.checked_add(STEP_DEADLINE)?;
        let opening = self.opening.and_then(|at| at.checked_add(START_DEADLINE));
        Some(opening.map_or(asked, |opening| asked.max(opening)))
    }

    /// The loop woke: the script host is failed when it has not answered
    /// by its deadline, and a step is asked when one is due.
    ///
    /// # Errors
    /// When the decoder's output cannot be written.
    pub fn wake(&mut self, engine: &mut Engine, out: &mut dyn Write) -> Result<(), ProtocolError> {
        if self.link.stopped().is_some() || !self.running() {
            return Ok(());
        }
        if self.deadline().is_some_and(|deadline| Instant::now() >= deadline) {
            return self.fail(FailReason::Time, "STEP_DEADLINE: the script host did not answer in time", out);
        }
        if self.paused {
            return Ok(());
        }
        if let Some(at) = self.page.as_ref().map(|page| page.at) {
            self.painted.extend(engine.script_painted(at));
            self.painted.truncate(protocol::script::MAX_WATCHED_PAINTINGS);
            self.owed |= !self.painted.is_empty();
        }
        let now = self.tick(engine);
        let due = self.next_tick().filter(|due| *due <= now);
        let ticking = due.is_some();
        // The ticks a context asked for that passed with no step are told
        // in the next one's `missed` (section 3.2).
        let divisor = self
            .channel
            .iter()
            .chain(self.page.iter().map(|page| &page.context))
            .map(|context| u64::from(context.manifest.tick))
            .filter(|tick| *tick > 0)
            .min();
        if let (Some(due), Some(divisor)) = (due, divisor) {
            let skipped = now.saturating_sub(due).checked_div(divisor).unwrap_or(0);
            self.missed = self.missed.saturating_add(u16::try_from(skipped).unwrap_or(u16::MAX));
        }
        if ticking || !self.queue.is_empty() || self.resumed {
            self.step(engine, ticking, StepFlags::default(), out)?;
        }
        Ok(())
    }

    /// Asks a step: the events that wait, at most a tick's, and what the
    /// page has to report. With no room for one more the tick is counted
    /// as missed (section 3.2).
    fn step(&mut self, engine: &Engine, ticking: bool, flags: StepFlags, out: &mut dyn Write) -> Result<(), ProtocolError> {
        if !self.link.has_room() || self.link.unanswered() >= MAX_UNANSWERED {
            if ticking {
                self.missed = self.missed.saturating_add(1);
                self.last_tick = Some(self.tick(engine));
            }
            return Ok(());
        }
        let tick = self.tick(engine);
        // Lost releases are never held back (section 4.7): they are at the
        // queue's front, and ride beside the tick's events.
        let lost = self.queue.iter().take_while(|queued| queued.sequence.is_none() && queued.event.kind == EventKind::KeyUp).count().min(MAX_KEYS_DOWN);
        let taken: Vec<Queued> = self.queue.drain(..lost.saturating_add(MAX_EVENTS_PER_TICK).min(self.queue.len())).collect();
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        let step = Step {
            sequence,
            tick,
            missed: core::mem::take(&mut self.missed),
            flags: StepFlags { tick: ticking, resumed: core::mem::take(&mut self.resumed), sound_allowed: self.sound_allowed, ..flags },
            events: taken.iter().map(|queued| queued.event.clone()).collect(),
            painted: core::mem::take(&mut self.painted),
            ..Step::default()
        };
        let numbers: Vec<Option<u32>> = taken.iter().map(|queued| queued.sequence).collect();
        match self.link.step(out, &step) {
            Ok(()) => {}
            Err(protocol::script::link::Refusal::Message(ProtocolError::Io(error))) => return Err(ProtocolError::Io(error)),
            // A step this side could not write is the decoder's own
            // fault; its events keep their defaults.
            Err(_) => return Self::defaults(&numbers, out),
        }
        self.waiting.push_back(Waiting { asked: Asked::Step(sequence), tick, at: Instant::now(), epoch: self.epoch, events: numbers });
        if ticking || self.owed {
            self.last_tick = Some(tick);
        }
        self.owed = false;
        for context in self.channel.iter_mut().chain(self.page.iter_mut().map(|page| &mut page.context)) {
            context.first_tick.get_or_insert(tick);
            // A timer that was due is asked for; the batch says the next.
            if context.next_due.is_some_and(|due| due <= tick) {
                context.next_due = None;
            }
        }
        Ok(())
    }

    /// Reports each of `events` as not cancelled.
    fn defaults(events: &[Option<u32>], out: &mut dyn Write) -> Result<(), ProtocolError> {
        events.iter().flatten().try_for_each(|sequence| report(out, ScriptReport::Verdict { sequence: *sequence, cancelled: false, handled: false }))
    }

    /// Stops the channel's scripts for good from this side, and tells the
    /// trusted core, which ends the script host and starts none until the
    /// channel is reloaded (section 3.1).
    fn fail(&mut self, reason: FailReason, text: &str, out: &mut dyn Write) -> Result<(), ProtocolError> {
        match self.link.fail(reason, text) {
            Received::Stopped(stopped) => self.stopped_now(&stopped, out),
            _ => Ok(()),
        }
    }

    /// The scripts stopped for good: every event that waited keeps its
    /// default (section 1 principle 5), nothing is claimed any more, and
    /// the last picture stays.
    fn stopped_now(&mut self, stopped: &Stopped, out: &mut dyn Write) -> Result<(), ProtocolError> {
        let waiting: Vec<Waiting> = self.waiting.drain(..).collect();
        for step in &waiting {
            Self::defaults(&step.events, out)?;
        }
        let queued: Vec<Option<u32>> = self.queue.drain(..).map(|queued| queued.sequence).collect();
        Self::defaults(&queued, out)?;
        self.channel = None;
        self.page = None;
        self.down.clear();
        self.opening = None;
        self.epoch = self.epoch.wrapping_add(1);
        report(out, ScriptReport::Stopped { reason: stopped.reason, text: log_line(&stopped.text) })?;
        self.say_claims(out)
    }

    /// A message of the script link, as the decoder's reader gave it: the
    /// script host's, or the trusted core's own FAILED. One its reader
    /// refused stops the scripts (section 11.2).
    ///
    /// # Errors
    /// When the decoder's output cannot be written.
    pub fn received(&mut self, message: Result<FromScriptHost, ProtocolError>, engine: &mut Engine, out: &mut dyn Write) -> Result<Outcome, ProtocolError> {
        let received = match message {
            Ok(message) => self.link.receive(message),
            Err(error) => self.link.fail(FailReason::Link, &format!("the script host's message was refused: {error}")),
        };
        let mut outcome = Outcome::default();
        match received {
            Received::Ready { api_max } => self.api_max = Some(api_max),
            Received::Dropped => {}
            Received::Stopped(stopped) => self.stopped_now(&stopped, out)?,
            Received::Batch { asked, batch } => {
                let Some(waiting) = self.waiting.pop_front().filter(|waiting| waiting.asked == asked) else {
                    self.fail(FailReason::Link, "the script host's answer is not to what this decoder asked", out)?;
                    return Ok(outcome);
                };
                if self.waiting.is_empty() {
                    self.opening = None;
                }
                outcome = self.take_batch(&waiting, &batch, engine, out)?;
                self.wake(engine, out)?;
            }
        }
        Ok(outcome)
    }

    /// Takes the batch that answers `waiting`: counts it against the
    /// rates, applies its drawing to the open page when that is still the
    /// page the step was asked for, and passes on its verdicts and what
    /// it asks of the browser.
    fn take_batch(&mut self, waiting: &Waiting, batch: &Batch, engine: &mut Engine, out: &mut dyn Write) -> Result<Outcome, ProtocolError> {
        let mut outcome = Outcome::default();
        if let Err(passed) = self.rates.count(waiting.tick, batch) {
            Self::defaults(&waiting.events, out)?;
            self.fail(FailReason::Link, &format!("{}: the script host passed a rate this decoder holds", passed.0), out)?;
            return Ok(outcome);
        }
        // Section 4.1: a verdict for each event of the step, in order; an
        // event the batch has none for keeps its default, and a close's
        // batch has none.
        let verdicts = batch.verdicts.as_deref().unwrap_or_default();
        if verdicts.len() > waiting.events.len() {
            Self::defaults(&waiting.events, out)?;
            self.fail(FailReason::Link, "the script host answered more events than its step carried", out)?;
            return Ok(outcome);
        }
        for (place, sequence) in waiting.events.iter().enumerate() {
            let verdict = verdicts.get(place).copied().unwrap_or_default();
            if let Some(sequence) = sequence {
                report(out, ScriptReport::Verdict { sequence: *sequence, cancelled: verdict.cancelled, handled: verdict.handled })?;
            }
        }
        // Only the open page, as it was when the step was asked: a batch
        // of a page the viewer has left, and a close's, draw nothing.
        let current = waiting.epoch == self.epoch && matches!(waiting.asked, Asked::Step(_));
        if let Some(at) = self.target(engine).filter(|_| current) {
            let designate = self.page.as_ref().is_some_and(|page| !page.designated && at.designation.is_some());
            match engine.apply_script(at, batch, designate) {
                Ok(arrived) => {
                    outcome.arrived.extend(arrived);
                    outcome.applied = true;
                }
                Err(refused) => {
                    self.fail(FailReason::Link, refused.0, out)?;
                    return Ok(outcome);
                }
            }
            if let Some(page) =
                self.page.as_mut().filter(|_| batch.teletext_rows.is_some() || batch.teletext_enhancement.is_some() || batch.teletext_settings.is_some())
            {
                page.designated = true;
            }
            if let Some(painted) = batch.watch.as_ref().and_then(|watch| watch.painted.as_ref()) {
                engine.script_watch(at, painted);
            }
            for label in batch.labels.iter().flatten() {
                report(out, ScriptReport::Label { page: PageSub::new(at.page, at.sub), handle: label.handle, label: label.label.clone() })?;
            }
            if let Some(title) = &batch.title {
                report(out, ScriptReport::Title { page: PageSub::new(at.page, at.sub), title: title.clone() })?;
            }
            if let Some(target) = &batch.navigate {
                report(out, ScriptReport::Navigate { page: PageSub::new(at.page, at.sub), target: target.clone() })?;
            }
        }
        // Sound is the channel's as much as the page's: a close's batch
        // stops what its context played.
        if let (Some(operations), Some(open)) = (&batch.sound, self.open) {
            engine.script_sound(open.page, operations);
        }
        for line in batch.log.iter().flatten() {
            report(out, ScriptReport::Log { level: line.level, context: line.context, text: line.text.clone() })?;
        }
        let mut stopped = false;
        for state in batch.state.iter().flatten() {
            let context = match state.context {
                ContextId::Channel => self.channel.as_mut(),
                ContextId::Page => self.page.as_mut().filter(|_| current).map(|page| &mut page.context),
            };
            let Some(context) = context else { continue };
            context.next_due = state.next_due;
            if state.stopped.is_some() && !context.stopped {
                context.stopped = true;
                stopped = true;
            }
        }
        if stopped {
            self.say_claims(out)?;
        }
        Ok(outcome)
    }

    /// The channel is closing: each context has its one handling of
    /// `close`, and the script host is told to end.
    ///
    /// # Errors
    /// When the decoder's output cannot be written.
    pub fn shutdown(&mut self, out: &mut dyn Write) -> Result<(), ProtocolError> {
        for context in [ContextId::Page, ContextId::Channel] {
            if let Err(protocol::script::link::Refusal::Message(ProtocolError::Io(error))) = self.link.close(out, context) {
                return Err(ProtocolError::Io(error));
            }
        }
        self.channel = None;
        self.page = None;
        self.link.shutdown(out)
    }
}

/// The presentation level in force, as a context is told it (`nt.level`).
fn level_of(engine: &Engine) -> Level {
    use neoteletext_core::config::PresentationLevel;
    match engine.config().level {
        PresentationLevel::Level1 => Level::One,
        PresentationLevel::Level15 => Level::OneFive,
        PresentationLevel::Level25 => Level::TwoFive,
        PresentationLevel::Level35 => Level::ThreeFive,
    }
}
