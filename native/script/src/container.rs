//! One container (script-api.md section 3.1): one instance of the script
//! engine's module holding at most a channel context and a page context,
//! with the event loop that steps them (section 3.2) and the time budget
//! that holds them (the security review's findings S-04 and S-05,
//! TODO-7270 and TODO-7271).
//!
//! A step runs, in order: the input events of the step, each to the page
//! context and then the channel context; network events that arrived;
//! timers that are due; and the tick. Microtasks run to exhaustion after
//! each callback, inside its limit. Three limits hold the time, all on
//! the monotonic wall clock from entry to exit, so that time inside a
//! host call is the script's time:
//!
//! - [`START_TIME_LIMIT`] for a context's top level and
//!   [`CALLBACK_TIME_LIMIT`] for one callback: passing either stops the
//!   container for good;
//! - [`STEP_TIME_BUDGET`] for everything a step runs: once it is used no
//!   further callback of the step starts, and the rest waits for the next
//!   step (lost releases alone are never held back);
//! - [`SECOND_TIME_BUDGET`] over any [`TICKS_PER_SECOND`] consecutive
//!   ticks: passing it stops the container for good.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use protocol::script::{Batch, ContextId, ContextOpen, Event, FailReason, SoundReport, Step};
use protocol::web::{FromWebClient, ToWebClient};

use crate::host::{Host, percent};
use crate::host::{Input, InputPayload};
use crate::limits::{
    CALLBACK_TIME_LIMIT, CLOSE_TIME_LIMIT, MAX_EVENTS_PER_TICK, MAX_KEY_NAME_BYTES, MAX_KEYS_DOWN, MAX_NETWORK_EVENTS_PER_STEP, MAX_POINTER_OBJECTS,
    MAX_QUEUED_EVENTS, MAX_TEXT_EVENT_BYTES, MAX_URL_BYTES, SECOND_TIME_BUDGET, START_TIME_LIMIT, STEP_TIME_BUDGET, TICKS_PER_SECOND,
};
use crate::link::{Context as _, DISPATCH_ORDER, LinkError, check_open};
use crate::machine::{Engine, Entry, Given, Machine, Stop};
use crate::ops::{DispatchBit, EventKind, Handle, KeyBit, NavigateCause, PointerAction};
use crate::random::math_seed;
use crate::watchdog::Watchdog;

/// The log levels the container itself writes at: a warning, an error.
const LEVEL_WARN: u8 = 3;
const LEVEL_ERROR: u8 = 4;
/// The bits of a pointer event's first whole number that hold the action.
const POINTER_ACTION_MASK: i32 = 0x0F;
/// The whole numbers a pointer event's payload starts with: the pointer's
/// number, its buttons, and its place within the thing hit.
const POINTER_PAYLOAD_INTS: usize = 4;
/// The low 31 bits of a tick count, as a tick event carries it.
const TICK_MASK: u64 = 0x7FFF_FFFF;
/// The name a script is run under when it has no address.
const UNNAMED_SCRIPT: &str = "script.js";
/// The limit [`TimeLimits::unhurried`] gives everything: an hour.
const UNHURRIED: Duration = Duration::from_secs(3600);
/// The name the prelude is run under.
const PRELUDE_NAME: &str = "nt:prelude";

/// The time a container's scripts are held to (section 3.2 rule 1 and
/// section 10.6), as one value so that the program that runs the library
/// states its policy in one place. The browser's script host uses the
/// default, which is the named constants and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeLimits {
    /// A context's top level: [`START_TIME_LIMIT`].
    pub start: Duration,
    /// One callback with its microtasks: [`CALLBACK_TIME_LIMIT`].
    pub callback: Duration,
    /// Everything one step runs: [`STEP_TIME_BUDGET`].
    pub step: Duration,
    /// Script time in any second of ticks: [`SECOND_TIME_BUDGET`].
    pub second: Duration,
    /// The one handling of `close`: [`CLOSE_TIME_LIMIT`].
    pub close: Duration,
}

impl Default for TimeLimits {
    fn default() -> Self {
        Self { start: START_TIME_LIMIT, callback: CALLBACK_TIME_LIMIT, step: STEP_TIME_BUDGET, second: SECOND_TIME_BUDGET, close: CLOSE_TIME_LIMIT }
    }
}

