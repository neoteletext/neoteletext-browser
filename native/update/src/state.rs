//! The launcher's state (`installation-and-updates.md` section 3): which
//! version is current, which was before it, which is on trial and how
//! many times it failed to start, and which versions are known bad. It is
//! the pointer of requirement 13.4: written whole to a new file, flushed,
//! and renamed over the old one, so after a power cut it names a complete
//! version, the old or the new.
//!
//! ```text
//! neoteletext-state 1
//! current 0.2.0
//! previous 0.1.0
//! trial 0.3.0 1
//! rolled-back 0.2.5
//! bad 0.2.5
//! ```
//!
//! Only the launcher writes it; it is read strictly all the same, since
//! anything running as the user can write it too.

use std::fmt::Write as _;

use crate::version::Version;

/// The first line: the format and its version.
const HEADER: &str = "neoteletext-state 1";
/// The failed starts after which a trial version is rolled back
/// (requirement 13.5: "fails to start twice").
pub const FAILED_STARTS_TO_ROLL_BACK: u8 = 2;
/// The most failed starts the file may say: one more than counted, since
/// a start is counted before it is made.
const MAX_FAILED: u8 = FAILED_STARTS_TO_ROLL_BACK + 1;
/// The most versions remembered as bad; the oldest is forgotten first,
/// since only a version newer than the current one is ever offered.
pub const MAX_BAD: usize = 32;
/// The longest state file: every line at its longest, with room.
pub const MAX_STATE: usize = 2 * 1024;
const _: () = assert!((MAX_BAD + 5) * (crate::version::MAX_TEXT + 16) <= MAX_STATE, "the bound holds every line");

/// A version on trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trial {
    /// Which.
    pub version: Version,
    /// How many of its starts did not commit.
    pub failed: u8,
}

/// The launcher's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// The version that runs when no trial does: never touched by an
    /// update until a trial commits.
    pub current: Version,
    /// The version before it, kept for one more update (section 3.1, step 7).
    pub previous: Option<Version>,
    /// The version being tried, if any.
    pub trial: Option<Trial>,
    /// A version rolled back, which the strip says once.
    pub rolled_back: Option<Version>,
    /// Versions never to be staged or started again, oldest first.
    pub bad: Vec<Version>,
}

