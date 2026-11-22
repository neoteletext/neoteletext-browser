//! What a helper is for, and how it reports what confines it.

use std::fmt;

/// The most memory the operating system lets a script host's process hold
/// (`script-api.md` section 10.6, `SCRIPT_PROCESS_MEMORY_BYTES`): 128 MiB,
/// twice the 64 MiB of the WebAssembly memory a container may grow to,
/// which leaves the interpreter's compiled module, the host's state
/// (`MAX_HOST_STATE_BYTES`, 8 MiB) and the process's own room. The script
/// library names the same figure, and the script-host program asserts
/// that the two agree.
pub const SCRIPT_MEMORY_BYTES: usize = 128 * 1024 * 1024;

/// What a helper is for, which is all it may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A decoder: its pipes and nothing else.
    Decoder,
    /// A script host (`script-api.md` section 11.1, DEC-7266): exactly a
    /// decoder's confinement, its pipes and nothing else, and the
    /// operating system's limit of [`SCRIPT_MEMORY_BYTES`] on its memory.
    /// A role of its own, so that the two differ only by a recorded
    /// decision.
    Script,
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
            Self::Script => "script host",
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
