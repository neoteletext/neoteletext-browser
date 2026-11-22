//! The script host's work (`neoteletext-common/docs/script-api.md`
//! section 11.1; DEC-7266, TODO-7292): it serves one (tab, channel)'s
//! scripts on a pair of pipes. On its input come the script link's
//! messages from the decoder and the web client's answers; on its output
//! go the script link's messages for the decoder and the scripts'
//! requests of the web client. The trusted core relays all of them, by
//! kind and length alone.
//!
//! What runs the scripts is the script library, unchanged: this crate
//! adds the messages around its [`Container`] and nothing a script can
//! reach. The program (`main.rs`) confines the process, gives every
//! container a watchdog that ends the process, and calls [`serve`].
//!
//! The rules of the pipe, which the decoder's end
//! (`protocol::script::link::HostLink`) holds from its side:
//!
//! - READY is said first, before anything is read.
//! - A context starts when the last piece of its source arrives. Nothing
//!   answers an opening: what its top level logged leaves in the next
//!   batch.
//! - **Every STEP and every `CONTEXT_CLOSE` is answered by exactly one
//!   BATCH**, in order, so that the trusted core can tell by counting
//!   kinds whether the script host owes an answer. A close's batch is what the one
//!   handling of `close` staged (section 3.1), which would otherwise be
//!   lost when the last context goes; it carries the number of the last
//!   step answered.
//! - A container that stops says FAILED for each context it held, and
//!   the process ends: it is never entered again (section 3.1).
//! - Anything malformed, out of its bounds or out of order ends the
//!   process, with a FAILED whose reason is the link.
//!
//! Everything read is untrusted: the decoder parses a hostile channel,
//! and the web client a hostile server.

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::io::{Read, Write};

use neoteletext_script::limits::MAX_SCRIPT_BYTES;
use neoteletext_script::{API_VERSION, Container, Engine, LinkError, Settings};
use protocol::ProtocolError;
use protocol::script::relay::{HostInput, read_host_input};
use protocol::script::{
    Batch, CONTEXTS, ContextId, ContextOpen, FailReason, FromScriptHost, LINK_VERSION, MAX_LOG_CHARS, SEED_BYTES, Step, ToScriptHost, write_to_decoder,
};
use protocol::web::write_request;

// The operating system's limit on the process is the figure the script
// library names (section 10.6); the confinement crate cannot name the
// library's, so the two are held equal here, where both are in reach.
const _: () = assert!(confine::SCRIPT_MEMORY_BYTES == neoteletext_script::limits::SCRIPT_PROCESS_MEMORY_BYTES);
// The bounds after which the trusted core takes a silent script host for
// hung are a backstop behind the library's own limits, never in their
// place (sections 3.2 and 11.1; DEC-7357 as revised): each is at least
// ten times everything the library allows before the answer it waits
// for.
const _: () = assert!(
    10 * (neoteletext_script::limits::CONTAINER_START_LIMIT.as_millis()
        + neoteletext_script::limits::START_TIME_LIMIT.as_millis()
        + neoteletext_script::limits::WATCHDOG_GRACE.as_millis())
        <= protocol::script::relay::START_HUNG_AFTER.as_millis()
);
const _: () = assert!(
    10 * (neoteletext_script::limits::STEP_TIME_BUDGET.as_millis()
        + neoteletext_script::limits::CALLBACK_TIME_LIMIT.as_millis()
        + neoteletext_script::limits::WATCHDOG_GRACE.as_millis())
        <= protocol::script::relay::STEP_HUNG_AFTER.as_millis()
);
// A callback at its limit, and the watchdog's grace after it, are within
// the time a step's answer is waited for on a machine with a processor
// free.
const _: () = assert!(
    neoteletext_script::limits::CALLBACK_TIME_LIMIT.as_millis() + neoteletext_script::limits::WATCHDOG_GRACE.as_millis()
        < protocol::script::relay::STEP_DEADLINE.as_millis()
);
// A script's source is collected whole before its context opens.
const _: () = assert!(MAX_SCRIPT_BYTES == protocol::script::MAX_SCRIPT_BYTES);

/// Fills a seed with bytes the operating system drew; false when it
/// could not.
pub type Entropy<'a> = &'a mut dyn FnMut(&mut [u8; SEED_BYTES]) -> bool;

/// How serving ended. The process ends with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// The decoder said SHUTDOWN, or the input ended between messages:
    /// the tab left the channel.
    Told,
    /// The container stopped for good, and FAILED was said.
    Stopped(FailReason),
    /// The link broke: what was read was malformed, out of its bounds or
    /// out of order, or the output could not be written.
    Link(String),
}

/// A context whose opening has come and whose source is still arriving.
struct Arriving {
    open: ContextOpen,
    source: Vec<u8>,
}