impl TimeLimits {
    /// Limits so long that no clock decides what a script sees: an hour
    /// for each. For a run that must be the same every time it is made
    /// whatever the machine is doing, which a budget on the wall clock
    /// cannot be: the library's tests of behaviour, and the standalone
    /// runner's stepped runs (section 16, "steps time deterministically").
    /// Never for a script that is not the author's own.
    #[must_use]
    pub const fn unhurried() -> Self {
        Self { start: UNHURRIED, callback: UNHURRIED, step: UNHURRIED, second: UNHURRIED, close: UNHURRIED }
    }
}

/// How containers are made.
#[derive(Clone, Default)]
pub struct Settings {
    /// The time the scripts are held to.
    pub time: TimeLimits,
    /// The watchdog thread that stands behind every entry, if any: the
    /// script-host program gives one that ends its process.
    pub watchdog: Option<Watchdog>,
}

/// What running a container has cost, for the measurements and the runner.
#[derive(Debug, Default, Clone)]
pub struct Costs {
    /// Making the container: instantiating the module, starting the C library and QuickJS.
    pub container: Duration,
    /// Opening each context: making its realm, running the prelude and the script's top level.
    pub opens: Vec<Duration>,
    /// The script time of the last step, and of the longest.
    pub last_step: Duration,
    /// See `last_step`.
    pub longest_step: Duration,
    /// The longest any one callback has run.
    pub longest_callback: Duration,
    /// The longest any one slice of fuel has taken to spend.
    pub longest_slice: Duration,
    /// The slices of fuel spent.
    pub slices: u64,
    /// The callbacks run.
    pub callbacks: u64,
}

/// Which contexts a waiting event is still owed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owed {
    /// Both, the page context first.
    Both,
    /// The channel context alone: the page context has had it.
    Channel,
}

/// An input event waiting for a step with time for it (section 4.7).
#[derive(Debug, Clone)]
struct Waiting {
    event: Input,
    /// The step it came in and its place there, for its verdict.
    origin: Option<(u32, usize)>,
    owed: Owed,
    /// A lost release: delivered first and never held back by a budget.
    lost: bool,
}

/// One container.
pub struct Container {
    machine: Machine,
    time: TimeLimits,
    prelude: String,
    queue: VecDeque<Waiting>,
    network: VecDeque<FromWebClient>,
    /// The script time used at each of the last second's ticks.
    second: VecDeque<(u64, Duration)>,
    /// The script time the step running has used.
    step_used: Duration,
    /// Tick events that could not start, for each context's next one.
    missed: [u32; 2],
    told_sound_allowed: bool,
    /// Whether the log has said that a network answer was dropped.
    told_no_network: bool,
    costs: Costs,
}

/// Whether a step may carry an event of `kind`: the rest are the host's
/// own to make.
const fn is_input(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::KEY_DOWN
            | EventKind::KEY_UP
            | EventKind::TEXT
            | EventKind::POINTER
            | EventKind::WHEEL
            | EventKind::FOCUS
            | EventKind::BLUR
            | EventKind::NAVIGATE
            | EventKind::PAGE_SHOW
            | EventKind::PAGE_HIDE
            | EventKind::UI
    )
}

/// A step's event as the host gives it to a context: of a kind a step
/// may carry, its bytes read as that kind's payload (a key's name, text
/// and addresses as UTF-8; a pointer's whole numbers, four bytes each,
/// big-endian as every number of the message is), within its bound.
fn input_of(event: &Event) -> Result<Input, LinkError> {
    let kind = EventKind::of(event.kind);
    if !is_input(kind) {
        return Err(LinkError::Refused("event kind: only the host makes it"));
    }
    let wrong = LinkError::Refused("event payload");
    let text = || core::str::from_utf8(&event.payload).map(str::to_owned).map_err(|_| LinkError::Refused("event payload"));
    let payload = match kind {
        EventKind::KEY_DOWN | EventKind::KEY_UP => {
            let name = text()?;
            if name.is_empty() || name.len() > MAX_KEY_NAME_BYTES {
                return Err(wrong);
            }
            InputPayload::Text(name)
        }
        EventKind::TEXT => {
            let typed = text()?;
            if typed.len() > MAX_TEXT_EVENT_BYTES {
                return Err(wrong);
            }
            InputPayload::Text(typed)
        }
        EventKind::NAVIGATE | EventKind::PAGE_SHOW => {
            let address = text()?;
            if address.len() > MAX_URL_BYTES {
                return Err(wrong);
            }
            InputPayload::Text(address)
        }
        EventKind::POINTER => {
            let (words, rest) = event.payload.as_chunks::<4>();
            if !rest.is_empty() || words.len() < POINTER_PAYLOAD_INTS || words.len() > POINTER_PAYLOAD_INTS.saturating_add(MAX_POINTER_OBJECTS) {
                return Err(wrong);
            }
            InputPayload::Ints(words.iter().map(|word| i32::from_be_bytes(*word)).collect())
        }
        EventKind::UI if !event.payload.is_empty() => InputPayload::Text(text()?),
        _ if event.payload.is_empty() => InputPayload::None,
        _ => return Err(wrong),
    };
    Ok(Input { kind, ints: event.numbers, payload })
}

