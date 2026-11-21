//! One connection to the session bus: authenticated, named by the bus
//! (`Hello`), and used for one caller's calls, one at a time.

use std::collections::VecDeque;
use std::io::{Read as _, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::address::session_addresses;
use crate::message::{FIXED_HEADER_BYTES, NO_REPLY_EXPECTED, frame_length};
use crate::value::wipe_bytes;
use crate::{Address, Error, Kind, Message, Value};

/// How long a method call waits for its reply: 25 seconds, the reference
/// implementation's default reply timeout (libdbus,
/// `_DBUS_DEFAULT_TIMEOUT_VALUE`). A wait for a signal that a person
/// answers, a dialog's or an unlock prompt's, has no timeout: the person
/// may take minutes.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(25);
/// The most signals kept while a reply is awaited. Only a signal a wait can
/// take is kept: one [`Connection::expect_signal`] named, from the unique
/// name that owned the service when the expectation was made, which the bus
/// and not the sender fills in. Every other signal is wiped and dropped as
/// it comes, so no other process can fill the connection or fail its calls
/// (the security review of 22 November 2026, TODO-7173 findings A1 and A2).
/// A caller waits for one or two; more than this from the service itself
/// is the service's fault.
pub const MAX_WAITING_SIGNALS: usize = 32;
/// The most signals expected at once: the chooser expects the portal's
/// `Response` on at most two paths (the predicted one and an older portal's
/// own), the key store one prompt's `Completed` at a time.
pub const MAX_EXPECTED_SIGNALS: usize = 4;
/// The longest line of the authentication exchange read: the server's
/// `OK` and its 32-digit identity are 35 bytes, `REJECTED` and its list of
/// mechanisms a few dozen ("Authentication Protocol").
const MAX_AUTHENTICATION_LINE: usize = 512;
/// The bus itself ("Message Bus Specification").
const BUS_NAME: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const BUS_INTERFACE: &str = "org.freedesktop.DBus";

/// A connection to the session bus.
pub struct Connection {
    stream: UnixStream,
    serial: u32,
    unique_name: String,
    signals: VecDeque<Message>,
    expected: Vec<Expected>,
}

/// A signal a wait will take: its sender's unique name, its path, its
/// interface and its member.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Expected {
    sender: String,
    path: String,
    interface: String,
    member: String,
}

impl Expected {
    fn new(sender: &str, path: &str, interface: &str, member: &str) -> Self {
        Self { sender: sender.to_owned(), path: path.to_owned(), interface: interface.to_owned(), member: member.to_owned() }
    }

    fn matches(&self, message: &Message) -> bool {
        message.kind == Kind::Signal
            && message.sender.as_deref() == Some(self.sender.as_str())
            && message.path.as_deref() == Some(self.path.as_str())
            && message.interface.as_deref() == Some(self.interface.as_str())
            && message.member.as_deref() == Some(self.member.as_str())
    }
}

