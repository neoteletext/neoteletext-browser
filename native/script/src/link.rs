//! What runs a (tab, channel)'s scripts, as one interface
//! (script-api.md section 11.1): [`ScriptLink`], and its in-process
//! implementation [`InProcess`], which calls the library directly. The
//! browser's implementation, which speaks the messages of section 11.2 to
//! a script-host process, is work package F3's.
//!
//! The types a link carries ([`ContextOpen`], [`Step`], [`Batch`] and
//! their parts) are **stand-ins** for the message types work package F1
//! (TODO-7288) defines in `native/protocol/src/script.rs`, which had not
//! landed when this was written: each holds what section 11.2's table
//! gives the message of its name, for the sections the shared core fills.
//! They are replaced by F1's types when those land; the reconciliation is
//! recorded as owed.

use crate::container::{Container, Settings};
use crate::limits::{
    MAX_EVENT_PAYLOAD_BYTES, MAX_KEY_ASKS, MAX_KEY_NAME_BYTES, MAX_LANGUAGE_BYTES, MAX_NETWORK_EVENTS_PER_STEP, MAX_SCRIPT_BYTES, MAX_STEP_CYCLE_ENDS,
    MAX_STEP_EVENTS, MAX_STEP_FAULTS, MAX_STEP_PAINTED, MAX_STEP_SOUND_REPORTS, MAX_TICK_DIVISOR, MAX_TITLE_BYTES, MAX_URL_BYTES,
};
use crate::machine::Engine;
use crate::ops::EventKind;
use crate::random::SEED_BYTES;

/// Which of a container's two contexts (section 3.1), numbered as the
/// `CONTEXT_OPEN` message numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextId {
    /// The channel context: it lives while the tab is in the channel.
    Channel = 1,
    /// The page context: it lives while one page or sub-page is shown.
    Page = 2,
}

impl ContextId {
    /// Both, in the order events are dispatched: the page context first
    /// (section 4.5).
    pub const DISPATCH_ORDER: [Self; 2] = [Self::Page, Self::Channel];

    /// Its number.
    #[must_use]
    pub const fn number(self) -> i32 {
        self as i32
    }

    /// The context numbered `number`.
    #[must_use]
    pub const fn from_number(number: i32) -> Option<Self> {
        match number {
            1 => Some(Self::Channel),
            2 => Some(Self::Page),
            _ => None,
        }
    }

    /// Its place in a table of the two.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Channel => 0,
            Self::Page => 1,
        }
    }

    /// The other context.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Channel => Self::Page,
            Self::Page => Self::Channel,
        }
    }
}

/// The renderer a page draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    /// Teletext's cells.
    Teletext,
    /// ReGIS's numbered drawings.
    Regis,
    /// NAPLPS's backdrop and cast.
    Naplps,
}

impl Renderer {
    /// The name the API gives it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Teletext => "teletext",
            Self::Regis => "regis",
            Self::Naplps => "naplps",
        }
    }
}

/// What a manifest's `pointer` member claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerClaim {
    /// No pointer events.
    #[default]
    None,
    /// Press, release, enter, leave and cancel.
    Press,
    /// Those and moves.
    Move,
}

/// What a script's manifest claims (section 3.1), as the decoder read it
/// before any code ran. It is the ceiling: an event the manifest did not
/// claim is never given to the context.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Claims {
    /// The keys claimed, by name: at most [`MAX_KEY_ASKS`], each at most [`MAX_KEY_NAME_BYTES`].
    pub keys: Vec<String>,
    /// Committed text.
    pub text: bool,
    /// The pointer.
    pub pointer: PointerClaim,
    /// The wheel.
    pub wheel: bool,
    /// 0: no tick events. n: one every n ticks, at most [`MAX_TICK_DIVISOR`].
    pub tick_divisor: u8,
    /// The script uses the network.
    pub network: bool,
}

/// The `CONTEXT_OPEN` message: everything a context is told of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextOpen {
    /// Which context.
    pub context: ContextId,
    /// The API version the manifest names.
    pub api: u16,
    /// The renderer of the page shown; none for a channel context opened
    /// before any page.
    pub renderer: Option<Renderer>,
    /// The page's number, 0x100 to 0x8FF.
    pub page: u16,
    /// The page's sub-code, or none.
    pub subcode: Option<u16>,
    /// The manifest's claims.
    pub claims: Claims,
    /// The language text is encoded for, at most [`MAX_LANGUAGE_BYTES`].
    pub language: String,
    /// The channel's address, the page file's and the script's, each at most [`MAX_URL_BYTES`].
    pub channel_url: String,
    /// See `channel_url`.
    pub page_url: String,
    /// See `channel_url`.
    pub script_url: String,
    /// The channel's title, at most [`MAX_TITLE_BYTES`].
    pub channel_title: String,
    /// The context's seed, fresh from the operating system (section 13 item 7).
    pub seed: [u8; SEED_BYTES],
    /// The time the context starts, in seconds since 1970.
    pub epoch_seconds: u64,
    /// Local time, in minutes east of UTC.
    pub zone_minutes: i16,
    /// The handles of the sounds the channel announced for this context.
    pub sounds: Vec<u8>,
    /// Whether the viewer has pressed a key or the pointer in this tab yet (section 3.5).
    pub sound_allowed: bool,
}

