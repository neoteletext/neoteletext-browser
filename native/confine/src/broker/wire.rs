//! The broker's wire format: each message a kind byte, a big-endian
//! length and a payload, every field read within its bound. The broker's
//! one input from another process, and fuzzed (`broker_requests`,
//! `broker_replies`).

use std::fmt;
use std::io;

use super::{
    COOKIE_KEY_BYTES, HEADER_BYTES, KIND_COOKIE_KEY, KIND_DONE, KIND_EXITED, KIND_FAILED, KIND_HEALTHY, KIND_KEY, KIND_KILL, KIND_LOCAL_TIME, KIND_OPEN_WEB,
    KIND_READ, KIND_SOURCE, KIND_SPAWN, KIND_SPAWNED, KIND_TIME, KIND_UPDATE_STATE, KIND_WAIT, MAX_ARGUMENTS, MAX_REPLY, MAX_TEXT, Program,
};
use crate::LocalTime;

/// How a helper ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    /// It ended successfully.
    pub success: bool,
    /// How, in the system's words.
    pub text: String,
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl From<std::process::ExitStatus> for Exit {
    fn from(status: std::process::ExitStatus) -> Self {
        Self { success: status.success(), text: status.to_string() }
    }
}

/// A request from the core.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// Start a helper.
    Spawn {
        /// Which.
        program: Program,
        /// May a network helper reach the local network.
        local_network: bool,
        /// Its arguments.
        arguments: Vec<String>,
    },
    /// End a helper.
    Kill(u32),
    /// Wait for a helper to end.
    Wait(u32),
    /// Read a file channel.
    Read(String),
    /// Hand a web address to the system's browser.
    OpenWeb(String),
    /// The time, and the local time zone's offset from it, which the core
    /// cannot read from the system's files (`history-and-bookmarks.md`
    /// section 7).
    LocalTime,
    /// The cookie file's key, from the platform's secret store, which the
    /// core cannot reach (`cookies.md` section 10).
    CookieKey,
    /// The version is healthy (`installation-and-updates.md` section 3.2):
    /// the broker tells the launcher, which commits a trial.
    Healthy,
    /// The launcher's state file, which the core may not read itself:
    /// answered with its bytes, or refused when no launcher started the
    /// broker.
    UpdateState,
}

/// The broker's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// Started: its number and how it was confined; its pipes travel with
    /// the reply.
    Spawned {
        /// The number the core names it by.
        id: u32,
        /// How it was confined.
        report: String,
    },
    /// Done.
    Done,
    /// It ended.
    Exited(Exit),
    /// A file channel's bytes.
    Source(Vec<u8>),
    /// Refused, and why.
    Failed(String),
    /// The time and the local offset.
    Time(LocalTime),
    /// The cookie file's key.
    Key(Vec<u8>),
}

fn push_text(payload: &mut Vec<u8>, text: &str) -> io::Result<()> {
    let length = u16::try_from(text.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    if text.len() > MAX_TEXT {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "text too long"));
    }
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(text.as_bytes());
    Ok(())
}

/// Reads fields, refusing anything short, long or not UTF-8.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < count {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message ends early"));
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?))
    }

    fn text(&mut self) -> io::Result<String> {
        let length = usize::from(self.u16()?);
        if length > MAX_TEXT {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "text too long"));
        }
        String::from_utf8(self.take(length)?.to_vec()).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "text not UTF-8"))
    }

    fn end(&self) -> io::Result<()> {
        if self.0.is_empty() { Ok(()) } else { Err(io::Error::new(io::ErrorKind::InvalidData, "message longer than its fields")) }
    }
}

pub(crate) fn frame(kind: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
    let length = u32::try_from(payload.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut bytes = Vec::with_capacity(HEADER_BYTES.saturating_add(payload.len()));
    bytes.push(kind);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// A request as bytes.
///
/// # Errors
/// When it breaks a bound.
pub fn encode_request(request: &Request) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match request {
        Request::Spawn { program, local_network, arguments } => {
            if arguments.len() > MAX_ARGUMENTS {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "too many arguments"));
            }
            payload.push(program.code());
            payload.push(u8::from(*local_network));
            payload.extend_from_slice(&u16::try_from(arguments.len()).map_err(|_| io::ErrorKind::InvalidInput)?.to_be_bytes());
            for argument in arguments {
                push_text(&mut payload, argument)?;
            }
            KIND_SPAWN
        }
        Request::Kill(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_KILL
        }
        Request::Wait(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_WAIT
        }
        Request::Read(path) => {
            push_text(&mut payload, path)?;
            KIND_READ
        }
        Request::OpenWeb(address) => {
            push_text(&mut payload, address)?;
            KIND_OPEN_WEB
        }
        Request::LocalTime => KIND_LOCAL_TIME,
        Request::CookieKey => KIND_COOKIE_KEY,
        Request::Healthy => KIND_HEALTHY,
        Request::UpdateState => KIND_UPDATE_STATE,
    };
    frame(kind, &payload)
}

