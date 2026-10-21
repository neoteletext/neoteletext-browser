//! The developer console's entries as the trusted core sends them
//! (`settings-home-console-proxy.md` section 3): cleaned text for each of
//! its tabs, which the interface only shows.

use alloc::string::String;
use alloc::vec::Vec;

/// The most entries a tab keeps for a channel (requirement 19.8).
pub const KEPT: usize = 1000;

/// The console's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConsoleTab {
    /// Each exchange the fetcher made, and the push connection's events.
    Network,
    /// Each packet received.
    Packets,
    /// The pages the decoder holds.
    Pages,
    /// Notices, refusals and warnings.
    Notices,
}

impl ConsoleTab {
    /// Every tab, in the order the console shows them.
    pub const ALL: [Self; 4] = [Self::Network, Self::Packets, Self::Pages, Self::Notices];

    /// Its label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Packets => "Packets",
            Self::Pages => "Pages",
            Self::Notices => "Notices",
        }
    }

    /// Its place in [`Self::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Network => 0,
            Self::Packets => 1,
            Self::Pages => 2,
            Self::Notices => 3,
        }
    }
}

/// One entry of a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleEntry {
    /// Its tab.
    pub tab: ConsoleTab,
    /// Milliseconds since the trusted core started.
    pub at_ms: u64,
    /// What it says, cleaned text.
    pub text: String,
}

/// New entries for a channel's tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleUpdate {
    /// The channel.
    pub channel: u32,
    /// The entries replace those shown, rather than follow them.
    pub replace: bool,
    /// The entries, oldest first.
    pub entries: Vec<ConsoleEntry>,
}