/// The payload of one event of a step.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EventPayload {
    /// None.
    #[default]
    None,
    /// A string.
    Text(String),
    /// Bytes.
    Bytes(Vec<u8>),
    /// Whole numbers.
    Ints(Vec<i32>),
}

impl EventPayload {
    /// Its length in bytes as it crosses.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        match self {
            Self::None => 0,
            Self::Text(text) => text.len(),
            Self::Bytes(bytes) => bytes.len(),
            Self::Ints(ints) => ints.len().saturating_mul(size_of::<i32>()),
        }
    }
}

/// One input event of a step: a kind of section 10.3 with its four whole
/// numbers and its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEvent {
    /// Its kind.
    pub kind: EventKind,
    /// Its four whole numbers.
    pub ints: [i32; 4],
    /// Its payload, at most [`MAX_EVENT_PAYLOAD_BYTES`].
    pub payload: EventPayload,
}

/// What the decoder reports of a playing sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundReport {
    /// A cue point was reached.
    Cue {
        /// The voice.
        voice: u8,
        /// The cue's number.
        cue: u16,
    },
    /// The sound ended.
    Ended {
        /// The voice.
        voice: u8,
    },
}

/// A renderer's fault, for the `fault` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// The renderer's fault code.
    pub code: i32,
    /// The bound's name.
    pub bound: String,
}

/// The `STEP` message: one presentation tick, or input that arrived
/// between ticks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[expect(clippy::struct_excessive_bools, reason = "the STEP message's flags byte, a field for each bit")]
pub struct Step {
    /// Its number, which the batch answers with.
    pub sequence: u32,
    /// The presentation clock's tick.
    pub tick: u64,
    /// Ticks the decoder did not send since the last step.
    pub missed: u16,
    /// Whether this step is a tick (and not only input between ticks).
    pub is_tick: bool,
    /// The tab is being hidden: the contexts are told and their sound released.
    pub pausing: bool,
    /// The first step after the tab was shown again.
    pub resumed: bool,
    /// The viewer has pressed a key or the pointer in this tab.
    pub sound_allowed: bool,
    /// The input events, at most [`MAX_STEP_EVENTS`].
    pub events: Vec<StepEvent>,
    /// Things painted since the last step, at most [`MAX_STEP_PAINTED`].
    pub painted: Vec<i32>,
    /// Things whose `once` cycle ended, at most [`MAX_STEP_CYCLE_ENDS`].
    pub cycle_ended: Vec<i32>,
    /// Sound cues and ends, at most [`MAX_STEP_SOUND_REPORTS`].
    pub sound: Vec<SoundReport>,
    /// Renderer faults, at most [`MAX_STEP_FAULTS`].
    pub faults: Vec<Fault>,
}

/// One operation of a batch's `SOUND` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundOperation {
    /// Play one of the channel's sounds on a voice.
    Play {
        /// The voice.
        voice: u8,
        /// The sound's handle.
        sound: u8,
        /// The gain, 0 to 100.
        gain: u8,
    },
    /// Stop what a voice plays.
    Stop {
        /// The voice.
        voice: u8,
    },
    /// Start a synthesizer on a part.
    SynthStart {
        /// The part.
        part: u8,
        /// The family, by the engine's codec number.
        family: u8,
        /// The variant.
        variant: u8,
        /// The gain, 0 to 100.
        gain: u8,
    },
    /// A MIDI channel message.
    Midi {
        /// The part.
        part: u8,
        /// The status: 0x80, 0x90, 0xB0, 0xC0 or 0xE0.
        status: u8,
        /// The first data byte.
        data1: u8,
        /// The second data byte.
        data2: u8,
    },
    /// All notes off and the instrument released.
    SynthStop {
        /// The part.
        part: u8,
    },
}

/// One line of a batch's `LOG` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// 0 debug, 1 log, 2 info, 3 warn, 4 error.
    pub level: u8,
    /// The context it came from, or none for the container itself.
    pub context: Option<ContextId>,
    /// The line.
    pub text: String,
}

