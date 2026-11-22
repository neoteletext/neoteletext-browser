//! A decoder's end of the link to its script host (the script API's
//! specification sections 11.1 and 11.2; DEC-7266, TODO-7292): what it
//! writes to open a context, step it and close it, and what each message
//! that comes back answers.
//!
//! The decoder's pipes are shared with the trusted core's own messages
//! (`super::relay`), so nothing here reads or waits: [`HostLink`] writes
//! whole messages to whatever the decoder writes its output to, and is
//! given each message of the link as the decoder reads it from its input.
//! Between the two it holds the link's rules, so that a script host,
//! which is untrusted, can answer only what it was asked:
//!
//! - **Every step and every close is answered by exactly one batch,** in
//!   the order asked. A step's batch carries the step's number; a
//!   close's carries what the one handling of `close` staged. A batch
//!   nobody asked for, or one with another step's number, breaks the
//!   link.
//! - **A broken link is a stopped script** (section 11.2): the scripts'
//!   defaults stay in force, nothing more is written, and what comes
//!   later is dropped. The container is not started again; the decoder
//!   makes a new link only when the viewer reloads (section 3.1).
//! - **The network's messages are not here.** The trusted core relays a
//!   script host's requests to the partition's web client itself; a
//!   decoder never sees them (section 11.3).

use std::collections::VecDeque;
use std::io::Write;

use super::relay::MAX_STEPS_UNANSWERED;
use super::{
    Batch, CONTEXTS, ContextId, ContextOpen, FailReason, FromScriptHost, LINK_VERSION, MAX_SOURCE_PIECE_BYTES, Step, ToScriptHost, check_source_length,
    write_to_host,
};
use crate::ProtocolError;

/// The most closes that wait for their batch beside the steps: each
/// context closed once and opened again while the first close's batch is
/// on its way.
const MAX_CLOSES_UNANSWERED: usize = 2 * CONTEXTS;

/// What a batch answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// The step of this number.
    Step(u32),
    /// The close of this context: the batch is what its one handling of
    /// `close` staged, and has no verdicts.
    Close(ContextId),
}

/// Why the scripts stopped for good.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    /// Why, as the specification numbers it.
    pub reason: FailReason,
    /// What to say in the developer log: the script host's words when it
    /// said them, which are untrusted plain text within the log's bound,
    /// or this side's own.
    pub text: String,
    /// What was asked and never answered, oldest first. Each such step's
    /// events keep their defaults (section 1 principle 5).
    pub unanswered: Vec<Asked>,
}

/// What a message from the script host meant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Received {
    /// The script host is running and speaks this link.
    Ready {
        /// The highest API version it has.
        api_max: u16,
    },
    /// The answer to what was asked.
    Batch {
        /// What it answers.
        asked: Asked,
        /// The batch, read whole and within its bounds.
        batch: Box<Batch>,
    },
    /// The scripts stopped for good, now.
    Stopped(Stopped),
    /// The scripts had stopped already: the message is dropped.
    Dropped,
}

/// Why the link wrote nothing.
#[derive(Debug)]
pub enum Refusal {
    /// The scripts have stopped for good.
    Stopped(FailReason),
    /// The context is already open, or the source is not the length its
    /// opening says.
    Context(&'static str),
    /// A field is out of its bounds, or the pipe could not be written.
    Message(ProtocolError),
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Stopped(reason) => write!(formatter, "the scripts have stopped ({reason:?})"),
            Self::Context(what) => write!(formatter, "the context {what}"),
            Self::Message(error) => write!(formatter, "{error}"),
        }
    }
}

impl core::error::Error for Refusal {}

/// A decoder's end of the link to one script host: one for each
/// container, so one for a (tab, channel) from a load to the next reload.
#[derive(Debug, Default)]
pub struct HostLink {
    /// HELLO has been written: the trusted core starts the script host
    /// when it relays it (section 11.1).
    greeted: bool,
    /// READY has been read.
    ready: bool,
    /// What waits for its batch, oldest first.
    asked: VecDeque<Asked>,
    /// Which contexts are open, as this side asked.
    open: [bool; CONTEXTS],
    stopped: Option<FailReason>,
}

impl HostLink {
    /// A link nothing has been said on.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Why the scripts stopped, once they have.
    #[must_use]
    pub const fn stopped(&self) -> Option<FailReason> {
        self.stopped
    }

    /// Whether `context` is open, as this side asked.
    #[must_use]
    pub fn is_open(&self, context: ContextId) -> bool {
        self.open.get(place(context)).copied().unwrap_or(false)
    }

