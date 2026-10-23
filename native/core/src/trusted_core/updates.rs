//! Updates in the trusted core (`installation-and-updates.md` sections 3
//! and 3.4): it tells the updater when to look, offers a newer version in
//! the strip once and in the settings panel always, has it staged only
//! once the user accepts, and finds a version healthy (section 3.2). It
//! parses none of the updater's formats: only its messages, each bounded
//! (`update::wire`), and the launcher's state, read strictly.

use std::fmt::Write as _;
use std::io::{BufReader, Write};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use confine::broker::{Broker, Helper, Program};
use update::state::State;
use update::version::Version;
use update::wire::{FromUpdater, ToUpdater, read_from_updater, write_to_updater};

use super::Event;

/// How often the updater looks while the browser runs (section 3.4).
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The time a version has to show it is healthy (section 3.2).
pub const HEALTH_WINDOW: Duration = Duration::from_secs(30);
/// How long after its first frame a version's helpers are given to fail
/// before it is found healthy: a helper that cannot confine itself ends
/// at once.
pub const HEALTH_SETTLE: Duration = Duration::from_secs(2);
/// After this long without a successful check, the panel says "Updates
/// unavailable" (section 9.3): the timestamp's life.
pub const UNAVAILABLE_AFTER: i64 = update::wire::TIMESTAMP_DAYS * 24 * 60 * 60;

/// What the core knows of a version's health (section 3.2).
#[derive(Debug)]
pub struct Health {
    started: Instant,
    drawn: Option<Instant>,
    settings_read: bool,
    failed: Option<String>,
    told: bool,
}

impl Health {
    fn new(settings_read: bool) -> Self {
        Self { started: Instant::now(), drawn: None, settings_read, failed: None, told: false }
    }

    /// Whether the version is healthy now: its window drawn, its settings
    /// read, no helper failed, within the window, and settled.
    #[must_use]
    pub fn healthy(&self, now: Instant) -> bool {
        let Some(drawn) = self.drawn else { return false };
        self.settings_read
            && self.failed.is_none()
            && drawn.saturating_duration_since(self.started) <= HEALTH_WINDOW
            && now.saturating_duration_since(drawn) >= HEALTH_SETTLE
    }

    /// When the health is next to be looked at.
    fn due(&self) -> Option<Instant> {
        if self.told || self.failed.is_some() || !self.settings_read {
            return None;
        }
        self.drawn.and_then(|drawn| drawn.checked_add(HEALTH_SETTLE))
    }
}

/// Updates, as the core keeps them.
pub struct Updates {
    /// The launcher's state, when a launcher started the browser.
    pub launched: Option<State>,
    /// Why the launcher's state was refused, when it was.
    pub state_refused: Option<String>,
    updater: Option<(Helper, Box<dyn Write + Send>)>,
    latest: Option<Version>,
    last_success: Option<i64>,
    failure: Option<String>,
    downloading: Option<Version>,
    staged: Option<Version>,
    /// The version the strip offered this run, once.
    offered: Option<Version>,
    /// The strip shows the offer now.
    pub strip: bool,
    next_check: Option<Instant>,
    /// The version's health.
    pub health: Health,
    /// This build carries The Update Framework's root, so it can update.
    pub keyed: bool,
}

impl std::fmt::Debug for Updates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updates").field("launched", &self.launched).field("latest", &self.latest).finish_non_exhaustive()
    }
}

impl Default for Updates {
    fn default() -> Self {
        Self::new(None, None, true)
    }
}

impl Updates {
    /// Updates of a browser the launcher started with `launched`.
    #[must_use]
    pub fn new(launched: Option<State>, state_refused: Option<String>, settings_read: bool) -> Self {
        Self {
            launched,
            state_refused,
            updater: None,
            latest: None,
            last_success: None,
            failure: None,
            downloading: None,
            staged: None,
            offered: None,
            strip: false,
            next_check: None,
            health: Health::new(settings_read),
            keyed: release_identity::TUF_ROOT.is_some(),
        }
    }