/// Why a context or a container was stopped: the `FAILED` message's reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// A time limit or budget was passed.
    Time = 1,
    /// The container's memory was exhausted.
    Memory = 2,
    /// The interpreter's stack limit was passed.
    Stack = 3,
    /// The container trapped.
    Trap = 4,
    /// Too many failures in a row.
    Failures = 5,
    /// The link failed.
    Link = 6,
}

/// A context's line in a batch's `STATE` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextState {
    /// Which context.
    pub context: ContextId,
    /// Why it is stopped, or none while it runs.
    pub stopped: Option<StopReason>,
    /// Its failures in a row.
    pub failures: u16,
    /// The presentation tick of its earliest timer or held note, so that
    /// the decoder wakes for it and never polls.
    pub next_due: Option<u64>,
}

/// One entry of a batch's `WATCH` section as the shared core fills it: a
/// thing and a kind of event a context listens to it for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    /// The context listening.
    pub context: ContextId,
    /// The thing's handle.
    pub handle: i32,
    /// The kind of event.
    pub kind: EventKind,
}

/// The `BATCH` message: everything one step changed, for the sections the
/// shared core fills.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Batch {
    /// The step's number.
    pub sequence: u32,
    /// One byte for each event of the step: bit 0 default cancelled, bit 1
    /// handled by a listener.
    pub verdicts: Vec<u8>,
    /// The sound operations, in order.
    pub sound: Vec<SoundOperation>,
    /// The page's new title.
    pub title: Option<String>,
    /// Where a script asked to go.
    pub navigate: Option<String>,
    /// What was logged.
    pub log: Vec<LogLine>,
    /// Log lines counted and dropped past the bound.
    pub log_dropped: u32,
    /// The watch list, whole, when it changed.
    pub watch: Option<Vec<Watch>>,
    /// Each context's state.
    pub states: Vec<ContextState>,
}

/// An answer for a script from the web client (section 11.3), as the
/// shared core carries it: the context, an event kind of section 10.3
/// numbered 30 to 41, its whole numbers and its payload. Work package W2
/// gives these their meaning; the core only queues and delivers them in
/// the step's second phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FromFetcher {
    /// The context it is for.
    pub context: ContextId,
    /// The event's number, 30 to 41.
    pub kind: i32,
    /// Its whole numbers.
    pub ints: [i32; 4],
    /// Its payload.
    pub payload: EventPayload,
}

/// What a script asked of the network (section 11.3). The shared core has
/// no network operation, so none is ever made here; work package W2 fills
/// this in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToFetcher {
    /// The context that asked.
    pub context: ContextId,
    /// The message, encoded.
    pub bytes: Vec<u8>,
}

/// Why a link refused or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// A field of a message was past its bound; the bound's name.
    Bound(&'static str),
    /// The context is already open, or is not open.
    Context(&'static str),
    /// A message held what its sender may not send.
    Refused(&'static str),
    /// The container is stopped for good, for this reason.
    Stopped(StopReason, String),
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bound(name) => write!(formatter, "a message passed the bound {name}"),
            Self::Context(what) => write!(formatter, "the context {what}"),
            Self::Refused(what) => write!(formatter, "refused: {what}"),
            Self::Stopped(reason, text) => write!(formatter, "the container is stopped ({reason:?}): {text}"),
        }
    }
}

impl core::error::Error for LinkError {}

/// What runs a (tab, channel)'s scripts. The browser's implementation
/// speaks the messages of section 11.2 to a script-host process; the
/// runner's calls the library in its own process. Nothing else differs
/// between them.
pub trait ScriptLink {
    /// Opens a context and runs `source`'s top level in it.
    ///
    /// # Errors
    /// When a field is past its bound, the context is already open, or
    /// the container is stopped. A top level that throws is not an error
    /// of the link: it is logged and counted as the context's failure.
    fn open(&mut self, context: ContextOpen, source: &[u8]) -> Result<(), LinkError>;
    /// Runs one step and gives everything it changed.
    ///
    /// # Errors
    /// When a field is past its bound or the container is stopped.
    fn step(&mut self, step: Step) -> Result<Batch, LinkError>;
    /// Ends a context: one handling of `close`, then its realm is freed.
    fn close(&mut self, context: ContextId);
    /// An answer for a script from the web client.
    fn network(&mut self, event: FromFetcher);
    /// What the scripts asked of the network since the last call.
    fn take_requests(&mut self) -> Vec<ToFetcher>;
}

