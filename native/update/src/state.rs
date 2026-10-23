//! The launcher's state (`installation-and-updates.md` section 3): which
//! version is current, which was before it, which is on trial, how many
//! of its starts failed after its broker said it had started, how many
//! never said so, and whether one is running, and which versions are
//! known bad. It is the pointer of requirement 13.4: written whole to a
//! new file, flushed, and renamed over the old one, so after a power cut
//! it names a complete version, the old or the new.
//!
//! ```text
//! neoteletext-state 2
//! current 0.2.0
//! previous 0.1.0
//! trial 0.3.0 1 0 0
//! rolled-back 0.2.5
//! bad 0.2.5
//! ```
//!
//! The trial line is `trial <version> <failed> <not started> <running>`,
//! the last `1` while a start of the trial that said it started has not
//! ended (decision 93). Format 1, whose trial line is
//! `trial <version> <failed>`, is still read, so a launcher that writes
//! format 2 reads what the launcher before it wrote.
//!
//! Only the launcher writes it; it is read strictly all the same, since
//! anything running as the user can write it too.

use std::fmt::Write as _;

use crate::version::Version;

/// The first line: the format and its version.
const HEADER: &str = "neoteletext-state 2";
/// The first line of format 1, still read.
const HEADER_1: &str = "neoteletext-state 1";
/// The failed starts after which a trial version is rolled back
/// (requirement 13.5: "fails to start twice"): starts whose broker said
/// it had started and that then ended without being healthy, in time or
/// not at all.
pub const FAILED_STARTS_TO_ROLL_BACK: u8 = 2;
/// The starts whose broker never said it had started, before its start
/// deadline or before it ended, after which a trial version is rolled
/// back (decision 93): a version that cannot even start is rolled back,
/// one more chance given than a crash after starting, since a start that
/// the system holds up for its own checks is counted here too.
pub const NOT_STARTED_TO_ROLL_BACK: u8 = 3;
/// The most failed starts the file may say: one more than counted, as
/// format 1, which counted a start before it was made, may say.
const MAX_FAILED: u8 = FAILED_STARTS_TO_ROLL_BACK + 1;
/// The most starts that never started the file may say: one more than
/// rolls back, since such a start is counted before it is made.
const MAX_NOT_STARTED: u8 = NOT_STARTED_TO_ROLL_BACK + 1;
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
    /// How many of its starts said they had started and then ended
    /// without being found healthy.
    pub failed: u8,
    /// How many starts in a row never said they had started: counted
    /// before each start, cleared when one says it.
    pub not_started: u8,
    /// A start that said it had started is running, or the launcher
    /// running it ended before it did: then it is counted as failed at
    /// the next start.
    pub running: bool,
}

impl Trial {
    /// A version just accepted, not yet started.
    #[must_use]
    pub const fn new(version: Version) -> Self {
        Self { version, failed: 0, not_started: 0, running: false }
    }
}

/// One decimal digit, at most `max`.
fn digit(text: &str, max: u8) -> Option<u8> {
    match text.as_bytes() {
        [digit @ b'0'..=b'9'] => Some(digit.wrapping_sub(b'0')).filter(|value| *value <= max),
        _ => None,
    }
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
        let format_1 = match lines.next() {
            Some(HEADER) => false,
            Some(HEADER_1) => true,
            _ => return Err(Refused("not a state file of this format")),
        };
        let current = lines.next().and_then(|line| line.strip_prefix("current ")).and_then(Version::parse).ok_or(Refused("no current version"))?;
        let mut state = Self::installed(current);
        if let Some(previous) = lines.next_if(|line| line.starts_with("previous ")) {
            state.previous = Some(previous.strip_prefix("previous ").and_then(Version::parse).ok_or(Refused("a previous version that is not one"))?);
        }
        if let Some(trial) = lines.next_if(|line| line.starts_with("trial ")) {
            let fields: Vec<&str> = trial.split(' ').skip(1).collect();
            let (version, failed, not_started, running) = match (format_1, fields.as_slice()) {
                (true, [version, failed]) => (*version, *failed, "0", "0"),
                (false, [version, failed, not_started, running]) => (*version, *failed, *not_started, *running),
                _ => return Err(Refused("a trial line that is not one")),
            };
            let version = Version::parse(version).ok_or(Refused("a trial version that is not one"))?;
            let failed = digit(failed, MAX_FAILED).ok_or(Refused("a count of failed starts that is not one"))?;
            let not_started = digit(not_started, MAX_NOT_STARTED).ok_or(Refused("a count of starts that never started that is not one"))?;
            let running = match running {
                "0" => false,
                "1" => true,
                _ => return Err(Refused("a trial's running mark that is not one")),
            };
            state.trial = Some(Trial { version, failed, not_started, running });
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
            let _ = writeln!(
                text,
                "trial {} {} {} {}",
                trial.version,
                trial.failed.min(MAX_FAILED),
                trial.not_started.min(MAX_NOT_STARTED),
                u8::from(trial.running)
            );
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
            trial: Some(Trial { version: Version::new(0, 3, 0), failed: 1, not_started: 2, running: true }),
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
        assert!(good.contains("\ntrial 0.3.0 1 2 1\n"), "{good}");
        let refused = |text: &str| State::parse(text.as_bytes()).is_err();
        assert!(!refused(&good));
        for bad in [
            good.trim_end().to_owned(),
            good.replace("current 0.2.0", "current 0.2"),
            good.replace("trial 0.3.0 1 2 1", "trial 0.3.0 9 2 1"),
            good.replace("trial 0.3.0 1 2 1", "trial 0.3.0 10 2 1"),
            good.replace("trial 0.3.0 1 2 1", "trial 0.3.0 1 5 1"),
            good.replace("trial 0.3.0 1 2 1", "trial 0.3.0 1 2 2"),
            good.replace("trial 0.3.0 1 2 1", "trial 0.3.0 1 2"),
            good.replace("trial 0.3.0 1 2 1", "trial 0.3.0 1"),
            good.replace("trial 0.3.0 1 2 1", "trial 0.3.0 1 2 1 0"),
            good.replace("neoteletext-state 2", "neoteletext-state 1"),
            good.replace("bad 0.2.5\nbad 0.2.6", "bad 0.2.6\nbad 0.2.5"),
            good.replace("bad 0.2.6", "bad 0.2.5"),
            good.replace("previous 0.1.0\n", "") + "previous 0.1.0\n",
            good.replace("neoteletext-state 2", "neoteletext-state 3"),
            good.replace('\n', "\r\n"),
            good.clone() + "\n",
        ] {
            assert!(refused(&bad), "{bad:?}");
        }
    }

    /// Decision 93: the trial's two counts and its running mark survive the
    /// file, and format 1, which the launcher before wrote, is still read,
    /// its trial neither started nor running.
    #[test]
    fn a_trial_round_trips_and_format_1_is_read() {
        let state = full();
        assert_eq!(State::parse(state.to_text().as_bytes()), Ok(state));
        let old = "neoteletext-state 1\ncurrent 0.2.0\ntrial 0.3.0 1\nbad 0.2.5\n";
        let read = State::parse(old.as_bytes()).unwrap();
        assert_eq!(read.trial, Some(Trial { version: Version::new(0, 3, 0), failed: 1, not_started: 0, running: false }));
        assert!(read.is_bad(Version::new(0, 2, 5)));
        assert!(read.to_text().starts_with("neoteletext-state 2\n"), "written in the new format");
        assert!(State::parse(b"neoteletext-state 1\ncurrent 0.2.0\ntrial 0.3.0 1 0 0\n").is_err(), "format 1 has two fields");
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
