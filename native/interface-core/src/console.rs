//! The developer console's entries as the trusted core sends them
//! (`settings-home-console-proxy.md` section 3): cleaned text for each of
//! its tabs, which the interface only shows.

use alloc::string::String;
use alloc::vec::Vec;

/// The most entries a tab keeps for a channel (requirement 19.8).
pub const KEPT: usize = 1000;

/// The longest entry: a packet's hexadecimal and text with its address.
pub const MAX_ENTRY: usize = 240;

/// The console's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConsoleTab {
    /// Each exchange the fetcher made, and the push connection's events.
    Network,
    /// Each packet received.
    Packets,
    /// The pages the decoder holds.
    Pages,
    /// The browser's log, as a web browser's console: each process's
    /// notices, refusals, warnings and errors, and how each was confined
    /// (decision 70; it was the Notices tab).
    Log,
}

/// How much a log line matters, as a web browser's console sorts them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Something failed.
    Error,
    /// Something was refused or may not work.
    Warning,
    /// What happened.
    #[default]
    Info,
}

impl Level {
    /// Every level, the most serious first.
    pub const ALL: [Self; 3] = [Self::Error, Self::Warning, Self::Info];

    /// Its label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "Errors",
            Self::Warning => "Warnings",
            Self::Info => "Info",
        }
    }
}

impl ConsoleTab {
    /// Every tab, in the order the console shows them.
    pub const ALL: [Self; 4] = [Self::Network, Self::Packets, Self::Pages, Self::Log];

    /// Its label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Packets => "Packets",
            Self::Pages => "Pages",
            Self::Log => "Log",
        }
    }

    /// Its place in [`Self::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Network => 0,
            Self::Packets => 1,
            Self::Pages => 2,
            Self::Log => 3,
        }
    }
}

/// One entry of a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleEntry {
    /// Its tab.
    pub tab: ConsoleTab,
    /// How much it matters: information in every tab but the log.
    pub level: Level,
    /// Milliseconds since the trusted core started.
    pub at_ms: u64,
    /// What it says, cleaned text.
    pub text: String,
}

/// One figure of the Stats tab: what is counted, and how much.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    /// What is counted, cleaned text.
    pub label: String,
    /// How much, written out, cleaned text.
    pub value: String,
}

/// The most figures the Stats tab shows: the channel's twelve, and the
/// "Page cost" section of REQ-6626 with room to spare.
pub const MAX_STATS: usize = 48;

/// New entries for a channel's tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleUpdate {
    /// The channel.
    pub channel: u32,
    /// The entries replace those shown, rather than follow them.
    pub replace: bool,
    /// The entries, oldest first.
    pub entries: Vec<ConsoleEntry>,
    /// The channel's figures for the Stats tab, in place of those shown;
    /// `None` when they have not changed.
    pub stats: Option<Vec<Stat>>,
}

/// What a recording of the open tab's traffic is doing (REQ-6406).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordingState {
    /// None: the console is closed, or one could not be started.
    #[default]
    Off,
    /// Every message is being written.
    Recording,
    /// Open, and nothing is written until it is resumed.
    Paused,
    /// Ended, its file closed.
    Stopped,
    /// The tab is private, and is never recorded.
    Private,
}

impl RecordingState {
    /// Every state, in the order of their bytes between processes.
    pub const ALL: [Self; 5] = [Self::Off, Self::Recording, Self::Paused, Self::Stopped, Self::Private];
}

/// The recording, as the console's bar shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordingView {
    /// What it is doing.
    pub state: RecordingState,
    /// The bytes written to its file so far.
    pub bytes: u64,
    /// Its file's name, without its folder; empty when there is none.
    pub name: String,
}

/// The most bytes of a recording's file name shown.
pub const MAX_RECORDING_NAME: usize = 160;

/// What a developer asks of the recording from the console's bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingRequest {
    /// Write nothing until resumed.
    Pause,
    /// Write again.
    Resume,
    /// End it and close its file.
    Stop,
    /// Start a new one.
    Record,
}

impl RecordingRequest {
    /// Every request, in the order of their bytes between processes.
    pub const ALL: [Self; 4] = [Self::Pause, Self::Resume, Self::Stop, Self::Record];
}