/// Whether a script may cancel the default action of this event (section
/// 4.2). The host decides, not the prelude: an event whose default is the
/// viewer leaving, or that the browser's own controls caused, cannot be
/// stopped by any script.
fn cancelable(event: &Input) -> bool {
    let [first, ..] = event.ints;
    match event.kind {
        EventKind::KEY_DOWN | EventKind::UI => true,
        EventKind::POINTER => {
            let action = first & POINTER_ACTION_MASK;
            action == PointerAction::DOWN.number() || action == PointerAction::UP.number()
        }
        EventKind::NAVIGATE => first == NavigateCause::SCRIPT.number() || first == NavigateCause::LINK.number() || first == NavigateCause::KEY.number(),
        _ => false,
    }
}

impl Container {
    /// Makes a container with no context, its local time `zone_minutes`
    /// east of UTC.
    ///
    /// # Errors
    /// When the container cannot be made.
    pub fn new(engine: &Engine, settings: &Settings, zone_minutes: i16) -> Result<Self, Stop> {
        let machine = Machine::new(engine, Host::new(), zone_minutes, settings.watchdog.clone())?;
        let costs = Costs { container: machine.start_time, ..Costs::default() };
        Ok(Self {
            machine,
            time: settings.time,
            prelude: crate::ops::prelude(),
            queue: VecDeque::new(),
            network: VecDeque::new(),
            second: VecDeque::new(),
            step_used: Duration::ZERO,
            missed: [0; 2],
            told_sound_allowed: false,
            told_no_network: false,
            costs,
        })
    }

    /// What running the container has cost.
    #[must_use]
    pub const fn costs(&self) -> &Costs {
        &self.costs
    }

    /// Whether `context` is open.
    #[must_use]
    pub fn is_open(&self, context: ContextId) -> bool {
        self.machine.host_ref().is_open(context)
    }

    /// Why the container was stopped for good, if it was.
    #[must_use]
    pub const fn stopped(&self) -> Option<&Stop> {
        self.machine.stopped()
    }

    /// The bytes of the container's linear memory and of QuickJS's heap in it.
    #[must_use]
    pub fn memory(&self) -> (usize, usize) {
        (self.machine.linear_bytes(), self.machine.heap_bytes())
    }

    /// The bytes the host holds for the container outside that memory.
    #[must_use]
    pub fn host_bytes(&self) -> usize {
        self.machine.host_ref().state_bytes()
    }

    /// The error a stopped container answers every call with.
    fn stopped_error(&self) -> Option<LinkError> {
        self.machine.stopped().map(|stop| LinkError::Stopped(stop.reason(), stop.to_string()))
    }

    /// Notes that the container has stopped: every context is marked, and
    /// the log says why.
    fn note_stop(&mut self, stop: &Stop) {
        self.machine.stop(stop.clone());
        let host = self.machine.host();
        host.end_callback();
        host.stop_all(stop.reason());
        host.log_line(LEVEL_ERROR, None, &stop.to_string());
    }

