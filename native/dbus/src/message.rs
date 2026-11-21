//! A message ("Message Format" in the specification): its fixed header,
//! its header fields and its body.

use crate::Error;
use crate::value::{MAX_DECODED_NODES_PER_BYTE, Type, Value, parse_signature, signature_of, wipe};
use crate::wire::{Reader, Writer};

/// The longest message this client sends or reads: 128 KiB. The
/// specification allows 128 MiB ("Message Format"); the browser's replies
/// are each a few hundred bytes (a portal's chosen file's address, a
/// 32-byte secret, a setting), so anything near this bound is not one of
/// them and is refused before it is read.
pub const MAX_MESSAGE_BYTES: usize = 128 * 1024;
/// The fixed part of the header: the byte order, the type, the flags, the
/// version, the body's length and the serial number, then the header
/// fields' array length ("Message Format").
pub(crate) const FIXED_HEADER_BYTES: usize = 16;
/// Where the header fields' array length is, after the 12 fixed bytes.
const FIELDS_LENGTH_AT: usize = 12;
/// Where the body's length is.
const BODY_LENGTH_AT: usize = 4;
/// The header ends padded to 8 ("Message Format": "The length of the
/// header must be a multiple of 8").
const HEADER_ALIGNMENT: usize = 8;
/// "Message Format": the major protocol version, 1.
const PROTOCOL_VERSION: u8 = 1;
/// The byte-order marks: `l` little-endian, `B` big-endian.
const LITTLE_ENDIAN: u8 = b'l';
const BIG_ENDIAN: u8 = b'B';
/// "Message Format", flags: `NO_REPLY_EXPECTED`, which only the
/// connection, built on Unix alone, sets.
#[cfg(unix)]
pub(crate) const NO_REPLY_EXPECTED: u8 = 0x1;

/// "Header Fields": each field's code.
const FIELD_PATH: u8 = 1;
const FIELD_INTERFACE: u8 = 2;
const FIELD_MEMBER: u8 = 3;
const FIELD_ERROR_NAME: u8 = 4;
const FIELD_REPLY_SERIAL: u8 = 5;
const FIELD_DESTINATION: u8 = 6;
const FIELD_SENDER: u8 = 7;
const FIELD_SIGNATURE: u8 = 8;
const FIELD_UNIX_FDS: u8 = 9;

/// A message's type ("Message Types").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `METHOD_CALL`, 1.
    MethodCall,
    /// `METHOD_RETURN`, 2.
    MethodReturn,
    /// `ERROR`, 3.
    Error,
    /// `SIGNAL`, 4.
    Signal,
    /// A type this version of the specification does not name, which "must
    /// be ignored".
    Unknown(u8),
}

impl Kind {
    const fn code(self) -> u8 {
        match self {
            Self::MethodCall => 1,
            Self::MethodReturn => 2,
            Self::Error => 3,
            Self::Signal => 4,
            Self::Unknown(code) => code,
        }
    }

    const fn of(code: u8) -> Self {
        match code {
            1 => Self::MethodCall,
            2 => Self::MethodReturn,
            3 => Self::Error,
            4 => Self::Signal,
            other => Self::Unknown(other),
        }
    }
}

/// A message, its header fields named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Its type.
    pub kind: Kind,
    /// Its flags.
    pub flags: u8,
    /// Its serial number, not zero.
    pub serial: u32,
    /// The object it is to or from.
    pub path: Option<String>,
    /// The interface of its method or signal.
    pub interface: Option<String>,
    /// Its method's or signal's name.
    pub member: Option<String>,
    /// An error's name.
    pub error_name: Option<String>,
    /// The serial number of the call it answers.
    pub reply_serial: Option<u32>,
    /// Whom it is to.
    pub destination: Option<String>,
    /// Who sent it, as the bus says.
    pub sender: Option<String>,
    /// Its arguments.
    pub body: Vec<Value>,
}

impl Message {
    /// A method call to `member` of `interface` on `path` at
    /// `destination`, with `body`; its serial number is given when it is
    /// sent.
    #[must_use]
    pub fn method_call(destination: &str, path: &str, interface: &str, member: &str, body: Vec<Value>) -> Self {
        Self {
            kind: Kind::MethodCall,
            flags: 0,
            serial: 0,
            path: Some(path.to_owned()),
            interface: Some(interface.to_owned()),
            member: Some(member.to_owned()),
            error_name: None,
            reply_serial: None,
            destination: Some(destination.to_owned()),
            sender: None,
            body,
        }
    }

