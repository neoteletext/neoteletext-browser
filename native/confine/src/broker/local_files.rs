//! What local files add to the broker's wire (`sandboxing.md` section 7,
//! DEC-1551): what the interface declares on its own channel, and what opens
//! from outside: a choice in the chooser, and what a second start hands
//! over. Each message is a kind byte, a length and a payload, every field
//! read within its bound. The first is input from the interface and the
//! second from any program that reaches the single-instance channel, so both
//! are fuzzed (`broker_declarations`, `broker_openings`).

use std::io::{self, Read as _};

use super::wire::{Fields, push_text};
use super::{Channel, HEADER_BYTES, MAX_TEXT, frame, header};

/// The most files one drop, one opening from the system or one second start
/// names (DEC-1549 item 9): a screenful of tabs, a quarter of all a window
/// may hold (`protocol::MAX_TABS`). Each may be 64 MB, so this also bounds
/// what one act can make the broker read to 1 GB. The rest are refused with
/// a notice.
pub const MAX_FILES_AT_ONCE: usize = 16;
/// What the interface's channel is told, the variable in its environment that
/// names its end.
pub(crate) const CHANNEL_VARIABLE: &str = "NEOTELETEXT_BROKER_CHANNEL";
/// The largest declaration: its paths, their lengths, its flag and its count.
pub const MAX_DECLARATION: usize = MAX_FILES_AT_ONCE * (MAX_TEXT + 2) + 16;
/// The largest opening: the same paths, and as many links.
pub const MAX_OPENING: usize = 2 * MAX_FILES_AT_ONCE * (MAX_TEXT + 2) + 16;

const KIND_DECLARE: u8 = 30;
const KIND_DECLARED: u8 = 31;
const KIND_CHOSEN: u8 = 32;
const KIND_OUTSIDE: u8 = 33;

/// What the chooser chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChooseKind {
    /// A file to open as a channel.
    File,
    /// A folder of page files to open as a channel.
    Folder,
    /// A bookmarks file to import (TODO-1426).
    Bookmarks,
}

impl ChooseKind {
    const ALL: [Self; 3] = [Self::File, Self::Folder, Self::Bookmarks];

    pub(super) fn code(self) -> u8 {
        match self {
            Self::File => 1,
            Self::Folder => 2,
            Self::Bookmarks => 3,
        }
    }

    pub(super) fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// How the chooser is told it on its command line.
    #[must_use]
    pub fn argument(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Bookmarks => "bookmarks",
        }
    }

    /// What the chooser's command line names, strictly.
    #[must_use]
    pub fn from_argument(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.argument() == text)
    }
}

/// The user's own act, as the interface tells the broker: paths dropped,
/// typed, or named by the system, as text the broker reads as an address
/// (DEC-1551 item 3). Each is a claim, which is why it allows a read only for
/// a short while.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The paths, as `file://` addresses or paths, at most
    /// [`MAX_FILES_AT_ONCE`].
    pub paths: Vec<String>,
    /// Their grants are kept: not those of a private window.
    pub keep: bool,
}

/// What opens from outside the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opening {
    /// The chooser's answer.
    Chosen {
        /// What was asked for.
        kind: ChooseKind,
        /// The path chosen.
        path: String,
    },
    /// What a second start names.
    Outside {
        /// `neoteletext://` links, which the core checks again.
        links: Vec<String>,
        /// Files, each already granted by the broker.
        files: Vec<String>,
    },
}

fn push_list(payload: &mut Vec<u8>, list: &[String]) -> io::Result<()> {
    if list.len() > MAX_FILES_AT_ONCE {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "too many paths"));
    }
    payload.extend_from_slice(&u16::try_from(list.len()).map_err(|_| io::ErrorKind::InvalidInput)?.to_be_bytes());
    list.iter().try_for_each(|text| push_text(payload, text))
}

fn list(fields: &mut Fields<'_>) -> io::Result<Vec<String>> {
    let count = usize::from(fields.u16()?);
    if count > MAX_FILES_AT_ONCE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many paths"));
    }
    (0..count).map(|_| fields.text()).collect()
}

