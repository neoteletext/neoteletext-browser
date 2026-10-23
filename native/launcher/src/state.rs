//! The launcher's state file: read, rebuilt from what is installed when
//! it is missing or unreadable, and written whole.

use std::io;

use update::files;
use update::platform::Family;
use update::state::{MAX_STATE, State};
use update::version::Version;

#[cfg(target_os = "macos")]
use crate::macos;
use crate::{Launcher, io_text};

impl Launcher {
    /// The state, or one rebuilt from what is installed when the file is
    /// missing or unreadable (section 14 of the design).
    ///
    /// # Errors
    /// When there is no state and nothing installed to rebuild it from.
    pub fn read_state(&self) -> Result<State, String> {
        let path = self.layout.state();
        match files::read_bounded(&path, MAX_STATE) {
            Ok(bytes) => match State::parse(&bytes) {
                Ok(state) => return Ok(state),
                Err(refused) => eprintln!("neoteletext: the state file is not one ({refused}); rebuilding it from what is installed"),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_text(&path, &error)),
        }
        self.recovered()
    }

    /// A state naming the newest complete version installed.
    fn recovered(&self) -> Result<State, String> {
        if self.platform.family() == Family::Macos {
            #[cfg(target_os = "macos")]
            return macos::installed(self).map(State::installed);
        }
        let mut found: Vec<Version> = std::fs::read_dir(self.layout.versions())
            .map_err(|error| io_text(&self.layout.versions(), &error))?
            .filter_map(|entry| entry.ok()?.file_name().to_str().and_then(Version::parse))
            .collect();
        found.sort_unstable();
        found
            .into_iter()
            .rev()
            .find(|version| update::accept::complete(&self.layout.version(*version), self.platform).is_ok())
            .map(State::installed)
            .ok_or_else(|| String::from("no complete version is installed"))
    }

    /// Writes the state whole (requirement 13.4).
    ///
    /// # Errors
    /// When it cannot be written; the old state then stands.
    pub fn write_state(&self, state: &State) -> Result<(), String> {
        files::write_atomically(&self.layout.state(), state.to_text().as_bytes()).map_err(|error| io_text(&self.layout.state(), &error))
    }
}