/// One (tab, channel)'s scripts, served on a pair of pipes.
struct Serving<'a> {
    engine: &'a Engine,
    settings: &'a Settings,
    entropy: Entropy<'a>,
    output: &'a mut dyn Write,
    /// The container, from the first context's opening until the last
    /// has closed.
    container: Option<Container>,
    /// The contexts whose source is arriving, by their place.
    arriving: [Option<Arriving>; CONTEXTS],
    /// HELLO has been read.
    greeted: bool,
    /// The number of the last step answered, which a close's batch
    /// carries.
    last_sequence: u32,
}

/// What handling one message decided.
enum Next {
    /// Read the next.
    Go,
    /// End the process.
    End(End),
}

/// A context's place in a table of the two.
const fn place(context: ContextId) -> usize {
    match context {
        ContextId::Channel => 0,
        ContextId::Page => 1,
    }
}

/// `text` held to a line of the developer log: what a FAILED may carry.
fn log_line(text: &str) -> String {
    text.chars().filter(|character| !character.is_control()).take(MAX_LOG_CHARS).collect()
}

/// Makes a context's seed fresh: the bytes the decoder gave, which come
/// from a process that parses a hostile channel, are combined with bytes
/// this process draws from the operating system, so that a context's
/// random numbers start from something no channel chose or saw
/// (section 13 item 7; TODO-7279 item 3; TODO-7338 item 4).
fn freshen(seed: &mut [u8; SEED_BYTES], entropy: Entropy<'_>) -> bool {
    let mut drawn = [0_u8; SEED_BYTES];
    if !entropy(&mut drawn) {
        return false;
    }
    for (byte, fresh) in seed.iter_mut().zip(drawn) {
        *byte ^= fresh;
    }
    true
}