    /// Opens a context and runs `source`'s top level in it, all within
    /// [`START_TIME_LIMIT`]. A top level that throws is logged and counted
    /// as a failure of the context, which stays open with whatever it had
    /// set up, as on the web.
    ///
    /// # Errors
    /// When a field is past its bound, the context is already open, or
    /// the container is or becomes stopped.
    pub fn open(&mut self, open: ContextOpen, source: &[u8]) -> Result<(), LinkError> {
        check_open(&open, source)?;
        if let Some(error) = self.stopped_error() {
            return Err(error);
        }
        let context = open.context;
        if self.is_open(context) {
            return Err(LinkError::Context("is already open"));
        }
        let began = Instant::now();
        let deadline = began.checked_add(self.time.start);
        let name = if open.script_url.is_empty() { UNNAMED_SCRIPT.to_owned() } else { open.script_url.clone() };
        let seed = math_seed(&open.seed);
        let result = (|| {
            self.machine.open_context(context, seed, deadline)?;
            self.machine.host().open(open, source.len());
            self.machine.host().begin_callback(context, None, 0);
            let prelude = core::mem::take(&mut self.prelude);
            let ran = self.machine.eval(context, prelude.as_bytes(), PRELUDE_NAME, false, deadline);
            self.prelude = prelude;
            let ran = ran?;
            if ran.threw {
                return Err(Stop::Container(format!("the prelude failed: {}", ran.error.unwrap_or_default())));
            }
            self.machine.eval(context, source, &name, true, deadline)
        })();
        self.machine.host().end_callback();
        self.costs.opens.push(began.elapsed());
        self.costs.longest_slice = self.machine.longest_slice;
        self.costs.slices = self.machine.slices;
        match result {
            Ok(entry) => {
                self.after_entry(context, &entry);
                Ok(())
            }
            Err(stop) => {
                self.note_stop(&stop);
                Err(LinkError::Stopped(stop.reason(), stop.to_string()))
            }
        }
    }

    /// Ends a context: one handling of `close` within
    /// [`CLOSE_TIME_LIMIT`], then every handle, listener and timer of it
    /// is dropped, its sound released and its realm freed. What the
    /// handling staged leaves in the next batch, or by [`Self::drain`].
    pub fn close(&mut self, context: ContextId) {
        if !self.is_open(context) {
            return;
        }
        if self.machine.stopped().is_none() {
            // Not a step's callback: the close has its own limit.
            let _ = self.deliver(context, EventKind::CLOSE, [0; 4], Given::None, self.time.close);
        }
        if self.machine.stopped().is_none()
            && let Err(stop) = self.machine.close_context(context, Instant::now().checked_add(self.time.close))
        {
            self.note_stop(&stop);
        }
        self.machine.host().close(context);
        self.queue.retain(|waiting| !(context == ContextId::Channel && waiting.owed == Owed::Channel));
        // An answer names its request, not its context: which requests a
        // context made is for work package W2 to keep, with the requests.
        if !self.is_open(ContextId::Channel) && !self.is_open(ContextId::Page) {
            self.network.clear();
        }
        if let Some(missed) = self.missed.get_mut(context.index()) {
            *missed = 0;
        }
    }

    /// The staged output outside a step: what a `close` handling left.
    pub fn drain(&mut self, sequence: u32) -> Batch {
        self.machine.host().take_batch(sequence, &[])
    }

    /// An answer for a script from the web client, for the next step. At
    /// most [`MAX_QUEUED_EVENTS`] wait; more are dropped.
    pub fn network(&mut self, event: FromWebClient) {
        if self.network.len() < MAX_QUEUED_EVENTS {
            self.network.push_back(event);
        }
    }

    /// What the scripts asked of the network: nothing, until work package
    /// W2 adds the operations that ask.
    #[expect(clippy::missing_const_for_fn, reason = "the interface work package W2 fills in, which will not be constant")]
    pub fn take_requests(&mut self) -> Vec<ToWebClient> {
        Vec::new()
    }

    /// Whether the step has time left for another callback to start.
    fn has_time(&self) -> bool {
        self.step_used < self.time.step
    }

    /// Accounts for an entry that returned: its time, its failure or
    /// success, and its error in the log.
    fn after_entry(&mut self, context: ContextId, entry: &Entry) {
        self.step_used = self.step_used.saturating_add(entry.elapsed);
        self.costs.callbacks = self.costs.callbacks.saturating_add(1);
        self.costs.longest_callback = self.costs.longest_callback.max(entry.elapsed);
        self.costs.longest_slice = self.machine.longest_slice;
        self.costs.slices = self.machine.slices;
        let callback_limit = self.time.callback;
        let host = self.machine.host();
        host.figures.callback_percent = percent(entry.elapsed, callback_limit);
        host.figures.callback_percent_max = host.figures.callback_percent_max.max(host.figures.callback_percent);
        let failed = entry.threw || entry.rejected || entry.dispatch & DispatchBit::THREW.number() != 0;
        if let Some(error) = &entry.error {
            let what = if entry.threw { "uncaught" } else { "unhandled rejection" };
            host.log_line(LEVEL_ERROR, Some(context), &format!("{what}: {error}"));
        }
        if host.count_failure(context, failed) {
            host.release_sound(context);
            host.log_line(LEVEL_ERROR, Some(context), "MAX_CONSECUTIVE_FAILURES: the context failed too many times in a row and was stopped");
        }
    }