    /// How many steps and closes wait for their batch.
    #[must_use]
    pub fn unanswered(&self) -> usize {
        self.asked.len()
    }

    /// Whether another step may be asked: fewer than
    /// [`MAX_STEPS_UNANSWERED`] wait. A decoder skips a tick's step
    /// rather than ask past it, and counts the tick as missed.
    #[must_use]
    pub fn has_room(&self) -> bool {
        self.stopped.is_none() && self.steps_asked() < MAX_STEPS_UNANSWERED
    }

    fn steps_asked(&self) -> usize {
        self.asked.iter().filter(|asked| matches!(asked, Asked::Step(_))).count()
    }

    /// Opens a context: writes HELLO when this is the link's first word,
    /// then the opening and `source` in pieces. The script's top level
    /// runs when the last piece arrives; nothing answers it but the next
    /// step's batch, which carries what it logged and the context's
    /// state, or a FAILED.
    ///
    /// # Errors
    /// When the scripts have stopped, the context is open, the source is
    /// not the length the opening says, a field is out of its bounds, or
    /// `out` cannot be written. Nothing is written but in the last case.
    pub fn open(&mut self, out: &mut dyn Write, open: &ContextOpen, source: &[u8]) -> Result<(), Refusal> {
        self.running()?;
        if self.is_open(open.context) {
            return Err(Refusal::Context("is already open"));
        }
        open.check().map_err(Refusal::Message)?;
        if usize::try_from(open.source_bytes).ok() != Some(source.len()) {
            return Err(Refusal::Context("has a source of another length than its opening says"));
        }
        check_source_length(open.source_bytes).map_err(Refusal::Message)?;
        if !self.greeted {
            write_to_host(out, &ToScriptHost::Hello { link_version: LINK_VERSION }).map_err(Refusal::Message)?;
            self.greeted = true;
        }
        write_to_host(out, &ToScriptHost::ContextOpen(Box::new(open.clone()))).map_err(Refusal::Message)?;
        let mut offset = 0_usize;
        for piece in source.chunks(MAX_SOURCE_PIECE_BYTES) {
            let at = u32::try_from(offset).map_err(|_| Refusal::Message(ProtocolError::BadValue("source piece")))?;
            write_to_host(out, &ToScriptHost::Source { context: open.context, offset: at, bytes: piece.to_vec() }).map_err(Refusal::Message)?;
            offset = offset.saturating_add(piece.len());
        }
        if let Some(open) = self.open.get_mut(place(open.context)) {
            *open = true;
        }
        Ok(())
    }

    /// Asks a step. Its batch comes by [`Self::receive`], as
    /// [`Asked::Step`] of its number.
    ///
    /// # Errors
    /// When the scripts have stopped, no context is open,
    /// [`MAX_STEPS_UNANSWERED`] steps already wait, a field is out of its
    /// bounds, or `out` cannot be written.
    pub fn step(&mut self, out: &mut dyn Write, step: &Step) -> Result<(), Refusal> {
        self.running()?;
        if !self.open.contains(&true) {
            return Err(Refusal::Context("is not open"));
        }
        if self.steps_asked() >= MAX_STEPS_UNANSWERED {
            return Err(Refusal::Context("has too many steps unanswered"));
        }
        write_to_host(out, &ToScriptHost::Step(Box::new(step.clone()))).map_err(Refusal::Message)?;
        self.asked.push_back(Asked::Step(step.sequence));
        Ok(())
    }

    /// Closes a context, if it is open. Its batch comes by
    /// [`Self::receive`], as [`Asked::Close`].
    ///
    /// # Errors
    /// When the scripts have stopped or `out` cannot be written.
    pub fn close(&mut self, out: &mut dyn Write, context: ContextId) -> Result<(), Refusal> {
        self.running()?;
        if !self.is_open(context) {
            return Ok(());
        }
        if self.asked.len().saturating_sub(self.steps_asked()) >= MAX_CLOSES_UNANSWERED {
            return Err(Refusal::Context("has too many closes unanswered"));
        }
        write_to_host(out, &ToScriptHost::ContextClose(context)).map_err(Refusal::Message)?;
        self.asked.push_back(Asked::Close(context));
        if let Some(open) = self.open.get_mut(place(context)) {
            *open = false;
        }
        Ok(())
    }

    /// Tells the script host to end: the tab left the channel. Written
    /// even when the scripts have stopped, since the trusted core ends
    /// the process when it relays it.
    ///
    /// # Errors
    /// When `out` cannot be written.
    pub fn shutdown(&mut self, out: &mut dyn Write) -> Result<(), ProtocolError> {
        self.open = [false; CONTEXTS];
        self.asked.clear();
        if !self.greeted {
            return Ok(());
        }
        write_to_host(out, &ToScriptHost::Shutdown)
    }

