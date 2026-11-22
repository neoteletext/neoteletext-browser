//! The broker (decision 56): the one unconfined process, which the user
//! starts. It starts the trusted core confined, and then, on the core's
//! request, every helper, each confined as its program's role says, and
//! passes the core the helper's two pipes. It also reads a file channel's
//! bytes for the core and hands a web address to the system's browser for
//! it, so the core itself needs no files but its settings, no network and
//! no programs.
//!
//! A compromised core gains nothing from the broker: it can name only one
//! of the browser's own helpers, never a program or a role; the broker
//! confines each one by its program; a file is read through the broker's
//! own rules and bounds; a web address is checked by `web-opener` as
//! before. The broker parses only the core's requests, each bounded.
//!
//! The channel is a local-domain socket on macOS and Linux, which carries
//! the pipes as descriptors (`unix`), and two pipes on Windows, where the
//! broker duplicates the helper's pipe handles into the core's process
//! and sends their values.

use std::path::PathBuf;

use crate::Role;
#[cfg(unix)]
pub(crate) use crate::unix::Channel;
#[cfg(windows)]
pub(crate) use crate::windows::Channel;

mod client;
mod local_files;
pub(crate) mod server;
// A stand-in for tests of the trusted core (finding F-98), never shipped.
#[cfg(all(unix, feature = "test-support"))]
pub mod stand_in;
mod wire;

pub use client::{Broker, Helper};
pub(crate) use local_files::CHANNEL_VARIABLE;

/// The variable in which the process that starts a helper names its own
/// process number, so that on Linux a helper whose parent ended before the
/// helper first looked is seen to have been reparented (TODO-1278 item 2).
#[cfg(unix)]
pub(crate) const PARENT_VARIABLE: &str = "NEOTELETEXT_PARENT_PROCESS";
pub use local_files::{
    ChooseKind, Declaration, Declarer, MAX_DECLARATION, MAX_FILES_AT_ONCE, MAX_OPENING, Opening, decode_declaration, decode_opening, encode_declaration,
    encode_opening, read_opening, write_opening,
};
pub use server::{Core, Handlers, serve, start_core};
pub use wire::{Exit, Reply, Request, decode_reply, decode_request, encode_reply, encode_request};
pub(crate) use wire::{frame, header};

/// The most helpers alive at once: far more than tabs (`protocol::MAX_TABS`
/// is 64, each with a decoder and a fetcher), and few enough that a
/// compromised core cannot exhaust the machine.
pub const MAX_HELPERS: usize = 512;
/// The most arguments a helper is started with, and the longest.
pub const MAX_ARGUMENTS: usize = 64;
/// The longest argument, path or address in a request: twice the 4,096
/// bytes of the longest path, which holds an address of 8,000 octets
/// (neoteletext-common's REQ-1661) in a `neottx://` link. The trusted
/// core asserts that it does.
pub const MAX_TEXT: usize = 2 * 4096;
/// The largest reply: a file channel's bytes, at most 64 MB with a line
/// end for each of at most 4,096 files (`source.rs`), and room to spare.
pub const MAX_REPLY: usize = 64 * 1024 * 1024 + 64 * 1024;
/// A request is never more than its arguments.
const MAX_REQUEST: usize = MAX_ARGUMENTS * (MAX_TEXT + 2) + 16;
/// A frame's kind byte and big-endian length.
pub(crate) const HEADER_BYTES: usize = 5;

const KIND_SPAWN: u8 = 1;
const KIND_KILL: u8 = 2;
const KIND_WAIT: u8 = 3;
const KIND_READ: u8 = 4;
const KIND_OPEN_WEB: u8 = 5;
const KIND_LOCAL_TIME: u8 = 6;
// 7 was the cookie file's key, removed with cookies (DEC-6471); never
// reused.
const KIND_HEALTHY: u8 = 8;
const KIND_UPDATE_STATE: u8 = 9;
const KIND_CHOOSE: u8 = 10;
pub(crate) const KIND_SPAWNED: u8 = 11;
const KIND_DONE: u8 = 12;
const KIND_EXITED: u8 = 13;
const KIND_SOURCE: u8 = 14;
const KIND_FAILED: u8 = 15;
const KIND_TIME: u8 = 16;
const KIND_KEY: u8 = 17;
const KIND_OPENINGS: u8 = 18;
/// The reply that carries the pipe of what opens from outside.
pub(crate) const KIND_PIPE: u8 = 19;
/// A picture the user chose to convert (`docs/picture-tools.md`).
const KIND_READ_PICTURE: u8 = 20;
/// The converted page, written where the user chose to save it.
const KIND_SAVE: u8 = 21;
/// A font for pictures the user chose (renderer parity PAR-35), in the next
/// number free.
const KIND_READ_FONT: u8 = 25;
/// The script a page file opened from disk names (the script API's
/// specification section 12.7), in the next number free.
const KIND_READ_SCRIPT: u8 = 26;
/// The viewer key file's sealing key (neoteletext-common's
/// `docs/viewer-state.md` VS-108).
const KIND_VIEWER_KEYS_KEY: u8 = 22;
/// A helper's processor time, read by the broker from the operating
/// system's account of it for the core's "Page cost" (REQ-6626 item 1 a),
/// and its reply.
const KIND_PROCESSOR_TIME: u8 = 23;
const KIND_SPENT: u8 = 24;
/// The most bytes the broker saves for the core: a converted page, at
/// most `protocol::picture::MAX_PICTURE_PAGE`, which the core asserts fits.
pub const MAX_SAVE: usize = 256 * 1024;
/// The viewer key file's sealing key: 256 bits, a ChaCha20-Poly1305 key
/// (RFC 8439 section 2.8, VS-108).
pub const SEALING_KEY_BYTES: usize = 32;
/// The port of TLS, the update host's (`installation-and-updates.md`
/// section 8.1).
const HTTPS_PORT: u16 = 443;
/// The longest launcher state the broker hands the core: the state file's
/// own bound (`update::state::MAX_STATE`), with room.
pub const MAX_UPDATE_STATE: usize = 4 * 1024;