    /// Gives `context` one event as a callback within `limit`, unless the
    /// context is not running or has no listener for the kind (a timer
    /// always has one). The entry, or none when the context was not
    /// entered.
    ///
    /// # Errors
    /// The stop, when the container is or becomes stopped.
    fn deliver(&mut self, context: ContextId, kind: EventKind, numbers: [i32; 4], payload: Given<'_>, limit: Duration) -> Result<Option<Entry>, Stop> {
        if let Some(stop) = self.machine.stopped() {
            return Err(stop.clone());
        }
        let host = self.machine.host_ref();
        if !host.runs(context) || (kind != EventKind::TIMER && !host.listens(context, kind)) {
            return Ok(None);
        }
        self.machine.host().begin_callback(context, Some(kind), numbers[0]);
        let result = self.machine.event(context, kind.number(), numbers, payload, Instant::now().checked_add(limit));
        self.machine.host().end_callback();
        match result {
            Ok(entry) => {
                self.after_entry(context, &entry);
                Ok(Some(entry))
            }
            Err(stop) => {
                self.note_stop(&stop);
                Err(stop)
            }
        }
    }

    /// Tells the other context of each change of the shared store, while
    /// the step has time (section 3.6: in the same step, after the
    /// callback that made the change).
    fn tell_shared(&mut self) -> Result<(), Stop> {
        while self.has_time() {
            let Some((context, key)) = self.machine.host().take_shared_notice() else { break };
            self.deliver(context, EventKind::SHARED, [0; 4], Given::Text(&key), self.time.callback)?;
        }
        Ok(())
    }

    /// Adds an input event to those waiting, by the rules of section 4.7.
    fn enqueue(&mut self, event: Input, origin: Option<(u32, usize)>) {
        let [first, second, third, fourth] = event.ints;
        let is_move = |event: &Input| event.kind == EventKind::POINTER && event.ints[0] & POINTER_ACTION_MASK == PointerAction::MOVE.number();
        if is_move(&event) {
            // Waiting moves for one target become the latest.
            if let Some(waiting) =
                self.queue.iter_mut().find(|waiting| waiting.owed == Owed::Both && is_move(&waiting.event) && waiting.event.ints[3] == fourth)
            {
                waiting.event = event;
                waiting.origin = origin;
                return;
            }
        } else if event.kind == EventKind::WHEEL {
            // Wheel amounts are summed.
            if let Some(waiting) =
                self.queue.iter_mut().find(|waiting| waiting.owed == Owed::Both && waiting.event.kind == EventKind::WHEEL && waiting.event.ints[2] == third)
            {
                waiting.event.ints[0] = waiting.event.ints[0].saturating_add(first);
                waiting.event.ints[1] = waiting.event.ints[1].saturating_add(second);
                waiting.origin = origin;
                return;
            }
        }
        if self.queue.len() >= MAX_QUEUED_EVENTS {
            // The oldest moves go first.
            if let Some(index) = self.queue.iter().position(|waiting| is_move(&waiting.event)) {
                self.queue.remove(index);
            } else if is_move(&event) || event.kind == EventKind::WHEEL {
                return;
            } else {
                // A key or button event is lost: the script is given lost
                // releases for everything down, so that nothing stays held.
                self.lose_everything_down();
                return;
            }
        }
        // A release the decoder says was lost (the page lost the keyboard,
        // was paused or is closing) goes ahead of everything but the lost
        // releases already waiting, and is never held back by a budget.
        if event.kind == EventKind::KEY_UP && first & KeyBit::REPEAT_OR_LOST.number() != 0 {
            let after = self.queue.iter().take_while(|waiting| waiting.lost).count();
            self.queue.insert(after, Waiting { event, origin, owed: Owed::Both, lost: true });
            return;
        }
        self.queue.push_back(Waiting { event, origin, owed: Owed::Both, lost: false });
    }