/// A declaration as bytes.
///
/// # Errors
/// When it breaks a bound.
pub fn encode_declaration(declaration: &Declaration) -> io::Result<Vec<u8>> {
    let mut payload = vec![u8::from(declaration.keep)];
    push_list(&mut payload, &declaration.paths)?;
    frame(KIND_DECLARE, &payload)
}

/// A declaration read from its kind and payload, strictly: input from the
/// interface to the one process not confined.
///
/// # Errors
/// On any malformed or out-of-bounds message.
pub fn decode_declaration(kind: u8, payload: &[u8]) -> io::Result<Declaration> {
    if kind != KIND_DECLARE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no such message"));
    }
    let mut fields = Fields(payload);
    let keep = fields.flag()?;
    let paths = list(&mut fields)?;
    fields.end()?;
    Ok(Declaration { paths, keep })
}

/// The answer to a declaration, which says it was taken.
pub(super) fn acknowledgement() -> [u8; HEADER_BYTES] {
    [KIND_DECLARED, 0, 0, 0, 0]
}

/// An opening as bytes.
///
/// # Errors
/// When it breaks a bound.
pub fn encode_opening(opening: &Opening) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match opening {
        Opening::Chosen { kind, path } => {
            payload.push(kind.code());
            push_text(&mut payload, path)?;
            KIND_CHOSEN
        }
        Opening::Outside { links, files } => {
            push_list(&mut payload, links)?;
            push_list(&mut payload, files)?;
            KIND_OUTSIDE
        }
    };
    frame(kind, &payload)
}

/// An opening read from its kind and payload, strictly: what a second start
/// sends may come from any program on this computer that reaches the
/// channel.
///
/// # Errors
/// On any malformed or out-of-bounds message.
pub fn decode_opening(kind: u8, payload: &[u8]) -> io::Result<Opening> {
    let mut fields = Fields(payload);
    let opening = match kind {
        KIND_CHOSEN => {
            let kind = ChooseKind::from_code(fields.u8()?).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no such kind of choice"))?;
            Opening::Chosen { kind, path: fields.text()? }
        }
        KIND_OUTSIDE => {
            let links = list(&mut fields)?;
            Opening::Outside { links, files: list(&mut fields)? }
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "no such message")),
    };
    fields.end()?;
    Ok(opening)
}

/// Writes `opening` on a pipe or a socket.
///
/// # Errors
/// When it breaks a bound, or the other end has gone.
pub fn write_opening(pipe: &mut impl io::Write, opening: &Opening) -> io::Result<()> {
    pipe.write_all(&encode_opening(opening)?)?;
    pipe.flush()
}