impl Connection {
    /// A connection to the session bus, at the first of its addresses that
    /// answers.
    ///
    /// # Errors
    ///
    /// [`Error::NoBus`] when no address is known, or the last address's
    /// failure.
    pub fn session() -> Result<Self, Error> {
        let mut last = Error::NoBus(String::from("no address of a local socket"));
        for address in session_addresses()? {
            match Self::connect(&address) {
                Ok(connection) => return Ok(connection),
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    /// A connection to the bus at `address`, authenticated and named.
    ///
    /// # Errors
    ///
    /// The socket's, the authentication's or `Hello`'s failure.
    pub fn connect(address: &Address) -> Result<Self, Error> {
        let stream = match address {
            Address::Path(path) => UnixStream::connect(std::path::Path::new(std::ffi::OsStr::from_bytes(path))),
            Address::Abstract(name) => connect_abstract(name),
        }
        .map_err(|error| Error::NoBus(error.to_string()))?;
        stream.set_read_timeout(Some(CALL_TIMEOUT))?;
        stream.set_write_timeout(Some(CALL_TIMEOUT))?;
        let mut connection = Self { stream, serial: 0, unique_name: String::new(), signals: VecDeque::new(), expected: Vec::new() };
        connection.authenticate()?;
        let reply = connection.call(Message::method_call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "Hello", Vec::new()))?;
        connection.unique_name = match reply.as_slice() {
            [Value::String(name)] => name.clone(),
            _ => return Err(Error::UnexpectedReply("Hello did not answer a name")),
        };
        Ok(connection)
    }

    /// "Authentication Protocol": a zero byte, then `AUTH EXTERNAL` with
    /// this process's user number, its decimal digits in hexadecimal; the
    /// bus checks it against the socket's credentials and answers `OK`.
    fn authenticate(&mut self) -> Result<(), Error> {
        let user = rustix::process::getuid().as_raw().to_string();
        let identity = user.bytes().fold(String::new(), |mut identity, byte| {
            use std::fmt::Write as _;
            let _ = write!(identity, "{byte:02x}");
            identity
        });
        self.stream.write_all(format!("\0AUTH EXTERNAL {identity}\r\n").as_bytes())?;
        let line = self.authentication_line()?;
        if !line.starts_with("OK ") {
            return Err(Error::Authentication(line));
        }
        self.stream.write_all(b"BEGIN\r\n")?;
        Ok(())
    }

    /// One line from the server, without its `\r\n`, read a byte at a time
    /// so that nothing after it is taken.
    fn authentication_line(&mut self) -> Result<String, Error> {
        let mut line = Vec::new();
        loop {
            let mut byte = [0];
            self.stream.read_exact(&mut byte)?;
            line.extend_from_slice(&byte);
            if line.ends_with(b"\r\n") {
                line.truncate(line.len().saturating_sub(2));
                return String::from_utf8(line).map_err(|_| Error::Authentication(String::from("a line that is not text")));
            }
            if line.len() >= MAX_AUTHENTICATION_LINE {
                return Err(Error::Authentication(String::from("a line too long")));
            }
        }
    }

    /// The name the bus gave this connection, such as `:1.234`.
    #[must_use]
    pub fn unique_name(&self) -> &str {
        &self.unique_name
    }

    fn send(&mut self, mut message: Message) -> Result<u32, Error> {
        self.serial = self.serial.checked_add(1).ok_or(Error::Malformed("serial numbers exhausted"))?;
        message.serial = self.serial;
        let encoded = message.encode();
        message.wipe();
        let mut bytes = encoded?;
        let written = self.stream.write_all(&bytes);
        wipe_bytes(&mut bytes);
        written?;
        Ok(self.serial)
    }

    /// The next message from the bus, before `deadline` if there is one.
    /// Once this has failed, the stream may be inside a message, and the
    /// connection is not to be used again.
    fn receive(&mut self, deadline: Option<Instant>) -> Result<Message, Error> {
        let timeout = match deadline {
            Some(deadline) => Some(deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()).ok_or(Error::Timeout)?),
            None => None,
        };
        self.stream.set_read_timeout(timeout)?;
        let mut head = [0; FIXED_HEADER_BYTES];
        self.stream.read_exact(&mut head)?;
        let length = frame_length(&head)?;
        let mut bytes = vec![0; length];
        let (start, rest) = bytes.split_at_mut(FIXED_HEADER_BYTES.min(length));
        start.copy_from_slice(head.get(..start.len()).ok_or(Error::Malformed("a message shorter than its fixed header"))?);
        let read = self.stream.read_exact(rest).map_err(Error::from).and_then(|()| Message::decode(&bytes));
        wipe_bytes(&mut bytes);
        read
    }

    /// Sends `call` and waits at most [`CALL_TIMEOUT`] for its reply,
    /// keeping the expected signals that come meanwhile for
    /// [`Connection::wait_for_signal`] and dropping every other.
    ///
    /// # Errors
    ///
    /// [`Error::Remote`] for an error reply, and the socket's or the
    /// message's failure.
    pub fn call(&mut self, call: Message) -> Result<Vec<Value>, Error> {
        let serial = self.send(call)?;
        let deadline = Instant::now().checked_add(CALL_TIMEOUT);
        loop {
            let mut message = self.receive(deadline)?;
            match message.kind {
                Kind::MethodReturn if message.reply_serial == Some(serial) => return Ok(message.body),
                Kind::Error if message.reply_serial == Some(serial) => {
                    let text = match message.body.first() {
                        Some(Value::String(text)) => text.clone(),
                        _ => String::new(),
                    };
                    message.wipe();
                    return Err(Error::Remote { name: message.error_name.unwrap_or_default(), message: text });
                }
                Kind::Signal if self.expected.iter().any(|expected| expected.matches(&message)) => {
                    if self.signals.len() >= MAX_WAITING_SIGNALS {
                        message.wipe();
                        return Err(Error::TooManySignals);
                    }
                    self.signals.push_back(message);
                }
                // Signals no wait will take, calls to this connection, which
                // serves nothing, and replies to nothing it asked: the
                // caller of a call gets its error from the bus when this
                // connection closes.
                _ => message.wipe(),
            }
        }
    }

    /// Sends `call` and waits for no reply (`NO_REPLY_EXPECTED`).
    ///
    /// # Errors
    ///
    /// The socket's or the message's failure.
    pub fn call_without_reply(&mut self, mut call: Message) -> Result<(), Error> {
        call.flags |= NO_REPLY_EXPECTED;
        self.send(call).map(|_| ())
    }

    /// The unique name that owns `name` now (`GetNameOwner`).
    ///
    /// # Errors
    ///
    /// [`Error::Remote`] when no one owns it.
    pub fn name_owner(&mut self, name: &str) -> Result<String, Error> {
        match self.call(Message::method_call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "GetNameOwner", vec![Value::String(name.to_owned())]))?.as_slice() {
            [Value::String(owner)] => Ok(owner.clone()),
            _ => Err(Error::UnexpectedReply("GetNameOwner did not answer a name")),
        }
    }