    /// Queues a lost release for every key down and a cancel for the
    /// pointer, ahead of everything waiting; they are never held back.
    fn lose_everything_down(&mut self) {
        let keys = self.machine.host_ref().keys_held();
        for name in keys.into_iter().take(MAX_KEYS_DOWN.saturating_mul(2)) {
            if self.queue.iter().any(|waiting| waiting.lost && waiting.event.payload == InputPayload::Text(name.clone())) {
                continue;
            }
            let event = Input { kind: EventKind::KEY_UP, ints: [KeyBit::REPEAT_OR_LOST.number(), 0, 0, 0], payload: InputPayload::Text(name) };
            self.queue.push_front(Waiting { event, origin: None, owed: Owed::Both, lost: true });
        }
        if self.machine.host_ref().pointer_pressed() && !self.queue.iter().any(|waiting| waiting.lost && waiting.event.kind == EventKind::POINTER) {
            let event =
                Input { kind: EventKind::POINTER, ints: [PointerAction::CANCEL.number(), 0, 0, 0], payload: InputPayload::Ints(vec![0; POINTER_PAYLOAD_INTS]) };
            self.queue.push_front(Waiting { event, origin: None, owed: Owed::Both, lost: true });
        }
    }

    /// Dispatches one waiting event: to the page context, then the channel
    /// context, which is given the same fields with no target (section
    /// 4.5). Its verdict's bits; and whether the channel context is still
    /// owed it because the step ran out of time between the two.
    fn dispatch(&mut self, waiting: &Waiting) -> Result<(i32, bool), Stop> {
        let mut bits = 0;
        let event = &waiting.event;
        // Reports about a thing go to the context that listens to it, and
        // sound's to whoever listens to `nt.sound`.
        let about_thing = matches!(event.kind, EventKind::PAINTED | EventKind::CYCLE_END | EventKind::UI);
        for context in DISPATCH_ORDER {
            if waiting.owed == Owed::Channel && context == ContextId::Page {
                continue;
            }
            if context == ContextId::Channel
                && waiting.owed == Owed::Both
                && !waiting.lost
                && !self.has_time()
                && self.machine.host_ref().runs(ContextId::Channel)
            {
                return Ok((bits, true));
            }
            if !self.machine.host_ref().runs(context) {
                continue;
            }
            if about_thing && !self.machine.host_ref().listens_to(context, event.ints[0], event.kind) {
                continue;
            }
            if !self.machine.host().take_in(context, event) {
                continue;
            }
            let mut numbers = event.ints;
            let mut ints_given: Vec<i32> = Vec::new();
            let payload = match &event.payload {
                InputPayload::None => Given::None,
                InputPayload::Text(text) => Given::Text(text),
                InputPayload::Bytes(bytes) => Given::Bytes(bytes),
                InputPayload::Ints(ints) => {
                    ints_given.extend(ints);
                    Given::Ints(&[])
                }
            };
            if context == ContextId::Channel {
                // A thing is the page context's: the channel context is
                // told where and what, never which thing.
                match event.kind {
                    EventKind::POINTER => {
                        numbers[3] = Handle::NT.number();
                        ints_given.truncate(POINTER_PAYLOAD_INTS);
                        if let Some(local) = ints_given.get_mut(2..POINTER_PAYLOAD_INTS) {
                            local.copy_from_slice(&[numbers[1], numbers[2]]);
                        }
                    }
                    EventKind::WHEEL => numbers[2] = Handle::NT.number(),
                    _ => {}
                }
            }
            let payload = if matches!(event.payload, InputPayload::Ints(_)) { Given::Ints(&ints_given) } else { payload };
            if let Some(entry) = self.deliver(context, event.kind, numbers, payload, self.time.callback)? {
                bits |= entry.dispatch & (DispatchBit::CANCELLED.number() | DispatchBit::HANDLED.number());
            }
            self.tell_shared()?;
        }
        if !cancelable(event) {
            bits &= !DispatchBit::CANCELLED.number();
        }
        Ok((bits, false))
    }

