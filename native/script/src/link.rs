//! What runs a (tab, channel)'s scripts, as one interface
//! (script-api.md section 11.1): [`ScriptLink`], and its in-process
//! implementation [`InProcess`], which calls the library directly. The
//! browser's implementation, which speaks the messages of section 11.2 to
//! a script-host process, is work package F3's.
//!
//! What a link carries is the protocol crate's, and nothing of it is
//! defined here: a context's opening, a step and its batch are
//! `protocol::script`'s messages (work package F1, TODO-7288), and the
//! network's answers and requests are `protocol::web`'s. The library
//! checks each as the message's own reader would before it acts on it, so
//! that the in-process link refuses exactly what the pipe would.

use protocol::ProtocolError;
use protocol::script::{Batch, ContextId, ContextOpen, FailReason, Step};
use protocol::web::{FromWebClient, ToWebClient};

use crate::container::{Container, Settings};
use crate::limits::MAX_NETWORK_EVENTS_PER_STEP;
use crate::machine::Engine;

/// The two contexts in the order an event is dispatched: the page context
/// first (section 4.5).
pub(crate) const DISPATCH_ORDER: [ContextId; 2] = [ContextId::Page, ContextId::Channel];

/// The context numbered `number`, as the shim numbers its two.
pub(crate) fn context_numbered(number: i32) -> Option<ContextId> {
    u8::try_from(number).ok().and_then(|code| ContextId::of(code).ok())
}

/// What the library asks of a context's name beside what the message
/// gives.
pub(crate) trait Context: Copy {
    /// Its number, as the shim and the message both number it.
    fn number(self) -> i32;
    /// Its place in a table of the two.
    fn index(self) -> usize;
    /// The other context.
    fn other(self) -> Self;
}

impl Context for ContextId {
    fn number(self) -> i32 {
        i32::from(self.code())
    }

    fn index(self) -> usize {
        match self {
            Self::Channel => 0,
            Self::Page => 1,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Channel => Self::Page,
            Self::Page => Self::Channel,
        }
    }
}

/// Why a link refused or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The context is already open, or is not open.
    Context(&'static str),
    /// A message was out of its bounds or held what its sender may not
    /// send; which field, as the message's own reader names it.
    Refused(&'static str),
    /// The container is stopped for good, for this reason.
    Stopped(FailReason, String),
}

impl LinkError {
    /// The refusal a message's own check gives.
    pub(crate) fn of(error: &ProtocolError) -> Self {
        match error {
            ProtocolError::BadValue(what) => Self::Refused(what),
            _ => Self::Refused("message"),
        }
    }
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
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
    /// When a field is out of its bounds, the source is not the length
    /// the opening says, the context is already open, or the container is
    /// stopped. A top level that throws is not an error of the link: it
    /// is logged and counted as the context's failure.
    fn open(&mut self, context: ContextOpen, source: &[u8]) -> Result<(), LinkError>;
    /// Runs one step and gives everything it changed.
    ///
    /// # Errors
    /// When a field is out of its bounds or the container is stopped.
    fn step(&mut self, step: Step) -> Result<Batch, LinkError>;
    /// Ends a context: one handling of `close`, then its realm is freed.
    fn close(&mut self, context: ContextId);
    /// An answer for a script from the web client.
    fn network(&mut self, event: FromWebClient);
    /// What the scripts asked of the network since the last call.
    fn take_requests(&mut self) -> Vec<ToWebClient>;
}

/// Checks a context's opening as its reader would, and that `source` is
/// the script it says follows.
///
/// # Errors
/// The first field out of its bounds.
pub fn check_open(open: &ContextOpen, source: &[u8]) -> Result<(), LinkError> {
    open.check().map_err(|error| LinkError::of(&error))?;
    if usize::try_from(open.source_bytes).ok() == Some(source.len()) { Ok(()) } else { Err(LinkError::Refused("source length")) }
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
    waiting: Vec<FromWebClient>,
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

    fn network(&mut self, event: FromWebClient) {
        // Held to a bound here too: answers past it are dropped, and the
        // web client's own flow control (section 11.3's `More`) is what
        // keeps a well-behaved one from sending them.
        if self.waiting.len() < MAX_NETWORK_EVENTS_PER_STEP.saturating_mul(MAX_NETWORK_EVENTS_PER_STEP) {
            self.waiting.push(event);
        }
    }

    fn take_requests(&mut self) -> Vec<ToWebClient> {
        self.container.as_mut().map(Container::take_requests).unwrap_or_default()
    }
}