    /// Asks the bus for the signals `rule` matches ("Match Rules",
    /// `AddMatch`).
    ///
    /// # Errors
    ///
    /// [`Error::Remote`] when the bus refuses the rule.
    pub fn add_match(&mut self, rule: &str) -> Result<(), Error> {
        self.call(Message::method_call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "AddMatch", vec![Value::String(rule.to_owned())])).map(|_| ())
    }

    /// Asks the bus for the signal `member` of `interface` on `path` sent by
    /// `sender`, a unique name (`AddMatch`), and keeps it if it comes while
    /// a call awaits its reply, for [`Connection::wait_for_signal`]. Only a
    /// signal expected here is kept during a call.
    ///
    /// # Errors
    ///
    /// [`Error::TooManySignals`] beyond [`MAX_EXPECTED_SIGNALS`],
    /// [`Error::Malformed`] when a part could not be quoted in a match
    /// rule, and [`Error::Remote`] when the bus refuses the rule.
    pub fn expect_signal(&mut self, sender: &str, path: &str, interface: &str, member: &str) -> Result<(), Error> {
        let expected = Expected::new(sender, path, interface, member);
        if self.expected.contains(&expected) {
            return Ok(());
        }
        if self.expected.len() >= MAX_EXPECTED_SIGNALS {
            return Err(Error::TooManySignals);
        }
        // "Match Rules": a value is quoted with `'`, which none of a valid
        // name or path holds.
        if [sender, path, interface, member].iter().any(|part| part.contains(['\'', '\\'])) {
            return Err(Error::Malformed("a match rule's value holding a quote"));
        }
        self.add_match(&format!("type='signal',sender='{sender}',path='{path}',interface='{interface}',member='{member}'"))?;
        self.expected.push(expected);
        Ok(())
    }

    /// How many signals are kept for [`Connection::wait_for_signal`].
    #[must_use]
    pub fn waiting_signals(&self) -> usize {
        self.signals.len()
    }

    /// Waits, with no timeout, for the signal `member` of `interface` on
    /// `path` sent by `sender`, a unique name; returns its arguments. Any
    /// other signal is dropped. A person answers the signals waited for
    /// here, so the wait ends when they do or when the bus closes.
    /// The expectation [`Connection::expect_signal`] made for it, if any,
    /// ends with the wait.
    ///
    /// # Errors
    ///
    /// The socket's or a message's failure.
    pub fn wait_for_signal(&mut self, sender: &str, path: &str, interface: &str, member: &str) -> Result<Vec<Value>, Error> {
        let wanted = Expected::new(sender, path, interface, member);
        self.expected.retain(|expected| *expected != wanted);
        // Signals kept for another expectation stay kept.
        if let Some(at) = self.signals.iter().position(|message| wanted.matches(message)) {
            return self.signals.remove(at).map(|message| message.body).ok_or(Error::Malformed("a kept signal went missing"));
        }
        loop {
            let mut message = self.receive(None)?;
            if wanted.matches(&message) {
                return Ok(message.body);
            }
            if message.kind == Kind::Signal && self.expected.iter().any(|expected| expected.matches(&message)) && self.signals.len() < MAX_WAITING_SIGNALS {
                self.signals.push_back(message);
            } else {
                message.wipe();
            }
        }
    }
}

/// A socket in Linux's abstract namespace.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn connect_abstract(name: &[u8]) -> std::io::Result<UnixStream> {
    use std::os::linux::net::SocketAddrExt as _;
    UnixStream::connect_addr(&std::os::unix::net::SocketAddr::from_abstract_name(name)?)
}

/// Only Linux has the abstract namespace.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn connect_abstract(_name: &[u8]) -> std::io::Result<UnixStream> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "an abstract socket outside Linux"))
}