/// The next opening on a pipe or a socket, within its bound.
///
/// # Errors
/// When the pipe ends, or the message is malformed.
pub fn read_opening(pipe: &mut impl io::Read) -> io::Result<Opening> {
    let mut bytes = [0_u8; HEADER_BYTES];
    pipe.read_exact(&mut bytes)?;
    let (kind, length) = header(bytes, MAX_OPENING)?;
    let mut payload = Vec::with_capacity(length);
    pipe.take(length as u64).read_to_end(&mut payload)?;
    if payload.len() != length {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    decode_opening(kind, &payload)
}

/// The interface's end of its own channel to the broker.
pub struct Declarer {
    channel: Channel,
}

impl Declarer {
    /// The channel the broker started this process with.
    ///
    /// # Errors
    /// When this process was not started by a broker.
    pub fn from_parent() -> io::Result<Self> {
        let value = std::env::var(CHANNEL_VARIABLE).map_err(|_| io::Error::other("no channel to the broker"))?;
        Ok(Self { channel: Channel::inherited(&value)? })
    }

    /// Tells the broker of `declaration`, and returns when it has taken it:
    /// so an intent sent after this cannot overtake it (DEC-1551 item 3).
    ///
    /// # Errors
    /// When it breaks a bound, or the broker closed the channel.
    pub fn declare(&mut self, declaration: &Declaration) -> io::Result<()> {
        self.channel.write(&encode_declaration(declaration)?)?;
        let (kind, _) = self.channel.read_frame(0)?;
        if kind == KIND_DECLARED { Ok(()) } else { Err(io::Error::new(io::ErrorKind::InvalidData, "an unexpected reply")) }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    fn paths(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("/tmp/{index}.tti")).collect()
    }

    #[test]
    fn a_declaration_and_an_opening_round_trip() {
        let declaration = Declaration { paths: paths(MAX_FILES_AT_ONCE), keep: true };
        let bytes = encode_declaration(&declaration).unwrap();
        assert!(bytes.len() <= MAX_DECLARATION + HEADER_BYTES);
        assert_eq!(decode_declaration(bytes[0], &bytes[HEADER_BYTES..]).unwrap(), declaration);
        for opening in [
            Opening::Chosen { kind: ChooseKind::Bookmarks, path: "/tmp/b.html".into() },
            Opening::Outside { links: vec!["neoteletext://https://a.example/".into()], files: paths(2) },
        ] {
            let mut pipe = Vec::new();
            write_opening(&mut pipe, &opening).unwrap();
            assert_eq!(read_opening(&mut pipe.as_slice()).unwrap(), opening);
        }
    }

    /// DEC-1551 item 3: the broker's parser for the interface's channel
    /// takes nothing out of bounds.
    #[test]
    fn a_message_out_of_bounds_is_refused() {
        assert!(encode_declaration(&Declaration { paths: paths(MAX_FILES_AT_ONCE + 1), keep: true }).is_err());
        let too_many = u16::try_from(MAX_FILES_AT_ONCE + 1).unwrap().to_be_bytes();
        assert!(decode_declaration(KIND_DECLARE, &[1, too_many[0], too_many[1]]).is_err(), "too many");
        assert!(decode_declaration(KIND_DECLARE, &[2, 0, 0]).is_err(), "a flag that is not one");
        assert!(decode_declaration(KIND_DECLARE, &[1, 0, 1, 0, 2, 0xFF, 0xFE]).is_err(), "not UTF-8");
        assert!(decode_declaration(KIND_DECLARE, &[1, 0, 0, 0]).is_err(), "long");
        assert!(decode_declaration(KIND_DECLARE, &[1]).is_err(), "short");
        assert!(decode_declaration(99, &[1, 0, 0]).is_err(), "no such message");
        assert!(decode_opening(KIND_CHOSEN, &[9, 0, 0]).is_err(), "no such kind");
        assert!(decode_opening(KIND_OUTSIDE, &[too_many[0], too_many[1], 0, 0]).is_err(), "too many");
        assert!(read_opening(&mut [KIND_OUTSIDE, 0xFF, 0xFF, 0xFF, 0xFF].as_slice()).is_err(), "too long");
    }

    #[test]
    fn a_kind_of_choice_is_named_strictly() {
        for kind in ChooseKind::ALL {
            assert_eq!(ChooseKind::from_argument(kind.argument()), Some(kind));
        }
        assert_eq!(ChooseKind::from_argument("--file"), None);
    }

    /// DEC-1551 item 3 on a real channel: the broker's side takes the
    /// declaration before it acknowledges, so what the interface does next
    /// finds it taken; a malformed message closes the channel.
    #[cfg(unix)]
    #[test]
    fn a_declaration_is_taken_before_it_is_acknowledged() {
        use std::os::unix::net::UnixStream;
        use std::sync::{Arc, Mutex};

        let (ours, theirs) = UnixStream::pair().unwrap();
        let taken = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&taken);
        crate::broker::server::hear_declarations(Channel::new(ours), Arc::new(move |declaration| recorded.lock().unwrap().push(declaration)));
        let mut declarer = Declarer { channel: Channel::new(theirs) };
        let declaration = Declaration { paths: paths(1), keep: false };
        declarer.declare(&declaration).unwrap();
        assert_eq!(*taken.lock().unwrap(), [declaration], "taken by the time it returns");
        declarer.channel.write(&[99, 0, 0, 0, 0]).unwrap();
        assert!(declarer.channel.read_frame(0).is_err(), "the channel closed");
        assert_eq!(taken.lock().unwrap().len(), 1);
    }
}