    /// What `message`, read from the script host, means.
    pub fn receive(&mut self, message: FromScriptHost) -> Received {
        if self.stopped.is_some() {
            return Received::Dropped;
        }
        match message {
            FromScriptHost::Ready { link_version, api_max } if !self.ready && link_version == LINK_VERSION => {
                self.ready = true;
                Received::Ready { api_max }
            }
            FromScriptHost::Ready { .. } => self.fail(FailReason::Link, "the script host said it was ready twice, or speaks another link"),
            FromScriptHost::Batch(batch) => match self.asked.front().copied() {
                _ if !self.ready => self.fail(FailReason::Link, "the script host answered before it was ready"),
                Some(Asked::Step(sequence)) if batch.sequence != sequence => {
                    self.fail(FailReason::Link, "the script host answered another step than the one asked")
                }
                Some(asked) => {
                    self.asked.pop_front();
                    Received::Batch { asked, batch }
                }
                None => self.fail(FailReason::Link, "the script host sent a batch nothing asked for"),
            },
            FromScriptHost::Failed { reason, text, .. } => self.fail(reason, &text),
        }
    }

    /// Stops the scripts for good from this side: the script host's
    /// message was refused by its reader, the trusted core said the
    /// process ended, or the decoder's own deadline passed.
    pub fn fail(&mut self, reason: FailReason, text: &str) -> Received {
        if self.stopped.is_some() {
            return Received::Dropped;
        }
        self.stopped = Some(reason);
        self.open = [false; CONTEXTS];
        Received::Stopped(Stopped { reason, text: text.to_owned(), unanswered: self.asked.drain(..).collect() })
    }

    fn running(&self) -> Result<(), Refusal> {
        self.stopped.map_or(Ok(()), |reason| Err(Refusal::Stopped(reason)))
    }
}