/// A request read from its kind and payload, strictly: the broker's one
/// input from another process.
///
/// # Errors
/// On any malformed or out-of-bounds request.
pub fn decode_request(kind: u8, payload: &[u8]) -> io::Result<Request> {
    let mut fields = Fields(payload);
    let request = match kind {
        KIND_SPAWN => {
            let program = Program::from_code(fields.u8()?).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no such program"))?;
            let local_network = match fields.u8()? {
                0 => false,
                1 => true,
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "a flag that is not one")),
            };
            let count = usize::from(fields.u16()?);
            if count > MAX_ARGUMENTS {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "too many arguments"));
            }
            let arguments = (0..count).map(|_| fields.text()).collect::<io::Result<Vec<_>>>()?;
            Request::Spawn { program, local_network, arguments }
        }
        KIND_KILL => Request::Kill(fields.u32()?),
        KIND_WAIT => Request::Wait(fields.u32()?),
        KIND_READ => Request::Read(fields.text()?),
        KIND_OPEN_WEB => Request::OpenWeb(fields.text()?),
        KIND_LOCAL_TIME => Request::LocalTime,
        KIND_COOKIE_KEY => Request::CookieKey,
        KIND_HEALTHY => Request::Healthy,
        KIND_UPDATE_STATE => Request::UpdateState,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "no such request")),
    };
    fields.end()?;
    Ok(request)
}

/// A reply as bytes.
///
/// # Errors
/// When it breaks a bound.
pub fn encode_reply(reply: &Reply) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match reply {
        Reply::Spawned { id, report } => {
            payload.extend_from_slice(&id.to_be_bytes());
            push_text(&mut payload, report)?;
            KIND_SPAWNED
        }
        Reply::Done => KIND_DONE,
        Reply::Exited(exit) => {
            payload.push(u8::from(exit.success));
            push_text(&mut payload, &exit.text)?;
            KIND_EXITED
        }
        Reply::Source(bytes) => {
            if bytes.len() > MAX_REPLY {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "source too large"));
            }
            payload.extend_from_slice(bytes);
            KIND_SOURCE
        }
        Reply::Failed(reason) => {
            push_text(&mut payload, reason)?;
            KIND_FAILED
        }
        Reply::Time(time) => {
            if !time.is_plausible() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "an offset no zone has"));
            }
            payload.extend_from_slice(&time.utc.to_be_bytes());
            payload.extend_from_slice(&time.offset.to_be_bytes());
            KIND_TIME
        }
        Reply::Key(key) => {
            if key.len() != COOKIE_KEY_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "a key of the wrong length"));
            }
            payload.extend_from_slice(key);
            KIND_KEY
        }
    };
    frame(kind, &payload)
}

/// A reply read from its kind and payload, strictly.
///
/// # Errors
/// On any malformed reply.
pub fn decode_reply(kind: u8, payload: Vec<u8>) -> io::Result<Reply> {
    if kind == KIND_SOURCE {
        return Ok(Reply::Source(payload));
    }
    let mut fields = Fields(&payload);
    let reply = match kind {
        KIND_SPAWNED => Reply::Spawned { id: fields.u32()?, report: fields.text()? },
        KIND_DONE => Reply::Done,
        KIND_EXITED => {
            let success = fields.u8()? == 1;
            Reply::Exited(Exit { success, text: fields.text()? })
        }
        KIND_FAILED => Reply::Failed(fields.text()?),
        KIND_TIME => {
            let utc = i64::from_be_bytes(fields.take(8)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?);
            let offset = i32::from_be_bytes(fields.take(4)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?);
            let time = LocalTime { utc, offset };
            if !time.is_plausible() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "an offset no zone has"));
            }
            Reply::Time(time)
        }
        KIND_KEY => Reply::Key(fields.take(COOKIE_KEY_BYTES)?.to_vec()),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "no such reply")),
    };
    fields.end()?;
    Ok(reply)
}

