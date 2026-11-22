//! [`ScriptLink`] over a script-host process (script-api.md section
//! 11.1; work package F3, TODO-7292): the same interface as
//! [`crate::InProcess`], speaking the messages of section 11.2 on a pair
//! of pipes to the program `neoteletext-script-host`, so that what drives
//! a (tab, channel)'s scripts is the same code whether they run in a
//! process of their own or, in the standalone runner, in its own.
//!
//! The link's rules are the protocol crate's
//! (`protocol::script::link::HostLink`), which holds no engine and which
//! a decoder uses directly, since its pipes are shared with the trusted
//! core's messages and it cannot wait on them. This adds only the
//! waiting: a step is written and its batch read before it returns.
//!
//! The network is not this link's: in the browser the trusted core
//! relays a script host's requests to its partition's web client and the
//! answers back (section 11.3), so [`ScriptLink::network`] and
//! [`ScriptLink::take_requests`] have nothing to carry here. On pipes
//! with no trusted core between, a request the script host writes is
//! refused where its batch is awaited.

use std::io::{Read, Write};

use protocol::script::link::{Asked, HostLink, Received, Refusal};
use protocol::script::{Batch, ContextId, ContextOpen, FailReason, Step, read_to_decoder};
use protocol::web::{FromWebClient, ToWebClient};

use crate::link::{LinkError, ScriptLink};

/// What a stopped link answers when the script host gave no words.
const STOPPED: &str = "the scripts were stopped";

/// A link to a script-host process: `output` is the process's input and
/// `input` its output.
pub struct ProcessLink<R: Read, W: Write> {
    input: R,
    output: W,
    link: HostLink,
    /// The script host's highest API version, once it has said READY.
    api_max: Option<u16>,
    /// What each close's one handling staged, in the order closed.
    closed: Vec<(ContextId, Batch)>,
}

impl<R: Read, W: Write> ProcessLink<R, W> {
    /// A link on the pipes of a script host that has just been started.
    pub fn new(input: R, output: W) -> Self {
        Self { input, output, link: HostLink::new(), api_max: None, closed: Vec::new() }
    }

    /// The highest API version the script host has, once it has answered
    /// anything.
    pub const fn api_max(&self) -> Option<u16> {
        self.api_max
    }

    /// What the closes since the last call staged: the sound a `close`
    /// handler stopped, the lines it logged (section 3.1).
    pub fn take_closed(&mut self) -> Vec<(ContextId, Batch)> {
        core::mem::take(&mut self.closed)
    }

    /// Tells the script host to end.
    pub fn shutdown(&mut self) {
        let _ = self.link.shutdown(&mut self.output);
        let _ = self.output.flush();
    }

    fn refused(refusal: &Refusal) -> LinkError {
        match refusal {
            Refusal::Stopped(reason) => LinkError::Stopped(*reason, String::from(STOPPED)),
            Refusal::Context(what) => LinkError::Context(what),
            Refusal::Message(error) => LinkError::of(error),
        }
    }

    /// The error of what stopped the link.
    fn stop(received: Received, reason: Option<FailReason>) -> LinkError {
        match received {
            Received::Stopped(stopped) => LinkError::Stopped(stopped.reason, stopped.text),
            _ => LinkError::Stopped(reason.unwrap_or(FailReason::Link), String::from(STOPPED)),
        }
    }

    /// Reads until `asked` is answered, keeping the batches of closes
    /// that come before it.
    fn await_batch(&mut self, asked: Asked) -> Result<Batch, LinkError> {
        loop {
            let received = match read_to_decoder(&mut self.input) {
                Ok(Some(message)) => self.link.receive(message),
                Ok(None) => self.link.fail(FailReason::Link, "the script host ended"),
                Err(error) => self.link.fail(FailReason::Link, &format!("the script host's message was refused: {error}")),
            };
            match received {
                Received::Ready { api_max } => self.api_max = Some(api_max),
                Received::Batch { asked: answered, batch } if answered == asked => return Ok(*batch),
                Received::Batch { asked: Asked::Close(context), batch } => self.closed.push((context, *batch)),
                Received::Batch { .. } => {
                    let failed = self.link.fail(FailReason::Link, "a batch out of its order");
                    return Err(Self::stop(failed, self.link.stopped()));
                }
                stopped @ (Received::Stopped(_) | Received::Dropped) => return Err(Self::stop(stopped, self.link.stopped())),
            }
        }
    }
}

impl<R: Read, W: Write> ScriptLink for ProcessLink<R, W> {
    fn open(&mut self, context: ContextOpen, source: &[u8]) -> Result<(), LinkError> {
        self.link.open(&mut self.output, &context, source).map_err(|refusal| Self::refused(&refusal))?;
        self.output.flush().map_err(|_| LinkError::Refused("the script host's pipe"))
    }

    fn step(&mut self, step: Step) -> Result<Batch, LinkError> {
        self.link.step(&mut self.output, &step).map_err(|refusal| Self::refused(&refusal))?;
        self.output.flush().map_err(|_| LinkError::Refused("the script host's pipe"))?;
        self.await_batch(Asked::Step(step.sequence))
    }

    fn close(&mut self, context: ContextId) {
        if !self.link.is_open(context) || self.link.close(&mut self.output, context).is_err() || self.output.flush().is_err() {
            return;
        }
        if let Ok(batch) = self.await_batch(Asked::Close(context)) {
            self.closed.push((context, batch));
        }
    }

    fn network(&mut self, _event: FromWebClient) {}

    fn take_requests(&mut self) -> Vec<ToWebClient> {
        Vec::new()
    }
}