/// A context's place in a table of the two.
const fn place(context: ContextId) -> usize {
    match context {
        ContextId::Channel => 0,
        ContextId::Page => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::super::relay::{HostInput, read_host_input};
    use super::super::tests::open as opening;
    use super::*;

    fn opening_of(context: ContextId, source: &[u8]) -> ContextOpen {
        ContextOpen { context, source_bytes: u32::try_from(source.len()).unwrap(), ..opening() }
    }

    fn written(bytes: &[u8]) -> Vec<ToScriptHost> {
        let mut input = bytes;
        let mut messages = Vec::new();
        while let Some(HostInput::Link(message)) = read_host_input(&mut input).unwrap() {
            messages.push(message);
        }
        messages
    }

    fn ready() -> FromScriptHost {
        FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 }
    }

    fn batch(sequence: u32) -> FromScriptHost {
        FromScriptHost::Batch(Box::new(Batch::new(sequence)))
    }

    /// Section 11.2: HELLO once, then the opening and the source in
    /// pieces of at most `MAX_SOURCE_PIECE_BYTES` that end at its length.
    #[test]
    fn an_opening_is_hello_once_then_the_source_in_pieces() {
        let mut link = HostLink::new();
        let mut out = Vec::new();
        let source = vec![b' '; MAX_SOURCE_PIECE_BYTES + 5];
        link.open(&mut out, &opening_of(ContextId::Channel, &source), &source).unwrap();
        link.open(&mut out, &opening_of(ContextId::Page, b"1"), b"1").unwrap();
        let messages = written(&out);
        assert_eq!(messages.len(), 6);
        assert_eq!(messages[0], ToScriptHost::Hello { link_version: LINK_VERSION });
        assert!(matches!(&messages[1], ToScriptHost::ContextOpen(open) if open.context == ContextId::Channel));
        assert!(matches!(&messages[2], ToScriptHost::Source { offset: 0, bytes, .. } if bytes.len() == MAX_SOURCE_PIECE_BYTES));
        assert!(matches!(&messages[3], ToScriptHost::Source { offset, bytes, .. } if *offset as usize == MAX_SOURCE_PIECE_BYTES && bytes.len() == 5));
        assert!(matches!(&messages[4], ToScriptHost::ContextOpen(open) if open.context == ContextId::Page));
        assert!(matches!(link.open(&mut out, &opening_of(ContextId::Page, b"1"), b"1"), Err(Refusal::Context(_))), "open twice");
        assert!(matches!(link.open(&mut Vec::new(), &opening_of(ContextId::Page, b"1"), b"12"), Err(Refusal::Context(_))));
    }

    /// Section 11.2 and TODO-7292: each step and each close is answered
    /// by one batch in the order asked, and a batch with another number
    /// or nothing asked for stops the scripts, with what went unanswered.
    #[test]
    fn a_batch_answers_what_was_asked_in_order_or_breaks_the_link() {
        let mut link = HostLink::new();
        let mut out = Vec::new();
        assert!(matches!(link.step(&mut out, &Step::default()), Err(Refusal::Context(_))), "a step with no context");
        link.open(&mut out, &opening_of(ContextId::Page, b"1"), b"1").unwrap();
        assert_eq!(link.receive(ready()), Received::Ready { api_max: 1 });
        link.step(&mut out, &Step { sequence: 4, ..Step::default() }).unwrap();
        link.close(&mut out, ContextId::Page).unwrap();
        assert_eq!(link.unanswered(), 2);
        assert_eq!(link.receive(batch(4)), Received::Batch { asked: Asked::Step(4), batch: Box::new(Batch::new(4)) });
        assert_eq!(link.receive(batch(4)), Received::Batch { asked: Asked::Close(ContextId::Page), batch: Box::new(Batch::new(4)) });
        let Received::Stopped(stopped) = link.receive(batch(5)) else { panic!("an unasked batch was taken") };
        assert_eq!((stopped.reason, stopped.unanswered.len()), (FailReason::Link, 0));
        assert_eq!(link.receive(batch(5)), Received::Dropped);
        assert!(matches!(link.step(&mut out, &Step::default()), Err(Refusal::Stopped(FailReason::Link))));

        let mut link = HostLink::new();
        link.open(&mut out, &opening_of(ContextId::Page, b"1"), b"1").unwrap();
        link.receive(ready());
        link.step(&mut out, &Step { sequence: 1, ..Step::default() }).unwrap();
        link.step(&mut out, &Step { sequence: 2, ..Step::default() }).unwrap();
        let Received::Stopped(stopped) = link.receive(batch(2)) else { panic!("a batch out of order was taken") };
        assert_eq!(stopped.unanswered, vec![Asked::Step(1), Asked::Step(2)]);
    }

    /// Section 11.2: a FAILED stops every context with the script host's
    /// reason; a second READY, another link's version and an answer
    /// before READY each break the link; at most `MAX_STEPS_UNANSWERED`
    /// steps wait.
    #[test]
    fn a_failure_or_a_broken_greeting_stops_the_scripts() {
        let mut out = Vec::new();
        let mut link = HostLink::new();
        link.open(&mut out, &opening_of(ContextId::Channel, b"1"), b"1").unwrap();
        link.receive(ready());
        link.step(&mut out, &Step { sequence: 9, ..Step::default() }).unwrap();
        let failed = FromScriptHost::Failed { context: ContextId::Channel, reason: FailReason::Time, text: "too long".into() };
        assert_eq!(link.receive(failed), Received::Stopped(Stopped { reason: FailReason::Time, text: "too long".into(), unanswered: vec![Asked::Step(9)] }));
        assert_eq!(link.stopped(), Some(FailReason::Time));
        assert!(!link.is_open(ContextId::Channel));

        for greeting in [vec![ready(), ready()], vec![FromScriptHost::Ready { link_version: LINK_VERSION + 1, api_max: 1 }], vec![batch(0)]] {
            let mut link = HostLink::new();
            let last = greeting.into_iter().map(|message| link.receive(message)).last();
            assert!(matches!(last, Some(Received::Stopped(Stopped { reason: FailReason::Link, .. }))));
        }

        let mut link = HostLink::new();
        link.open(&mut out, &opening_of(ContextId::Channel, b"1"), b"1").unwrap();
        for sequence in 0..u32::try_from(MAX_STEPS_UNANSWERED).unwrap() {
            assert!(link.has_room());
            link.step(&mut out, &Step { sequence, ..Step::default() }).unwrap();
        }
        assert!(!link.has_room());
        assert!(matches!(link.step(&mut out, &Step::default()), Err(Refusal::Context(_))));
    }

    /// Section 11.1: a link that never opened a context has no script
    /// host to end, and one that did tells it to end even once stopped.
    #[test]
    fn shutdown_is_written_only_where_a_script_host_was_asked_for() {
        let mut out = Vec::new();
        HostLink::new().shutdown(&mut out).unwrap();
        assert_eq!(out, [] as [u8; 0]);
        let mut link = HostLink::new();
        link.open(&mut out, &opening_of(ContextId::Page, b"1"), b"1").unwrap();
        link.fail(FailReason::Time, "late");
        out.clear();
        link.shutdown(&mut out).unwrap();
        assert_eq!(written(&out), vec![ToScriptHost::Shutdown]);
    }
}
