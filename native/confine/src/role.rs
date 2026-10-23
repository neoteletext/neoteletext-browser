//! What a helper is for, and how it reports what confines it.

use std::fmt;

/// What a helper is for, which is all it may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A decoder: its pipes and nothing else.
    Decoder,
    /// A fetcher or the phone keyboard: the network, with the system's
    /// name resolution, and nothing else.
    Network,
    /// The interface: the window system, fonts, text input, the
    /// pasteboard and the Dock; no network, no sound (decision 55).
    Interface,
    /// The trusted core: its settings folder and nothing else; no
    /// network, no programs (decision 56).
    Core,
    /// The sound helper: the sound output and nothing else (decision 55).
    Sound,
    /// The updater (`installation-and-updates.md` section 8.1): TCP to one
    /// port, the update host's or the user's proxy's, with the system's
    /// name resolution, and its staging folder written; nothing else.
    Updater {
        /// The one port it may connect to.
        port: u16,
    },
}

impl Role {
    /// The helper's name in what it reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Decoder => "decoder",
            Self::Network => "fetcher",
            Self::Interface => "interface",
            Self::Core => "trusted core",
            Self::Sound => "sound",
            Self::Updater { .. } => "updater",
        }
    }
}

/// What confines a process, as it reports it: every layer its role names,
/// since a process with fewer is not started (decision 58).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The layers in force, in words ("seccomp", "Landlock").
    pub layers: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.layers.join(", "))
    }
}
