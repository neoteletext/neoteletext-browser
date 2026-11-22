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

/// How long a script host has to answer a step with its batch before the
/// trusted core ends it (the specification's `STEP_DEADLINE`, sections 3.2
/// rule 5(b) and 10.6): twenty times the step's own budget of 12 ms and
/// five times the longest callback, so that a host that is merely slow on
/// a loaded machine is not ended, and far less than a viewer waits for a
/// page that has stopped.
pub const STEP_DEADLINE: Duration = Duration::from_millis(250);

/// How long a script host has from the last piece of an opening the
/// trusted core relayed to it until it must have answered what waits:
/// its process's start, a container's start (at most a second), a
/// script's top level (at most `START_TIME_LIMIT`, a second), its
/// watchdog's grace, and [`STEP_DEADLINE`] after them. The script-host
/// program asserts that this covers the script library's limits, which
/// this crate cannot name.
pub const START_DEADLINE: Duration = Duration::from_millis(2_500);

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