    /// Reads the launcher's state through the broker.
    #[must_use]
    pub fn of_broker(broker: &Broker, settings_read: bool) -> Self {
        match broker.update_state() {
            Ok(bytes) => match State::parse(&bytes) {
                Ok(state) => Self::new(Some(state), None, settings_read),
                Err(refused) => Self::new(None, Some(format!("the launcher's state is not one: {refused}")), settings_read),
            },
            // No launcher started the browser: it does not update itself.
            Err(_) => Self::new(None, None, settings_read),
        }
    }

    /// The version running.
    #[must_use]
    pub fn running(&self) -> Option<Version> {
        self.launched.as_ref().map(State::running)
    }

    /// Starts the updater and asks it to look, if this browser was
    /// installed by the launcher and the user lets it check (section 3.4).
    pub fn start(&mut self, broker: &Arc<Broker>, check: bool, proxy: Option<&str>, events: &Sender<Event>) {
        if !check || self.launched.is_none() || self.updater.is_some() {
            return;
        }
        let arguments: Vec<String> = proxy.map(|proxy| vec![String::from("--proxy"), proxy.to_owned()]).unwrap_or_default();
        let mut child = match broker.spawn(Program::Updater, &arguments, false) {
            Ok(child) => child,
            Err(error) => {
                eprintln!("neoteletext: the updater cannot start: {error}");
                self.failure = Some(error.to_string());
                return;
            }
        };
        let (Some(output), Some(input)) = (child.stdout.take(), child.stdin.take()) else { return };
        let events = events.clone();
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            let end = loop {
                match read_from_updater(&mut output) {
                    Ok(message) => {
                        if events.send(Event::Updater(message)).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break None,
                    Err(error) => break Some(error.to_string()),
                }
            };
            let _ = events.send(Event::UpdaterEnded(end));
        });
        self.updater = Some((child, Box::new(input)));
        self.check();
    }

    /// Stops the updater: the user turned checking off.
    pub fn stop(&mut self) {
        if let Some((mut child, input)) = self.updater.take() {
            drop(input);
            let _ = child.kill();
            let _ = child.wait();
        }
        self.next_check = None;
    }

    fn tell(&mut self, message: &ToUpdater) {
        let Some((_, input)) = self.updater.as_mut() else { return };
        if let Err(error) = write_to_updater(input, message) {
            eprintln!("neoteletext: cannot reach the updater: {error}");
            self.stop();
        }
    }

    fn check(&mut self) {
        self.tell(&ToUpdater::Check);
        self.next_check = Instant::now().checked_add(CHECK_INTERVAL);
    }

    /// The updater said something, untrusted. Whether the view changed.
    pub fn said(&mut self, message: FromUpdater) -> bool {
        match message {
            FromUpdater::Latest { version, checked } => {
                self.latest = version;
                self.last_success = Some(checked);
                self.failure = None;
                if let Some(newer) = self.offer()
                    && self.offered != Some(newer)
                {
                    self.offered = Some(newer);
                    self.strip = true;
                }
            }
            FromUpdater::Staged(version) => {
                if self.downloading == Some(version) {
                    self.downloading = None;
                    self.staged = Some(version);
                    self.strip = false;
                }
            }
            FromUpdater::Failed { reason, last_success } => {
                self.downloading = None;
                self.last_success = last_success.or(self.last_success);
                self.failure = Some(reason);
            }
        }
        true
    }

    /// The updater ended.
    pub fn ended(&mut self, reason: Option<String>) {
        if let Some(reason) = &reason {
            eprintln!("neoteletext: the updater ended: {reason}");
        }
        self.updater = None;
        self.failure = Some(reason.unwrap_or_else(|| String::from("the updater ended")));
    }

    /// The version to offer: the newest the repository names, newer than
    /// the one running, not known bad, not already staged or downloading.
    #[must_use]
    pub fn offer(&self) -> Option<Version> {
        let state = self.launched.as_ref()?;
        let latest = self.latest?;
        (latest > state.running() && !state.is_bad(latest) && self.staged != Some(latest) && self.downloading != Some(latest)).then_some(latest)
    }

    /// The user accepted the offer: the updater downloads and stages it
    /// (section 3.4). Nothing is downloaded before.
    pub fn install(&mut self) {
        if let Some(version) = self.offer() {
            self.downloading = Some(version);
            self.strip = false;
            self.tell(&ToUpdater::Download(version));
        }
    }

    /// The user put the offer off: the strip stops showing it.
    pub fn later(&mut self) {
        self.strip = false;
    }

    /// A helper failed before the version was found healthy.
    pub fn helper_failed(&mut self, what: &str) {
        if !self.health.told && self.health.failed.is_none() {
            self.health.failed = Some(what.to_owned());
        }
    }

    /// The interface drew its first frame.
    pub fn drawn(&mut self) {
        if self.health.drawn.is_none() {
            self.health.drawn = Some(Instant::now());
        }
    }

    /// When the timed work is next due.
    #[must_use]
    pub fn due(&self) -> Option<Instant> {
        [self.next_check.filter(|_| self.updater.is_some()), self.health.due()].into_iter().flatten().min()
    }

    /// The timed work now due: the daily check, and telling the launcher
    /// the version is healthy.
    pub fn tick(&mut self, broker: Option<&Broker>, now: Instant) {
        if self.next_check.is_some_and(|at| now >= at) && self.updater.is_some() {
            self.check();
        }
        if !self.health.told && self.health.healthy(now) {
            self.health.told = true;
            if self.launched.is_some()
                && let Some(broker) = broker
            {
                match broker.healthy() {
                    Ok(()) => eprintln!("neoteletext: the version is healthy"),
                    Err(why) => eprintln!("neoteletext: the launcher was not told the version is healthy: {why}"),
                }
            }
        }
    }

    /// The panel's line about updates (section 3.4), in the core's words.
    #[must_use]
    pub fn status(&self, check: bool, now_utc: i64) -> String {
        let mut text = String::new();
        let Some(state) = &self.launched else {
            return match &self.state_refused {
                Some(why) => format!("Updates unavailable: {why}"),
                None => String::from("This copy was not installed by the installer, so it does not update itself"),
            };
        };
        if !self.keyed {
            return String::from("This build carries no update keys, so it does not update itself");
        }
        if let Some(rolled_back) = state.rolled_back {
            let _ = write!(text, "Version {rolled_back} did not start and was rolled back; settings changed under it stay with it. ");
        }
        if let Some(staged) = self.staged {
            let _ = write!(text, "NeoTeletext {staged} starts at the next start");
        } else if let Some(version) = self.downloading {
            let _ = write!(text, "Downloading NeoTeletext {version}");
        } else if !check {
            text.push_str("Not checking for updates");
        } else if self.last_success.is_none_or(|at| now_utc.saturating_sub(at) > UNAVAILABLE_AFTER) && self.failure.is_some() {
            text.push_str("Updates unavailable");
        } else if let Some(version) = self.offer() {
            let _ = write!(text, "NeoTeletext {version} is available");
        } else if let Some(failure) = &self.failure {
            let _ = write!(text, "Could not check for updates: {failure}");
        } else if self.last_success.is_some() {
            text.push_str("NeoTeletext is up to date");
        } else {
            text.push_str("Checking for updates");
        }
        text
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// A browser the launcher started, of a build with update keys.
    fn launched(version: Version) -> Updates {
        Updates { keyed: true, ..Updates::new(Some(State::installed(version)), None, true) }
    }

    /// Section 3.4: a newer version is offered once in the strip, nothing
    /// is downloaded until the user accepts, and a bad one is never offered.
    #[test]
    fn a_newer_version_is_offered_once_and_downloaded_only_when_accepted() {
        let mut updates = launched(Version::new(0, 1, 0));
        updates.said(FromUpdater::Latest { version: Some(Version::new(0, 2, 0)), checked: 100 });
        assert!(updates.strip, "offered in the strip");
        assert_eq!(updates.offer(), Some(Version::new(0, 2, 0)));
        assert_eq!(updates.downloading, None, "nothing downloaded before the user accepts");
        updates.later();
        assert!(!updates.strip);
        updates.said(FromUpdater::Latest { version: Some(Version::new(0, 2, 0)), checked: 200 });
        assert!(!updates.strip, "offered once");
        assert_eq!(updates.status(true, 200), "NeoTeletext 0.2.0 is available", "the panel keeps offering it");
        updates.install();
        assert_eq!(updates.downloading, Some(Version::new(0, 2, 0)));
        updates.said(FromUpdater::Staged(Version::new(0, 2, 0)));
        assert_eq!(updates.status(true, 200), "NeoTeletext 0.2.0 starts at the next start");
        assert_eq!(updates.offer(), None);

        let mut state = State::installed(Version::new(0, 1, 0));
        state.mark_bad(Version::new(0, 2, 0));
        let mut updates = Updates { keyed: true, ..Updates::new(Some(state), None, true) };
        updates.said(FromUpdater::Latest { version: Some(Version::new(0, 2, 0)), checked: 100 });
        assert_eq!(updates.offer(), None, "a version rolled back is never offered");
        assert!(!updates.strip);
        updates.said(FromUpdater::Latest { version: Some(Version::new(0, 1, 0)), checked: 100 });
        assert_eq!(updates.offer(), None, "nor an older or the same version");
    }

    /// Section 3.4: "Updates unavailable" after 7 days without a successful
    /// check, never sooner; and the panel's other states.
    #[test]
    fn updates_are_unavailable_only_after_seven_days() {
        let unkeyed = Updates::new(Some(State::installed(Version::new(0, 1, 0))), None, true);
        if !unkeyed.keyed {
            assert!(unkeyed.status(true, 0).contains("no update keys"));
        }
        let mut updates = launched(Version::new(0, 1, 0));
        updates.said(FromUpdater::Latest { version: None, checked: 1_000 });
        assert_eq!(updates.status(true, 1_000), "NeoTeletext is up to date");
        updates.said(FromUpdater::Failed { reason: String::from("the timestamp has expired"), last_success: Some(1_000) });
        assert!(updates.status(true, 1_000 + UNAVAILABLE_AFTER).starts_with("Could not check"), "not yet");
        assert_eq!(updates.status(true, 1_001 + UNAVAILABLE_AFTER), "Updates unavailable");
        assert_eq!(updates.status(false, 1_001 + UNAVAILABLE_AFTER), "Not checking for updates");
    }

    /// Section 3.2: healthy once the window has drawn within 30 seconds,
    /// the settings were read and no helper failed, after a moment to fail.
    #[test]
    fn a_version_is_healthy_only_when_it_drew_read_its_settings_and_lost_no_helper() {
        let mut updates = launched(Version::new(0, 1, 0));
        let now = Instant::now();
        assert!(!updates.health.healthy(now), "nothing drawn");
        updates.drawn();
        assert!(!updates.health.healthy(now), "not settled");
        assert!(updates.health.healthy(now + HEALTH_SETTLE + Duration::from_millis(1)));
        updates.helper_failed("the decoder for channel 1 stopped: cannot apply Landlock");
        assert!(!updates.health.healthy(now + HEALTH_SETTLE * 2), "a helper that did not confine");
        let mut unread = Updates::new(Some(State::installed(Version::new(0, 1, 0))), None, false);
        unread.drawn();
        assert!(!unread.health.healthy(now + HEALTH_SETTLE * 2), "settings unreadable");
        let mut late = launched(Version::new(0, 1, 0));
        late.health.started = now.checked_sub(HEALTH_WINDOW * 2).unwrap();
        late.drawn();
        assert!(!late.health.healthy(Instant::now() + HEALTH_SETTLE * 2), "drawn too late");
    }

    #[test]
    fn a_browser_not_installed_does_not_update() {
        let updates = Updates::default();
        assert_eq!(updates.running(), None);
        assert!(updates.status(true, 0).contains("not installed"));
    }
}
