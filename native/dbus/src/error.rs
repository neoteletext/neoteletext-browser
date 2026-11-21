//! What can go wrong, in one type.

/// Why a D-Bus exchange failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No session bus address could be found or reached.
    NoBus(String),
    /// The socket failed.
    Io(String),
    /// The bus refused authentication, or answered it in a way the
    /// specification does not allow.
    Authentication(String),
    /// Bytes from the bus that are not a valid message, or a message this
    /// client would not send; the text names the rule broken.
    Malformed(&'static str),
    /// A message longer than [`crate::MAX_MESSAGE_BYTES`].
    TooLong,
    /// The other side answered with an error message: its name and its
    /// text, if it gave one.
    Remote {
        /// The error's name, such as `org.freedesktop.DBus.Error.UnknownMethod`.
        name: String,
        /// The error's text.
        message: String,
    },
    /// A reply had the wrong arguments for the call.
    UnexpectedReply(&'static str),
    /// No reply came within [`crate::CALL_TIMEOUT`].
    Timeout,
    /// The bus closed the connection.
    Closed,
    /// More signals arrived while a reply was awaited than are kept.
    TooManySignals,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBus(why) => write!(f, "no session bus: {why}"),
            Self::Io(why) => write!(f, "the session bus's socket: {why}"),
            Self::Authentication(why) => write!(f, "the session bus refused authentication: {why}"),
            Self::Malformed(rule) => write!(f, "a malformed D-Bus message: {rule}"),
            Self::TooLong => write!(f, "a D-Bus message longer than {} bytes", crate::MAX_MESSAGE_BYTES),
            Self::Remote { name, message } if message.is_empty() => write!(f, "{name}"),
            Self::Remote { name, message } => write!(f, "{name}: {message}"),
            Self::UnexpectedReply(what) => write!(f, "an unexpected D-Bus reply: {what}"),
            Self::Timeout => write!(f, "no D-Bus reply in time"),
            Self::Closed => write!(f, "the session bus closed the connection"),
            Self::TooManySignals => write!(f, "too many D-Bus signals while waiting for a reply"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => Self::Timeout,
            std::io::ErrorKind::UnexpectedEof => Self::Closed,
            _ => Self::Io(error.to_string()),
        }
    }
}
