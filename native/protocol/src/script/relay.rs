//! The pipes the script link shares with other messages, and what the
//! trusted core needs to relay it (the script API's specification
//! sections 11.1 and 11.2; DEC-7266, TODO-7292).
//!
//! A script host has one pair of pipes, to the trusted core. So has a
//! decoder. The script link's messages and the web client's therefore
//! travel among others, and each reader here tells them apart by the kind
//! byte alone:
//!
//! - **A decoder's output** is its own messages to the trusted core and
//!   the script link's messages for its script host
//!   ([`read_decoder_output`]).
//! - **A script host's output** is the script link's messages for its
//!   decoder and its requests of the web client ([`read_host_output`]).
//! - **A script host's input** is the script link's messages from its
//!   decoder and the web client's answers ([`read_host_input`]).
//! - **A decoder's input** is the trusted core's messages and the script
//!   link's messages from its script host ([`read_decoder_input`]).
//!
//! The trusted core reads the first two, and reads no payload of the
//! script link's or the web client's: a [`Relayed`] is a kind, a length
//! the framing held to its bound, and bytes passed on as they stand. A
//! kind that the pipe's writer may not send is refused, and the caller
//! ends that process.

use std::io::{self, Read};
use std::time::Duration;

use crate::decoder::{FromDecoder, ToDecoder, read_from_core, read_from_decoder};
use crate::framing::HEADER_BYTES;
use crate::web::{FromWebClient, is_answer_kind, is_request_kind, read_answer};
use crate::{MAX_PAYLOAD, ProtocolError};

use super::{
    FromScriptHost, KIND_BATCH, KIND_CONTEXT_CLOSE, KIND_CONTEXT_OPEN, KIND_FAILED, KIND_HELLO, KIND_READY, KIND_SHUTDOWN, KIND_SOURCE, KIND_STEP,
    ToScriptHost, is_to_decoder_kind, is_to_host_kind, read_to_decoder, read_to_host,
};

/// How long a step's answer is waited for before the page goes on without
/// it (the specification's `STEP_DEADLINE`, section 10.6): twenty times
/// the step's own budget of 12 ms and five times the longest callback.
/// The trusted core takes a key's default no later than twice this
/// whatever the script host does. It ends no script host: on 23 November
/// 2026 it was measured that a script host given no processor, or one in
/// an unoptimised build that needs a second of its own work to start,
/// passes any wall-clock time of this size without a fault of its own
/// (DEC-7357 as revised; [`Silence`]).
pub const STEP_DEADLINE: Duration = Duration::from_millis(250);

/// How long a script host that owes an answer may say nothing before it is
/// taken for hung and ended: counted on the wall clock from the oldest
/// thing asked of it, or from its last answer when that is later. It is
/// a backstop and nothing else. A script that runs too long is stopped
/// by the script host itself, which counts its limits on its own
/// processor time and says so within them; this bound is for a script
/// host that can no longer say anything. It is forty times
/// [`STEP_DEADLINE`]: the slowest honest answer is a step at its budget
/// and a callback at its limit, about 60 ms of the script host's work, so
/// a script host that takes this long was given less than one part in a
/// hundred and sixty of a processor, or is hung.
pub const STEP_HUNG_AFTER: Duration = Duration::from_secs(10);

/// As [`STEP_HUNG_AFTER`], from the last piece of an opening until the
/// first answer to something asked after it: the script host's process
/// starts, compiles the engine's module and the prelude, starts a
/// container and runs a script's top level before that answer. Measured
/// on 23 November 2026 on twenty processors at a load of 22 to 35: 0.28
/// seconds of its own work in an optimised build and 0.99 in an
/// unoptimised one when alone, 1.8 with eight at once, and with thirty-two
/// at once each had been given 0.7 to 0.9 seconds of work in 2.5 seconds.
/// Thirty seconds is some ten times the longest start that reasoning
/// gives for that machine, and still an end for a viewer who is waiting.
pub const START_HUNG_AFTER: Duration = Duration::from_secs(30);

/// What a script host owes and since when: the one rule by which the
/// trusted core, which ends the process, and the decoder, which stops
/// drawing for it, take a script host for hung.
#[derive(Clone, Debug, Default)]
pub struct Silence {
    /// When each thing asked and not yet answered was asked, oldest first.
    asked: std::collections::VecDeque<std::time::Instant>,
    /// When the last piece of an opening was passed on, until something
    /// asked after it is answered.
    opening: Option<std::time::Instant>,
    /// When the script host last answered.
    heard: Option<std::time::Instant>,
}

impl Silence {
    /// A step or a close was asked at `now`.
    pub fn asked(&mut self, now: std::time::Instant) {
        self.asked.push_back(now);
    }

    /// A piece of an opening was passed on at `now`.
    pub const fn opening(&mut self, now: std::time::Instant) {
        self.opening = Some(now);
    }

    /// The oldest thing asked was answered at `now`. A script host takes
    /// what it is given in order, so an answer to something asked after
    /// an opening says the opening is done.
    pub fn answered(&mut self, now: std::time::Instant) {
        let answered = self.asked.pop_front();
        self.heard = Some(now);
        if let (Some(asked), Some(opening)) = (answered, self.opening)
            && asked >= opening
        {
            self.opening = None;
        }
    }