/// The chooser's first argument that asks it the desktop's reduced-motion
/// preference instead of showing a dialog (REQ-754; Linux, where the
/// confined interface reaches no session bus, decision 86).
pub const REDUCED_MOTION_ARGUMENT: &str = "reduced-motion";
/// What the chooser prints, and nothing else, when the desktop asks for
/// reduced motion. Anything else it prints, or its failure, means no
/// preference, so a chooser that cannot reach the portal changes nothing.
pub const REDUCED_MOTION_ANSWER: &[u8] = b"reduced";
/// The variable in which the broker tells the interface it starts that the
/// desktop asks for reduced motion (Linux). The broker sets it itself; it
/// is never passed on from the broker's own environment.
pub const REDUCED_MOTION_VARIABLE: &str = "NEOTELETEXT_REDUCED_MOTION";
/// The variable's one value that means reduced motion.
pub const REDUCED_MOTION_VALUE: &str = "1";

/// The browser's own helpers: the only programs the broker starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// A channel's decoder.
    Decoder,
    /// A web channel's fetcher.
    Fetcher,
    /// The window.
    Interface,
    /// The phone keyboard.
    Keyboard,
    /// The sound output (decision 55).
    Sound,
    /// The updater (`installation-and-updates.md` section 3).
    Updater,
    /// A (tab, channel)'s script host (`script-api.md` section 11.1,
    /// DEC-7266).
    ScriptHost,
}

/// What the broker's own rules give the updater: the folder it writes and
/// the one port it reaches (`installation-and-updates.md` section 8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdaterGrant {
    /// Its staging folder.
    pub staging: PathBuf,
    /// The update host's port, or the proxy's.
    pub port: u16,
}

impl Program {
    const ALL: [Self; 7] = [Self::Decoder, Self::Fetcher, Self::Interface, Self::Keyboard, Self::Sound, Self::Updater, Self::ScriptHost];

    /// The role it is confined as, decided here and never by the core; the
    /// updater's port is the broker's handler's (`Handlers::updater`), and
    /// this its role on the port of TLS.
    #[must_use]
    pub fn role(self) -> Role {
        match self {
            Self::Decoder => Role::Decoder,
            Self::Fetcher | Self::Keyboard => Role::Network,
            Self::Interface => Role::Interface,
            Self::Sound => Role::Sound,
            Self::Updater => Role::Updater { port: HTTPS_PORT },
            Self::ScriptHost => Role::Script,
        }
    }

    /// What it is called in what the browser reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Decoder => "decoder",
            Self::Fetcher => "fetcher",
            Self::Interface => "interface",
            Self::Keyboard => "phone keyboard",
            Self::Sound => "sound helper",
            Self::Updater => "updater",
            Self::ScriptHost => "script host",
        }
    }

    /// Its program's name, beside the broker's own.
    #[must_use]
    pub fn file_name(self) -> String {
        let name = match self {
            Self::Decoder => "neoteletext-decoder",
            Self::Fetcher => "neoteletext-fetcher",
            Self::Interface => "neoteletext-interface",
            Self::Keyboard => "neoteletext-keyboard",
            Self::Sound => "neoteletext-sound",
            Self::Updater => "neoteletext-updater",
            Self::ScriptHost => "neoteletext-script-host",
        };
        format!("{name}{}", std::env::consts::EXE_SUFFIX)
    }

    fn code(self) -> u8 {
        match self {
            Self::Decoder => 1,
            Self::Fetcher => 2,
            Self::Interface => 3,
            Self::Keyboard => 4,
            Self::Sound => 5,
            Self::Updater => 6,
            Self::ScriptHost => 7,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|program| program.code() == code)
    }
}