/// Checks a `CONTEXT_OPEN`'s fields against their bounds.
///
/// # Errors
/// The first bound passed.
pub fn check_open(open: &ContextOpen, source: &[u8]) -> Result<(), LinkError> {
    let within = |ok: bool, name: &'static str| if ok { Ok(()) } else { Err(LinkError::Bound(name)) };
    within(source.len() <= MAX_SCRIPT_BYTES, "MAX_SCRIPT_BYTES")?;
    within(open.claims.keys.len() <= MAX_KEY_ASKS, "MAX_KEY_ASKS")?;
    within(open.claims.keys.iter().all(|key| !key.is_empty() && key.len() <= MAX_KEY_NAME_BYTES), "MAX_KEY_NAME_BYTES")?;
    within(open.claims.tick_divisor <= MAX_TICK_DIVISOR, "MAX_TICK_DIVISOR")?;
    within(open.language.len() <= MAX_LANGUAGE_BYTES, "MAX_LANGUAGE_BYTES")?;
    within(open.channel_url.len() <= MAX_URL_BYTES && open.page_url.len() <= MAX_URL_BYTES && open.script_url.len() <= MAX_URL_BYTES, "MAX_URL_BYTES")?;
    within(open.channel_title.len() <= MAX_TITLE_BYTES, "MAX_TITLE_BYTES")?;
    within(u8::try_from(open.sounds.len()).is_ok(), "MAX_SOUND_NUMBER")
}

/// Checks a `STEP`'s fields against their bounds.
///
/// # Errors
/// The first bound passed.
pub fn check_step(step: &Step) -> Result<(), LinkError> {
    let within = |ok: bool, name: &'static str| if ok { Ok(()) } else { Err(LinkError::Bound(name)) };
    within(step.events.len() <= MAX_STEP_EVENTS, "MAX_STEP_EVENTS")?;
    within(step.events.iter().all(|event| event.payload.byte_len() <= MAX_EVENT_PAYLOAD_BYTES), "MAX_EVENT_PAYLOAD_BYTES")?;
    within(step.painted.len() <= MAX_STEP_PAINTED, "MAX_STEP_PAINTED")?;
    within(step.cycle_ended.len() <= MAX_STEP_CYCLE_ENDS, "MAX_STEP_CYCLE_ENDS")?;
    within(step.sound.len() <= MAX_STEP_SOUND_REPORTS, "MAX_STEP_SOUND_REPORTS")?;
    within(step.faults.len() <= MAX_STEP_FAULTS && step.faults.iter().all(|fault| fault.bound.len() <= MAX_TITLE_BYTES), "MAX_STEP_FAULTS")
}

/// The in-process link: the library's container, called directly. The
/// standalone runner uses it, and the browser's script-host program wraps
/// it in the messages of section 11.2.
pub struct InProcess {
    engine: Engine,
    settings: Settings,
    container: Option<Container>,
    /// Network answers waiting for the next step, at most
    /// [`MAX_NETWORK_EVENTS_PER_STEP`] times the steps they wait.
    waiting: Vec<FromFetcher>,
}

impl InProcess {
    /// A link whose containers are made in `engine` with `settings`.
    #[must_use]
    pub const fn new(engine: Engine, settings: Settings) -> Self {
        Self { engine, settings, container: None, waiting: Vec::new() }
    }

    /// The container, once a context has been opened.
    pub const fn container(&mut self) -> Option<&mut Container> {
        self.container.as_mut()
    }
}

impl ScriptLink for InProcess {
    fn open(&mut self, context: ContextOpen, source: &[u8]) -> Result<(), LinkError> {
        check_open(&context, source)?;
        if self.container.is_none() {
            let made =
                Container::new(&self.engine, &self.settings, context.zone_minutes).map_err(|stop| LinkError::Stopped(stop.reason(), stop.to_string()))?;
            self.container = Some(made);
        }
        let Some(container) = self.container.as_mut() else { return Err(LinkError::Context("could not be made")) };
        container.open(context, source)
    }

    fn step(&mut self, step: Step) -> Result<Batch, LinkError> {
        let Some(container) = self.container.as_mut() else { return Err(LinkError::Context("is not open")) };
        for event in self.waiting.drain(..) {
            container.network(event);
        }
        container.step(&step)
    }

    fn close(&mut self, context: ContextId) {
        if let Some(container) = self.container.as_mut() {
            container.close(context);
            if !container.is_open(ContextId::Channel) && !container.is_open(ContextId::Page) {
                self.container = None;
            }
        }
    }

    fn network(&mut self, event: FromFetcher) {
        // Held to a bound here too: answers past it are dropped, and the
        // web client's own flow control (section 11.3's `More`) is what
        // keeps a well-behaved one from sending them.
        if self.waiting.len() < MAX_NETWORK_EVENTS_PER_STEP.saturating_mul(MAX_NETWORK_EVENTS_PER_STEP) {
            self.waiting.push(event);
        }
    }

    fn take_requests(&mut self) -> Vec<ToFetcher> {
        self.container.as_mut().map(Container::take_requests).unwrap_or_default()
    }
}