    /// How many things asked wait for their answer.
    #[must_use]
    pub fn owed(&self) -> usize {
        self.asked.len()
    }

    /// Nothing is owed any more: the script host was ended or told to end.
    pub fn forget(&mut self) {
        *self = Self::default();
    }

    /// When the script host is taken for hung, if it owes anything.
    #[must_use]
    pub fn hung_at(&self) -> Option<std::time::Instant> {
        let oldest = *self.asked.front()?;
        let silent_since = self.heard.map_or(oldest, |heard| heard.max(oldest));
        let step = silent_since.checked_add(STEP_HUNG_AFTER)?;
        let start = self.opening.and_then(|opening| opening.checked_add(START_HUNG_AFTER));
        Some(start.map_or(step, |start| start.max(step)))
    }
}

/// The most steps a script host may be behind by before the trusted core
/// ends it: a second of the page's ticks. A decoder sends a step for each
/// tick and for input between ticks, and a host that has not answered so
/// many has stopped, whatever the clock says.
pub const MAX_STEPS_UNANSWERED: usize = neoteletext_rules::limits::TICKS_PER_SECOND as usize;

/// What a message of the script link is, as far as its kind says: all the
/// trusted core knows of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// The decoder's first word: a script is to run.
    Hello,
    /// A context opens.
    ContextOpen,
    /// A piece of a script's source.
    Source,
    /// A step, which a batch answers.
    Step,
    /// A context closes.
    ContextClose,
    /// The script host is to end.
    Shutdown,
    /// The script host's first word.
    Ready,
    /// A step's answer.
    Batch,
    /// A context stopped for good.
    Failed,
}

impl LinkKind {
    /// What a message of `kind` is; none for a kind that is not the
    /// script link's.
    #[must_use]
    pub const fn of(kind: u8) -> Option<Self> {
        Some(match kind {
            KIND_HELLO => Self::Hello,
            KIND_CONTEXT_OPEN => Self::ContextOpen,
            KIND_SOURCE => Self::Source,
            KIND_STEP => Self::Step,
            KIND_CONTEXT_CLOSE => Self::ContextClose,
            KIND_SHUTDOWN => Self::Shutdown,
            KIND_READY => Self::Ready,
            KIND_BATCH => Self::Batch,
            KIND_FAILED => Self::Failed,
            _ => return None,
        })
    }
}

/// A message passed on unread: its kind, and the whole of it as it was
/// written, the kind byte and the length before the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relayed {
    kind: u8,
    frame: Vec<u8>,
}

impl Relayed {
    /// The message's kind byte.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        self.kind
    }

    /// What the message is, when it is the script link's.
    #[must_use]
    pub const fn link_kind(&self) -> Option<LinkKind> {
        LinkKind::of(self.kind)
    }

    /// The message as its reader takes it: to be written to the pipe it
    /// is for, as it stands.
    #[must_use]
    pub fn frame(&self) -> &[u8] {
        &self.frame
    }

    /// The message's bytes, given up.
    #[must_use]
    pub fn into_frame(self) -> Vec<u8> {
        self.frame
    }
}

/// The first byte of the next message, its kind, or `None` at a clean end.
fn read_kind(input: &mut dyn Read) -> Result<Option<u8>, ProtocolError> {
    let mut kind = [0_u8; 1];
    loop {
        match input.read(&mut kind) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(kind[0])),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// The rest of a message whose kind was read, unread: its length, held to
/// the framing's bound before any room is made for it, and its payload.
fn read_unread(kind: u8, input: &mut dyn Read) -> Result<Relayed, ProtocolError> {
    let mut length = [0_u8; HEADER_BYTES - 1];
    input.read_exact(&mut length)?;
    let payload = u32::from_be_bytes(length) as usize;
    if payload > MAX_PAYLOAD {
        return Err(ProtocolError::TooLong(payload));
    }
    let mut frame = Vec::with_capacity(HEADER_BYTES.saturating_add(payload));
    frame.push(kind);
    frame.extend_from_slice(&length);
    input.take(u64::try_from(payload).map_err(|_| ProtocolError::TooLong(payload))?).read_to_end(&mut frame)?;
    if frame.len() != HEADER_BYTES.saturating_add(payload) {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }
    Ok(Relayed { kind, frame })
}

/// The pipe with the kind byte that was read put back before it, for a
/// reader that takes a whole message.
fn whole<'a>(kind: &'a [u8; 1], input: &'a mut dyn Read) -> impl Read + 'a {
    kind.as_slice().chain(input)
}

/// What a decoder wrote: untrusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecoderOutput {
    /// A message for the trusted core, read whole.
    Core(FromDecoder),
    /// A message of the script link for the decoder's script host, its
    /// payload unread.
    ToHost(Relayed),
}