/// Why a state file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl State {
    /// The state of a fresh installation of `version`.
    #[must_use]
    pub const fn installed(version: Version) -> Self {
        Self { current: version, previous: None, trial: None, rolled_back: None, bad: Vec::new() }
    }

    /// Reads a state file strictly.
    ///
    /// # Errors
    /// On anything the format does not allow.
    pub fn parse(bytes: &[u8]) -> Result<Self, Refused> {
        if bytes.len() > MAX_STATE {
            return Err(Refused("the state file is too long"));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Refused("the state file is not text"))?;
        let body = text.strip_suffix('\n').ok_or(Refused("the state file does not end its last line"))?;
        let mut lines = body.split('\n').peekable();
        if lines.next() != Some(HEADER) {
            return Err(Refused("not a state file of this format"));
        }
        let current = lines.next().and_then(|line| line.strip_prefix("current ")).and_then(Version::parse).ok_or(Refused("no current version"))?;
        let mut state = Self::installed(current);
        if let Some(previous) = lines.next_if(|line| line.starts_with("previous ")) {
            state.previous = Some(previous.strip_prefix("previous ").and_then(Version::parse).ok_or(Refused("a previous version that is not one"))?);
        }
        if let Some(trial) = lines.next_if(|line| line.starts_with("trial ")) {
            let mut fields = trial.split(' ').skip(1);
            let (Some(version), Some(failed), None) = (fields.next(), fields.next(), fields.next()) else {
                return Err(Refused("a trial line that is not one"));
            };
            let version = Version::parse(version).ok_or(Refused("a trial version that is not one"))?;
            let failed = match failed.as_bytes() {
                [digit @ b'0'..=b'9'] => digit.wrapping_sub(b'0'),
                _ => return Err(Refused("a count of failed starts that is not one")),
            };
            if failed > MAX_FAILED {
                return Err(Refused("more failed starts than are counted"));
            }
            state.trial = Some(Trial { version, failed });
        }
        if let Some(rolled_back) = lines.next_if(|line| line.starts_with("rolled-back ")) {
            state.rolled_back =
                Some(rolled_back.strip_prefix("rolled-back ").and_then(Version::parse).ok_or(Refused("a rolled-back version that is not one"))?);
        }
        for line in lines {
            if state.bad.len() >= MAX_BAD {
                return Err(Refused("too many bad versions"));
            }
            let bad = line.strip_prefix("bad ").and_then(Version::parse).ok_or(Refused("a line that is not a bad version"))?;
            if state.bad.last().is_some_and(|last| *last >= bad) {
                return Err(Refused("the bad versions are not each once in order"));
            }
            state.bad.push(bad);
        }
        Ok(state)
    }

    /// The file's text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{HEADER}\ncurrent {}\n", self.current);
        if let Some(previous) = self.previous {
            let _ = writeln!(text, "previous {previous}");
        }
        if let Some(trial) = self.trial {
            let _ = writeln!(text, "trial {} {}", trial.version, trial.failed.min(MAX_FAILED));
        }
        if let Some(rolled_back) = self.rolled_back {
            let _ = writeln!(text, "rolled-back {rolled_back}");
        }
        for bad in &self.bad {
            let _ = writeln!(text, "bad {bad}");
        }
        text
    }

    /// Whether `version` is known bad.
    #[must_use]
    pub fn is_bad(&self, version: Version) -> bool {
        self.bad.contains(&version)
    }

    /// Marks `version` bad, forgetting the oldest beyond [`MAX_BAD`].
    pub fn mark_bad(&mut self, version: Version) {
        if !self.is_bad(version) {
            self.bad.push(version);
            self.bad.sort_unstable();
        }
        while self.bad.len() > MAX_BAD {
            self.bad.remove(0);
        }
    }

    /// The version a start runs: the trial's, else the current one.
    #[must_use]
    pub fn running(&self) -> Version {
        self.trial.map_or(self.current, |trial| trial.version)
    }

    /// The versions that must be kept installed.
    #[must_use]
    pub fn kept(&self) -> Vec<Version> {
        let mut kept = vec![self.current];
        kept.extend(self.previous);
        kept.extend(self.trial.map(|trial| trial.version));
        kept
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    fn full() -> State {
        State {
            current: Version::new(0, 2, 0),
            previous: Some(Version::new(0, 1, 0)),
            trial: Some(Trial { version: Version::new(0, 3, 0), failed: 1 }),
            rolled_back: Some(Version::new(0, 2, 5)),
            bad: vec![Version::new(0, 2, 5), Version::new(0, 2, 6)],
        }
    }

    #[test]
    fn a_state_round_trips() {
        for state in [full(), State::installed(Version::new(0, 1, 0))] {
            assert_eq!(State::parse(state.to_text().as_bytes()), Ok(state));
        }
        assert_eq!(full().running(), Version::new(0, 3, 0));
        assert_eq!(full().kept(), vec![Version::new(0, 2, 0), Version::new(0, 1, 0), Version::new(0, 3, 0)]);
    }

    #[test]
    fn anything_the_format_does_not_allow_is_refused() {
        let good = full().to_text();
        let refused = |text: &str| State::parse(text.as_bytes()).is_err();
        assert!(!refused(&good));
        for bad in [
            good.trim_end().to_owned(),
            good.replace("current 0.2.0", "current 0.2"),
            good.replace("trial 0.3.0 1", "trial 0.3.0 9"),
            good.replace("trial 0.3.0 1", "trial 0.3.0 10"),
            good.replace("trial 0.3.0 1", "trial 0.3.0"),
            good.replace("bad 0.2.5\nbad 0.2.6", "bad 0.2.6\nbad 0.2.5"),
            good.replace("bad 0.2.6", "bad 0.2.5"),
            good.replace("previous 0.1.0\n", "") + "previous 0.1.0\n",
            good.replace("neoteletext-state 1", "neoteletext-state 2"),
            good.replace('\n', "\r\n"),
            good.clone() + "\n",
        ] {
            assert!(refused(&bad), "{bad:?}");
        }
    }

    #[test]
    fn the_oldest_bad_version_is_forgotten_first() {
        let mut state = State::installed(Version::new(1, 0, 0));
        for patch in 0..=u16::try_from(MAX_BAD).unwrap() {
            state.mark_bad(Version::new(2, 0, patch));
        }
        state.mark_bad(Version::new(2, 0, 5));
        assert_eq!(state.bad.len(), MAX_BAD);
        assert!(!state.is_bad(Version::new(2, 0, 0)));
        assert!(state.is_bad(Version::new(2, 0, 1)));
        assert_eq!(State::parse(state.to_text().as_bytes()), Ok(state));
    }
}