    /// The message's bytes, little-endian.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] when a field or the body is not one this client
    /// may send, and [`Error::TooLong`] beyond [`MAX_MESSAGE_BYTES`].
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.serial == 0 {
            return Err(Error::Malformed("a serial number of zero"));
        }
        let mut body = Writer::default();
        for value in &self.body {
            body.value(value, 0)?;
        }
        let signature = signature_of(&self.body.iter().map(Value::kind).collect::<Vec<_>>());
        let mut fields = Vec::new();
        let mut field = |code: u8, value: Option<Value>| {
            if let Some(value) = value {
                fields.push(Value::Struct(vec![Value::Byte(code), Value::Variant(Box::new(value))]));
            }
        };
        field(FIELD_PATH, self.path.clone().map(Value::ObjectPath));
        field(FIELD_INTERFACE, self.interface.clone().map(Value::String));
        field(FIELD_MEMBER, self.member.clone().map(Value::String));
        field(FIELD_ERROR_NAME, self.error_name.clone().map(Value::String));
        field(FIELD_REPLY_SERIAL, self.reply_serial.map(Value::Uint32));
        field(FIELD_DESTINATION, self.destination.clone().map(Value::String));
        field(FIELD_SENDER, self.sender.clone().map(Value::String));
        field(FIELD_SIGNATURE, (!signature.is_empty()).then_some(Value::Signature(signature)));
        let mut message = Writer::default();
        message.u8(LITTLE_ENDIAN);
        message.u8(self.kind.code());
        message.u8(self.flags);
        message.u8(PROTOCOL_VERSION);
        message.u32(u32::try_from(body.bytes.len()).map_err(|_| Error::TooLong)?)?;
        message.u32(self.serial)?;
        message.value(&Value::Array(header_field_type(), fields), 0)?;
        message.pad(HEADER_ALIGNMENT)?;
        message.bytes.extend_from_slice(&body.bytes);
        // Both writers wipe what they hold when dropped, on every path.
        if message.bytes.len() > MAX_MESSAGE_BYTES {
            return Err(Error::TooLong);
        }
        Ok(message.into_bytes())
    }

    /// The message in `bytes`, all of them, checked as "Message Format",
    /// "Header Fields" and "Valid Names" say.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`], naming the rule broken, or
    /// [`Error::TooLong`].
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != frame_length(bytes)? {
            return Err(Error::Malformed("a message's length is not its header's"));
        }
        let big_endian = bytes.first() == Some(&BIG_ENDIAN);
        let budget = bytes.len().saturating_mul(MAX_DECODED_NODES_PER_BYTE);
        let mut reader = Reader { bytes, position: 0, big_endian, budget };
        reader.u8()?;
        let kind = Kind::of(reader.u8()?);
        let flags = reader.u8()?;
        if reader.u8()? != PROTOCOL_VERSION {
            return Err(Error::Malformed("a protocol version other than 1"));
        }
        let body_length = usize::try_from(reader.u32()?).map_err(|_| Error::TooLong)?;
        let serial = reader.u32()?;
        if serial == 0 {
            return Err(Error::Malformed("a serial number of zero"));
        }
        let mut message = Self {
            kind,
            flags,
            serial,
            path: None,
            interface: None,
            member: None,
            error_name: None,
            reply_serial: None,
            destination: None,
            sender: None,
            body: Vec::new(),
        };
        let mut signature = String::new();
        let Value::Array(_, fields) = reader.value(&Type::Array(Box::new(header_field_type())), 0)? else {
            return Err(Error::Malformed("header fields that are not an array"));
        };
        for field in fields {
            let Some([Value::Byte(code), Value::Variant(value)]) = field.as_struct() else {
                return Err(Error::Malformed("a header field that is not a code and a variant"));
            };
            let wrong = Error::Malformed("a header field of the wrong type");
            match (*code, value.as_ref()) {
                (FIELD_PATH, Value::ObjectPath(path)) => message.path = Some(path.clone()),
                (FIELD_INTERFACE, Value::String(name)) if valid_interface(name) => message.interface = Some(name.clone()),
                (FIELD_MEMBER, Value::String(name)) if valid_member(name) => message.member = Some(name.clone()),
                (FIELD_ERROR_NAME, Value::String(name)) if valid_interface(name) => message.error_name = Some(name.clone()),
                (FIELD_REPLY_SERIAL, Value::Uint32(number)) if *number != 0 => message.reply_serial = Some(*number),
                (FIELD_DESTINATION, Value::String(name)) if valid_bus_name(name) => message.destination = Some(name.clone()),
                (FIELD_SENDER, Value::String(name)) if valid_bus_name(name) => message.sender = Some(name.clone()),
                (FIELD_SIGNATURE, Value::Signature(given)) => signature.clone_from(given),
                // No descriptors are negotiated, so a message must carry none.
                (FIELD_UNIX_FDS, Value::Uint32(0)) => {}
                (
                    FIELD_PATH | FIELD_INTERFACE | FIELD_MEMBER | FIELD_ERROR_NAME | FIELD_REPLY_SERIAL | FIELD_DESTINATION | FIELD_SENDER | FIELD_SIGNATURE
                    | FIELD_UNIX_FDS,
                    _,
                ) => {
                    return Err(wrong);
                }
                // "Header Fields": unknown fields must be accepted and ignored.
                _ => {}
            }
        }
        let required = match kind {
            Kind::MethodCall => message.path.is_some() && message.member.is_some(),
            Kind::MethodReturn => message.reply_serial.is_some(),
            Kind::Error => message.error_name.is_some() && message.reply_serial.is_some(),
            Kind::Signal => message.path.is_some() && message.interface.is_some() && message.member.is_some(),
            Kind::Unknown(_) => true,
        };
        if !required {
            return Err(Error::Malformed("a message without a header field its type requires"));
        }
        reader.pad(HEADER_ALIGNMENT)?;
        if reader.bytes.len().checked_sub(reader.position) != Some(body_length) {
            return Err(Error::Malformed("a body of another length than its header says"));
        }
        let body = parse_signature(&signature).and_then(|kinds| {
            for kind in kinds {
                message.body.push(reader.value(&kind, 0)?);
            }
            if reader.position == bytes.len() { Ok(()) } else { Err(Error::Malformed("a body longer than its signature")) }
        });
        match body {
            Ok(()) => Ok(message),
            // What was read of a body refused part way is wiped, a secret's
            // bytes among it (TODO-7173 finding A4).
            Err(error) => {
                message.wipe();
                Err(error)
            }
        }
    }

    /// Clears the body's byte arrays (a secret's among them), as
    /// [`crate::wipe`] does.
    pub fn wipe(&mut self) {
        wipe(&mut self.body);
    }
}