/// A frame's header: its kind, and its length if within `max`.
pub(crate) fn header(bytes: [u8; HEADER_BYTES], max: usize) -> io::Result<(u8, usize)> {
    let [kind, a, b, c, d] = bytes;
    let length = u32::from_be_bytes([a, b, c, d]) as usize;
    if length > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message too long"));
    }
    Ok((kind, length))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;
    use crate::broker::{HTTPS_PORT, MAX_REQUEST};
    use crate::{MAX_OFFSET, Role};

    fn round_trip(request: &Request) -> Request {
        let bytes = encode_request(request).unwrap();
        let (kind, length) = header(bytes[..HEADER_BYTES].try_into().unwrap(), MAX_REQUEST).unwrap();
        assert_eq!(length, bytes.len() - HEADER_BYTES);
        decode_request(kind, &bytes[HEADER_BYTES..]).unwrap()
    }

    #[test]
    fn every_request_and_reply_round_trips() {
        for request in [
            Request::Spawn { program: Program::Fetcher, local_network: true, arguments: vec!["http://a.example/".into(), "--proxy".into()] },
            Request::Kill(7),
            Request::Wait(9),
            Request::Read("/tmp/x.t42".into()),
            Request::OpenWeb("https://a.example/".into()),
            Request::LocalTime,
            Request::CookieKey,
            Request::Healthy,
            Request::UpdateState,
            Request::Spawn { program: Program::Updater, local_network: false, arguments: vec!["--proxy".into(), "http://proxy.example:3128".into()] },
        ] {
            assert_eq!(round_trip(&request), request);
        }
        for reply in [
            Reply::Spawned { id: 3, report: "sandbox-exec".into() },
            Reply::Done,
            Reply::Exited(Exit { success: false, text: "signal: 9".into() }),
            Reply::Source(vec![1, 2, 3]),
            Reply::Failed("no".into()),
            Reply::Time(LocalTime { utc: 1_792_656_300, offset: 39_600 }),
            Reply::Key(vec![9; COOKIE_KEY_BYTES]),
        ] {
            let bytes = encode_reply(&reply).unwrap();
            let (kind, _) = header(bytes[..HEADER_BYTES].try_into().unwrap(), MAX_REPLY).unwrap();
            assert_eq!(decode_reply(kind, bytes[HEADER_BYTES..].to_vec()).unwrap(), reply);
        }
    }

    /// A compromised core names only the browser's own programs, and every
    /// bound is held.
    #[test]
    fn a_request_out_of_bounds_is_refused() {
        assert!(decode_request(KIND_SPAWN, &[99, 0, 0, 0]).is_err(), "no such program");
        assert!(decode_request(KIND_SPAWN, &[1, 2, 0, 0]).is_err(), "a flag that is not one");
        let too_many = u16::try_from(MAX_ARGUMENTS + 1).unwrap().to_be_bytes();
        assert!(decode_request(KIND_SPAWN, &[1, 0, too_many[0], too_many[1]]).is_err());
        assert!(decode_request(KIND_KILL, &[0, 0, 0]).is_err(), "short");
        assert!(decode_request(KIND_KILL, &[0, 0, 0, 1, 0]).is_err(), "long");
        assert!(decode_request(KIND_READ, &[0, 2, 0xFF, 0xFE]).is_err(), "not UTF-8");
        assert!(decode_request(77, &[]).is_err());
        assert!(decode_request(KIND_LOCAL_TIME, &[0]).is_err(), "long");
        let mut beyond = 1_i64.to_be_bytes().to_vec();
        beyond.extend_from_slice(&(MAX_OFFSET + 1).to_be_bytes());
        assert!(decode_reply(KIND_TIME, beyond).is_err(), "an offset no zone has");
        assert!(encode_reply(&Reply::Time(LocalTime { utc: 0, offset: -MAX_OFFSET - 1 })).is_err());
        assert!(header([KIND_SPAWN, 0xFF, 0xFF, 0xFF, 0xFF], MAX_REQUEST).is_err(), "too long");
        // The role comes from the program, never from the core.
        assert_eq!(Program::Keyboard.role(), Role::Network);
        assert_eq!(Program::Sound.role(), Role::Sound);
        assert_eq!(Program::Updater.role(), Role::Updater { port: HTTPS_PORT }, "its port is the broker's rules', never the core's");
    }
}