impl Serving<'_> {
    fn say(&mut self, message: &FromScriptHost) -> Result<(), ProtocolError> {
        write_to_decoder(self.output, message)
    }

    /// Says FAILED for `context`, as well as it can be said.
    fn failed(&mut self, context: ContextId, reason: FailReason, text: &str) {
        let _ = self.say(&FromScriptHost::Failed { context, reason, text: log_line(text) });
        let _ = self.output.flush();
    }

    /// The link broke: FAILED with the link as its reason, and the end.
    fn broke(&mut self, text: &str) -> Next {
        self.failed(ContextId::Channel, FailReason::Link, text);
        Next::End(End::Link(text.to_owned()))
    }

    /// The container stopped for good: FAILED for each context it held,
    /// and the end. `also` is a context that was being opened.
    fn stopped(&mut self, reason: FailReason, text: &str, also: Option<ContextId>) -> Next {
        for &context in ContextId::ALL {
            if also == Some(context) || self.container.as_ref().is_some_and(|container| container.is_open(context)) {
                self.failed(context, reason, text);
            }
        }
        self.container = None;
        Next::End(End::Stopped(reason))
    }

    /// Ends after `error` of the container: a stop, or a message the
    /// library refused as its reader would.
    fn refused(&mut self, error: &LinkError, opening: Option<ContextId>) -> Next {
        match error {
            LinkError::Stopped(reason, text) => self.stopped(*reason, text, opening),
            other => self.broke(&other.to_string()),
        }
    }

    fn handle(&mut self, message: HostInput) -> Next {
        match message {
            HostInput::Link(ToScriptHost::Hello { link_version }) => {
                if self.greeted || link_version != LINK_VERSION {
                    return self.broke("the decoder greeted twice, or speaks another link");
                }
                self.greeted = true;
                Next::Go
            }
            HostInput::Link(_) if !self.greeted => self.broke("the decoder spoke before its greeting"),
            HostInput::Link(ToScriptHost::ContextOpen(open)) => self.opening(*open),
            HostInput::Link(ToScriptHost::Source { context, offset, bytes }) => self.source(context, offset, &bytes),
            HostInput::Link(ToScriptHost::Step(step)) => self.step(&step),
            HostInput::Link(ToScriptHost::ContextClose(context)) => self.close(context),
            HostInput::Link(ToScriptHost::Shutdown) => Next::End(End::Told),
            HostInput::Answer(answer) => {
                // An answer with no container to hear it is dropped: its
                // request's context has closed.
                if let Some(container) = self.container.as_mut() {
                    container.network(answer);
                }
                Next::Go
            }
        }
    }

    fn opening(&mut self, mut open: ContextOpen) -> Next {
        let context = open.context;
        let slot = place(context);
        if self.arriving.get(slot).is_some_and(Option::is_some) || self.container.as_ref().is_some_and(|container| container.is_open(context)) {
            return self.broke("a context was opened twice");
        }
        if !freshen(&mut open.seed, &mut *self.entropy) {
            return self.broke("the operating system gave no random bytes for a context's seed");
        }
        // The reader held the length to `MAX_SCRIPT_BYTES`.
        let Ok(length) = usize::try_from(open.source_bytes) else { return self.broke("source length") };
        if let Some(arriving) = self.arriving.get_mut(slot) {
            *arriving = Some(Arriving { open, source: Vec::with_capacity(length.min(MAX_SCRIPT_BYTES)) });
        }
        Next::Go
    }

    fn source(&mut self, context: ContextId, offset: u32, bytes: &[u8]) -> Next {
        let slot = place(context);
        let Some(arriving) = self.arriving.get_mut(slot).and_then(Option::as_mut) else {
            return self.broke("a source piece for a context that is not opening");
        };
        let whole = usize::try_from(arriving.open.source_bytes).unwrap_or(usize::MAX);
        let end = arriving.source.len().saturating_add(bytes.len());
        if usize::try_from(offset).ok() != Some(arriving.source.len()) || end > whole {
            return self.broke("a source piece out of its place");
        }
        arriving.source.extend_from_slice(bytes);
        if end < whole {
            return Next::Go;
        }
        let Some(Arriving { open, source }) = self.arriving.get_mut(slot).and_then(Option::take) else { return Next::Go };
        if self.container.is_none() {
            match Container::new(self.engine, self.settings, open.zone_minutes) {
                Ok(container) => self.container = Some(container),
                Err(stop) => return self.stopped(stop.reason(), &stop.to_string(), Some(context)),
            }
        }
        let Some(container) = self.container.as_mut() else { return Next::Go };
        match container.open(open, &source) {
            Ok(()) => Next::Go,
            Err(error) => self.refused(&error, Some(context)),
        }
    }

    /// Writes a batch and what the scripts asked of the network since
    /// the last.
    fn answer(&mut self, batch: Batch) -> Result<(), ProtocolError> {
        self.say(&FromScriptHost::Batch(Box::new(batch)))?;
        let requests = self.container.as_mut().map(Container::take_requests).unwrap_or_default();
        for request in &requests {
            write_request(self.output, request)?;
        }
        Ok(())
    }

    fn step(&mut self, step: &Step) -> Next {
        self.last_sequence = step.sequence;
        // A step with no context open is answered with nothing: the
        // decoder's last close and this step crossed.
        let batch = match self.container.as_mut() {
            Some(container) => match container.step(step) {
                Ok(batch) => batch,
                Err(error) => return self.refused(&error, None),
            },
            None => Batch::new(step.sequence),
        };
        if let Err(error) = self.answer(batch) {
            return Next::End(End::Link(error.to_string()));
        }
        // A container stopped during the step answered it, with the stop
        // in its batch's state; it is not entered again.
        match self.container.as_ref().and_then(|container| container.stopped().map(|stop| (stop.reason(), stop.to_string()))) {
            Some((reason, text)) => self.stopped(reason, &text, None),
            None => Next::Go,
        }
    }

    fn close(&mut self, context: ContextId) -> Next {
        // An opening still arriving is dropped with its source.
        if let Some(arriving) = self.arriving.get_mut(place(context)) {
            *arriving = None;
        }
        let sequence = self.last_sequence;
        let batch = match self.container.as_mut() {
            Some(container) => {
                container.close(context);
                container.drain(sequence)
            }
            None => Batch::new(sequence),
        };
        if let Err(error) = self.answer(batch) {
            return Next::End(End::Link(error.to_string()));
        }
        let Some(container) = self.container.as_ref() else { return Next::Go };
        if let Some((reason, text)) = container.stopped().map(|stop| (stop.reason(), stop.to_string())) {
            return self.stopped(reason, &text, None);
        }
        // The last context gone, the container goes with it (section
        // 3.1); the next opening makes another.
        if !ContextId::ALL.iter().any(|&context| container.is_open(context)) {
            self.container = None;
        }
        Next::Go
    }
}

/// Serves one (tab, channel)'s scripts: says READY on `output`, then
/// handles each message of `input` until the decoder says to end, the
/// input ends, the container stops for good or the link breaks.
/// Containers are made in `engine` with `settings`, and each context's
/// seed is made fresh with `entropy`. `output` is flushed after each
/// message handled, so that a batch leaves when its step is done.
pub fn serve(input: &mut dyn Read, output: &mut dyn Write, engine: &Engine, settings: &Settings, entropy: Entropy<'_>) -> End {
    let mut serving = Serving { engine, settings, entropy, output, container: None, arriving: [const { None }; CONTEXTS], greeted: false, last_sequence: 0 };
    if let Err(error) = serving.say(&FromScriptHost::Ready { link_version: LINK_VERSION, api_max: API_VERSION }).and_then(|()| Ok(serving.output.flush()?)) {
        return End::Link(error.to_string());
    }
    loop {
        let message = match read_host_input(input) {
            Ok(Some(message)) => message,
            Ok(None) => return End::Told,
            Err(error) => {
                let text = format!("the script host refused what it was given: {error}");
                let Next::End(end) = serving.broke(&text) else { return End::Link(text) };
                return end;
            }
        };
        let next = serving.handle(message);
        if let Err(error) = serving.output.flush() {
            return End::Link(error.to_string());
        }
        if let Next::End(end) = next {
            return end;
        }
    }
}