/// A header field: its code and its value (`(yv)`).
fn header_field_type() -> Type {
    Type::Struct(vec![Type::Byte, Type::Variant])
}

/// The length of the message whose first [`FIXED_HEADER_BYTES`] are at the
/// start of `head`: the header, its padding and the body.
///
/// # Errors
///
/// [`Error::Malformed`] for an unknown byte order or fewer bytes than the
/// fixed header, and [`Error::TooLong`] beyond [`MAX_MESSAGE_BYTES`].
pub(crate) fn frame_length(head: &[u8]) -> Result<usize, Error> {
    let order = *head.first().ok_or(Error::Malformed("a message shorter than its fixed header"))?;
    if order != LITTLE_ENDIAN && order != BIG_ENDIAN {
        return Err(Error::Malformed("an unknown byte order"));
    }
    let number_at = |at: usize| -> Result<usize, Error> {
        let mut reader = Reader { bytes: head, position: at, big_endian: order == BIG_ENDIAN, budget: 0 };
        usize::try_from(reader.u32()?).map_err(|_| Error::TooLong)
    };
    let fields = number_at(FIELDS_LENGTH_AT)?;
    let body = number_at(BODY_LENGTH_AT)?;
    let total = FIXED_HEADER_BYTES
        .checked_add(fields)
        .and_then(|header| header.checked_next_multiple_of(HEADER_ALIGNMENT))
        .and_then(|header| header.checked_add(body))
        .ok_or(Error::TooLong)?;
    if total > MAX_MESSAGE_BYTES {
        return Err(Error::TooLong);
    }
    Ok(total)
}

/// "Valid Names": the longest bus, interface, member or error name.
const MAX_NAME_BYTES: usize = 255;

/// An interface or error name: two or more elements of ASCII letters,
/// digits and underscores, not starting with a digit, separated by dots.
fn valid_interface(name: &str) -> bool {
    name.len() <= MAX_NAME_BYTES && name.split('.').count() >= 2 && name.split('.').all(valid_member)
}