    /// The step's first phase: the input events (section 3.2 item 1).
    fn run_input(&mut self, step: &Step, events: Vec<Input>, verdicts: &mut [u8]) -> Result<(), Stop> {
        // Waiting key repeats are dropped: one an earlier step had no time
        // for is not worth more than the repeat that follows it.
        self.queue.retain(|waiting| !(waiting.event.kind == EventKind::KEY_DOWN && waiting.event.ints[0] & KeyBit::REPEAT_OR_LOST.number() != 0));
        for (index, event) in events.into_iter().enumerate() {
            self.enqueue(event, Some((step.sequence, index)));
        }
        // What the decoder reports of things and of sound joins the queue
        // behind the input, as events the host makes.
        let reports = step
            .painted
            .iter()
            .map(|handle| (EventKind::PAINTED, [*handle, 0, 0, 0], InputPayload::None))
            .chain(step.cycle_ended.iter().map(|handle| (EventKind::CYCLE_END, [*handle, 0, 0, 0], InputPayload::None)))
            .chain(step.sound.iter().map(|report| match report {
                SoundReport::Cue { voice, cue } => (EventKind::SOUND_CUE, [i32::from(*voice), *cue, 0, 0], InputPayload::None),
                SoundReport::Ended { voice } => (EventKind::SOUND_ENDED, [i32::from(*voice), 0, 0, 0], InputPayload::None),
            }))
            .chain(step.faults.iter().map(|fault| (EventKind::FAULT, [fault.code, 0, 0, 0], InputPayload::Text(fault.bound.clone()))));
        for (kind, ints, payload) in reports {
            if self.queue.len() < MAX_QUEUED_EVENTS {
                self.queue.push_back(Waiting { event: Input { kind, ints, payload }, origin: None, owed: Owed::Both, lost: false });
            }
        }
        let mut handled = 0_usize;
        while let Some(waiting) = self.queue.front() {
            // Lost releases are delivered first and never held back.
            if !waiting.lost && (!self.has_time() || handled >= MAX_EVENTS_PER_TICK) {
                break;
            }
            let Some(waiting) = self.queue.pop_front() else { break };
            handled = handled.saturating_add(1);
            let (bits, channel_owed) = self.dispatch(&waiting)?;
            if let Some((sequence, index)) = waiting.origin
                && sequence == step.sequence
                && let Some(verdict) = verdicts.get_mut(index)
            {
                *verdict |= u8::try_from(bits).unwrap_or(0);
            }
            if channel_owed {
                self.queue.push_front(Waiting { owed: Owed::Channel, ..waiting });
                break;
            }
        }
        Ok(())
    }

    /// The second phase: network events that arrived (item 2), at most
    /// [`MAX_NETWORK_EVENTS_PER_STEP`]. The shared core has no web API to
    /// give an answer to (operations 40 to 49 and events 30 to 41 are
    /// work package W2's), so each is taken from the queue and dropped,
    /// and the log says so once.
    fn run_network(&mut self) {
        for _ in 0..MAX_NETWORK_EVENTS_PER_STEP {
            if !self.has_time() || self.network.pop_front().is_none() {
                break;
            }
            if !core::mem::replace(&mut self.told_no_network, true) {
                self.machine.host().log_line(LEVEL_WARN, None, "an answer from the web client was dropped: this script library has no web API yet");
            }
        }
    }

    /// The third phase: timers that are due, in order of due tick and then
    /// of creation (item 3). A timer that cannot start stays due.
    fn run_timers(&mut self) -> Result<(), Stop> {
        for context in DISPATCH_ORDER {
            if !self.machine.host_ref().runs(context) {
                continue;
            }
            for id in self.machine.host().due_timers(context) {
                if !self.has_time() {
                    return Ok(());
                }
                // A callback before this one may have cancelled it.
                if !self.machine.host().fire_timer(context, id) {
                    continue;
                }
                self.deliver(context, EventKind::TIMER, [id, 0, 0, 0], Given::None, self.time.callback)?;
                self.tell_shared()?;
            }
        }
        Ok(())
    }

    /// The fourth phase: the tick event, for a context whose manifest
    /// asked for this tick (item 4). One that cannot start is counted in
    /// the next one's `missed`.
    fn run_tick(&mut self, step: &Step) -> Result<(), Stop> {
        for context in DISPATCH_ORDER {
            let host = self.machine.host_ref();
            let divisor = u64::from(host.tick_divisor(context));
            let now = host.now(context);
            if divisor == 0 || !host.runs(context) || now.checked_rem(divisor) != Some(0) {
                continue;
            }
            let Some(owed) = self.missed.get(context.index()).copied() else { continue };
            if !self.has_time() {
                if let Some(missed) = self.missed.get_mut(context.index()) {
                    *missed = owed.saturating_add(1);
                }
                continue;
            }
            let missed = owed.saturating_add(u32::from(step.missed).checked_div(u32::try_from(divisor).unwrap_or(1)).unwrap_or(0));
            if let Some(slot) = self.missed.get_mut(context.index()) {
                *slot = 0;
            }
            let tick = i32::try_from(now & TICK_MASK).unwrap_or(0);
            self.deliver(context, EventKind::TICK, [tick, i32::try_from(missed).unwrap_or(i32::MAX), 0, 0], Given::None, self.time.callback)?;
            self.tell_shared()?;
        }
        Ok(())
    }

