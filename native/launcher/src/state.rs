//! The launcher's state file: read, rebuilt from what is installed when
//! it is missing or unreadable, and written whole.

use std::io;

use update::files;
use update::platform::Family;
use update::state::{MAX_STATE, State, Trial};
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
                Ok(state) if self.installed(&state) => return Ok(state),
                Ok(state) => eprintln!("neoteletext: the state names version {}, which is not installed; rebuilding it from what is", state.current),
                Err(refused) => eprintln!("neoteletext: the state file is not one ({refused}); rebuilding it from what is installed"),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            // Longer than any state: not one either (finding F-80).
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                eprintln!("neoteletext: the state file is not one ({error}); rebuilding it from what is installed");
            }
            Err(error) => return Err(io_text(&path, &error)),
        }
        self.recovered()
    }

    /// Whether the current version `state` names is installed: on Windows
    /// and Linux, its folder is there; on macOS the bundle says which it is
    /// and `macos::reconcile` makes it so.
    fn installed(&self, state: &State) -> bool {
        self.platform.family() == Family::Macos || self.layout.version(state.current).is_dir()
    }

    /// A state rebuilt from what is installed, each version accepted again
    /// by the pinned signer (finding F-80): the newest becomes the trial,
    /// since nothing says it was ever found healthy, with the next newer
    /// one current; a version alone is current. On macOS the installed
    /// bundle, which the system checks at each start, is current.
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
        let mut accepted = found
            .into_iter()
            .rev()
            .filter(|version| update::accept::accept(&self.layout.version(*version), self.platform, self.signer.as_ref()).is_ok_and(|said| said == *version));
        let newest = accepted.next().ok_or_else(|| String::from("no complete version is installed"))?;
        Ok(match accepted.next() {
            Some(older) => State { trial: Some(Trial::new(newest)), ..State::installed(older) },
            None => State::installed(newest),
        })
    }

    /// Writes the state whole (requirement 13.4).
    ///
    /// # Errors
    /// When it cannot be written; the old state then stands.
    pub fn write_state(&self, state: &State) -> Result<(), String> {
        files::write_atomically(&self.layout.state(), state.to_text().as_bytes()).map_err(|error| io_text(&self.layout.state(), &error))
    }
}