/// A member name: ASCII letters, digits and underscores, not empty, not
/// starting with a digit.
fn valid_member(name: &str) -> bool {
    name.len() <= MAX_NAME_BYTES
        && name.bytes().next().is_some_and(|first| !first.is_ascii_digit())
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A bus name: a unique name (`:` then elements that may start with a
/// digit) or a well-known one (as an interface's, with hyphens allowed).
fn valid_bus_name(name: &str) -> bool {
    let (unique, elements) = match name.strip_prefix(':') {
        Some(rest) => (true, rest),
        None => (false, name),
    };
    name.len() <= MAX_NAME_BYTES
        && elements.split('.').count() >= 2
        && elements.split('.').all(|element| {
            element.bytes().next().is_some_and(|first| unique || !first.is_ascii_digit())
                && element.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    fn call() -> Message {
        let mut message = Message::method_call(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
            "OpenFile",
            vec![Value::String("x11:4a00003".into()), Value::String("Open File".into()), Value::variant_dict(vec![("multiple", Value::Boolean(false))])],
        );
        message.serial = 3;
        message
    }

    #[test]
    fn a_message_reads_back_as_written() {
        let bytes = call().encode().unwrap();
        assert_eq!(bytes.first(), Some(&LITTLE_ENDIAN));
        assert_eq!(frame_length(&bytes).unwrap(), bytes.len());
        assert_eq!(Message::decode(&bytes).unwrap(), call());
    }

    /// A reply to `Hello` laid out by hand as "Message Format" gives it: a
    /// method return of serial 0xFFFFFFFF answering serial 1, from the bus,
    /// to and carrying the unique name `:1.234`, with its fields in another
    /// order than this client writes them. The real bus's replies are read
    /// in `tests/session_bus.rs`.
    #[test]
    fn a_reply_the_bus_sent_reads() {
        let bytes: &[u8] = b"l\x02\x01\x01\x0b\0\0\0\xff\xff\xff\xff\x3d\0\0\0\x06\x01s\0\x06\0\0\0:1.234\0\0\x05\x01u\0\x01\0\0\0\x08\x01g\0\x01s\0\0\x07\x01s\0\x14\0\0\0org.freedesktop.DBus\0\0\0\0\x06\0\0\0:1.234\0";
        let message = Message::decode(bytes).unwrap();
        assert_eq!((message.kind, message.reply_serial, message.serial), (Kind::MethodReturn, Some(1), u32::MAX));
        assert_eq!(message.sender.as_deref(), Some("org.freedesktop.DBus"));
        assert_eq!(message.body, vec![Value::String(":1.234".into())]);
    }

    #[test]
    fn a_message_missing_what_its_type_needs_is_refused() {
        let mut signal = call();
        signal.kind = Kind::Signal;
        signal.interface = None;
        assert_eq!(Message::decode(&signal.encode().unwrap()), Err(Error::Malformed("a message without a header field its type requires")));
        let mut reply = call();
        reply.kind = Kind::MethodReturn;
        assert!(Message::decode(&reply.encode().unwrap()).is_err());
    }

    #[test]
    fn a_body_unlike_its_signature_is_refused() {
        let mut bytes = call().encode().unwrap();
        bytes.push(0);
        assert!(Message::decode(&bytes).is_err());
        bytes.truncate(bytes.len().saturating_sub(2));
        assert!(Message::decode(&bytes).is_err());
    }

    #[test]
    fn a_message_beyond_the_bound_is_refused_from_its_header() {
        let mut head = call().encode().unwrap();
        head.get_mut(BODY_LENGTH_AT..BODY_LENGTH_AT + 4).unwrap().copy_from_slice(&u32::try_from(MAX_MESSAGE_BYTES).unwrap().to_le_bytes());
        assert_eq!(frame_length(&head), Err(Error::TooLong));
        let mut long = call();
        long.body = vec![Value::Bytes(vec![0; MAX_MESSAGE_BYTES])];
        assert_eq!(long.encode(), Err(Error::TooLong));
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_interface("org.freedesktop.Secret.Service"));
        assert!(!valid_interface("org") && !valid_interface("org.9a") && !valid_interface("org..a"));
        assert!(valid_bus_name(":1.234") && valid_bus_name("org.freedesktop.secrets") && valid_bus_name("org.a-b.c"));
        assert!(!valid_bus_name(":1") && !valid_bus_name("org.1a") && !valid_bus_name(""));
        assert!(valid_member("OpenFile") && !valid_member("Open.File") && !valid_member("1a") && !valid_member(""));
    }
}