    /// Tells both contexts something with one whole number.
    fn tell_both(&mut self, kind: EventKind, first: i32) -> Result<(), Stop> {
        for context in DISPATCH_ORDER {
            self.deliver(context, kind, [first, 0, 0, 0], Given::None, self.time.callback)?;
        }
        Ok(())
    }

    fn run_phases(&mut self, step: &Step, events: Vec<Input>, verdicts: &mut [u8]) -> Result<(), Stop> {
        if self.machine.host_ref().sound_allowed() && !self.told_sound_allowed {
            self.told_sound_allowed = true;
            self.tell_both(EventKind::SOUND_ALLOWED, 1)?;
        }
        if step.flags.resumed {
            self.machine.host().set_hidden(false);
            self.tell_both(EventKind::VISIBILITY, 0)?;
        }
        self.run_input(step, events, verdicts)?;
        self.run_network();
        self.run_timers()?;
        if step.flags.tick {
            self.run_tick(step)?;
        }
        if step.flags.pausing {
            // Told before the pause, then every held note and playing
            // sound is released (section 3.1's table, section 3.5).
            self.machine.host().set_hidden(true);
            self.tell_both(EventKind::VISIBILITY, 1)?;
            for context in DISPATCH_ORDER {
                self.machine.host().release_sound(context);
            }
        }
        Ok(())
    }

    /// Runs one step and gives everything it changed as its batch. A
    /// container stopped during the step still answers it, with every
    /// verdict as it then stood (a default not cancelled happens, section
    /// 4.1) and the stop in its state and log; the next call is refused.
    ///
    /// # Errors
    /// When a field is past its bound, or the container was stopped
    /// before this step.
    pub fn step(&mut self, step: &Step) -> Result<Batch, LinkError> {
        step.check().map_err(|error| LinkError::of(&error))?;
        let events = step.events.iter().map(input_of).collect::<Result<Vec<Input>, LinkError>>()?;
        if let Some(error) = self.stopped_error() {
            return Err(error);
        }
        self.step_used = Duration::ZERO;
        self.machine.host().begin_step(step.tick, step.flags.sound_allowed);
        let mut verdicts = vec![0_u8; step.events.len()];
        // A stop is noted where it happens; the step is answered either way.
        let _ = self.run_phases(step, events, &mut verdicts);
        self.costs.last_step = self.step_used;
        self.costs.longest_step = self.costs.longest_step.max(self.step_used);
        // The budget over a second: this tick's time joins the window.
        let tick = step.tick;
        match self.second.back_mut() {
            Some((at, used)) if *at == tick => *used = used.saturating_add(self.step_used),
            _ => self.second.push_back((tick, self.step_used)),
        }
        while self.second.front().is_some_and(|(at, _)| at.saturating_add(TICKS_PER_SECOND) <= tick) {
            self.second.pop_front();
        }
        let in_second = self.second.iter().fold(Duration::ZERO, |sum, (_, used)| sum.saturating_add(*used));
        let (step_used, time) = (self.step_used, self.time);
        let figures = &mut self.machine.host().figures;
        figures.step_percent_max = figures.step_percent_max.max(percent(step_used, time.step));
        figures.second_percent = percent(in_second, time.second);
        if in_second > time.second && self.machine.stopped().is_none() {
            self.machine.host().log_line(LEVEL_WARN, None, "SECOND_TIME_BUDGET: the scripts used more time in a second than they may");
            self.note_stop(&Stop::Time);
        }
        Ok(self.machine.host().take_batch(step.sequence, &verdicts))
    }

    /// Why each context is stopped, for a `FAILED` message.
    #[must_use]
    pub fn stop_reason(&self) -> Option<FailReason> {
        self.machine.stopped().map(Stop::reason)
    }
}