/// Reads one message a decoder wrote, as the trusted core reads it, or
/// `None` at a clean end.
///
/// # Errors
/// On input errors, a length no message has, and whatever
/// [`read_from_decoder`] refuses of the decoder's own messages.
pub fn read_decoder_output(input: &mut dyn Read) -> Result<Option<DecoderOutput>, ProtocolError> {
    let Some(kind) = read_kind(input)? else { return Ok(None) };
    if is_to_host_kind(kind) {
        return read_unread(kind, input).map(|relayed| Some(DecoderOutput::ToHost(relayed)));
    }
    read_from_decoder(&mut whole(&[kind], input))?.map(DecoderOutput::Core).map(Some).ok_or_else(crate::fetcher::cut_short)
}

/// What a script host wrote: untrusted, and never read past its kind and
/// its length by the trusted core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostOutput {
    /// A message of the script link for the script host's decoder.
    ToDecoder(Relayed),
    /// A request of the web client, in the host form
    /// (`crate::web::relay_to_client` takes its bytes).
    Request(Relayed),
}

/// Reads one message a script host wrote, as the trusted core reads it,
/// or `None` at a clean end.
///
/// # Errors
/// On input errors, a length no message has, and
/// [`ProtocolError::UnknownKind`] at a kind a script host may not write:
/// every other pipe's kinds, the decoder's side of the script link, and
/// the web client's messages that are the trusted core's alone.
pub fn read_host_output(input: &mut dyn Read) -> Result<Option<HostOutput>, ProtocolError> {
    let Some(kind) = read_kind(input)? else { return Ok(None) };
    if is_to_decoder_kind(kind) {
        read_unread(kind, input).map(|relayed| Some(HostOutput::ToDecoder(relayed)))
    } else if is_request_kind(kind) {
        read_unread(kind, input).map(|relayed| Some(HostOutput::Request(relayed)))
    } else {
        Err(ProtocolError::UnknownKind(kind))
    }
}

/// What a script host is given: untrusted, each read whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostInput {
    /// A message of the script link from its decoder.
    Link(ToScriptHost),
    /// An answer from the web client.
    Answer(FromWebClient),
}

/// Reads one message as a script host reads it, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind a script host is never given, and any payload
/// that is malformed or out of its bounds. The script host then ends.
pub fn read_host_input(input: &mut dyn Read) -> Result<Option<HostInput>, ProtocolError> {
    let Some(kind) = read_kind(input)? else { return Ok(None) };
    let first = [kind];
    let mut input = whole(&first, input);
    if is_answer_kind(kind) {
        read_answer(&mut input)?.map(HostInput::Answer).map(Some).ok_or_else(crate::fetcher::cut_short)
    } else {
        // Every other kind is the script link's to accept or refuse.
        read_to_host(&mut input)?.map(HostInput::Link).map(Some).ok_or_else(crate::fetcher::cut_short)
    }
}

/// What a decoder is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecoderInput {
    /// A message from the trusted core.
    Core(ToDecoder),
    /// A message of the script link from the decoder's script host:
    /// untrusted, read whole and held to its bounds.
    Link(FromScriptHost),
}

/// Reads one message as a decoder reads it, or `None` at a clean end.
///
/// # Errors
/// On input errors, an unknown kind, and any payload that is malformed
/// or out of its bounds. A message of the script link that is refused is
/// the script host's fault, not the trusted core's: the caller treats
/// the script as stopped ([`DecoderInputError::Link`]) and goes on.
pub fn read_decoder_input(input: &mut dyn Read) -> Result<Option<DecoderInput>, DecoderInputError> {
    let Some(kind) = read_kind(input).map_err(DecoderInputError::Pipe)? else { return Ok(None) };
    if is_to_decoder_kind(kind) {
        // Taken off the pipe whole before it is read, so that a refused
        // message leaves the pipe at the next message's start.
        let relayed = read_unread(kind, input).map_err(DecoderInputError::Pipe)?;
        return match read_to_decoder(&mut relayed.frame()) {
            Ok(Some(message)) => Ok(Some(DecoderInput::Link(message))),
            Ok(None) => Err(DecoderInputError::Link(crate::fetcher::cut_short())),
            Err(error) => Err(DecoderInputError::Link(error)),
        };
    }
    match read_from_core(&mut whole(&[kind], input)) {
        Ok(Some(message)) => Ok(Some(DecoderInput::Core(message))),
        Ok(None) => Err(DecoderInputError::Pipe(crate::fetcher::cut_short())),
        Err(error) => Err(DecoderInputError::Pipe(error)),
    }
}

/// Why a decoder's input could not be read.
#[derive(Debug)]
pub enum DecoderInputError {
    /// The pipe broke, or the trusted core's own message was malformed:
    /// the decoder ends, as it does today.
    Pipe(ProtocolError),
    /// A message of the script link was malformed or out of its bounds.
    /// The pipe stands at the next message; the script is stopped (the
    /// specification's section 11.2: "the decoder treats the script as
    /// stopped").
    Link(ProtocolError),
}

impl core::fmt::Display for DecoderInputError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Pipe(error) => write!(formatter, "{error}"),
            Self::Link(error) => write!(formatter, "the script host's message was refused: {error}"),
        }
    }
}

impl core::error::Error for DecoderInputError {}

#[cfg(test)]
mod tests;
