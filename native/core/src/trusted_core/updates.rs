//! Updates in the trusted core (`installation-and-updates.md` sections 3
//! and 3.4): it tells the updater when to look, offers a newer version in
//! the strip once and in the settings panel always, has it staged only
//! once the user accepts, and finds a version healthy (section 3.2). It
//! parses none of the updater's formats: only its messages, each bounded
//! (`update::wire`), and the launcher's state, read strictly.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use confine::broker::{Broker, Helper, Program};
use update::state::State;
use update::version::Version;
use update::wire::{FromUpdater, ToUpdater, read_from_updater, write_to_updater};

use neoteletext::settings as stored;
use protocol::{ToCore, ToInterface, clean};

use super::processes::{end, relay};
use super::{Core, Event};

/// How often the updater looks while the browser runs (section 3.4).
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The time a version has to show it is healthy (section 3.2), and how
/// long after its first frame its helpers are given to fail: defined once
/// in `update::layout` for the core and the launcher, whose health
/// deadline is made of them (decision 93).
pub use update::layout::{HEALTH_SETTLE, HEALTH_WINDOW};
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
    /// When the core received the updater's last report of a successful
    /// check, on its own clock, in seconds since 1970 (UTC).
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
        listen(output, events.clone());
        self.updater = Some((child, Box::new(input)));
        self.check();
    }

    /// Stops the updater: the user turned checking off.
    pub fn stop(&mut self) {
        if let Some((mut child, input)) = self.updater.take() {
            drop(input);
            end(&mut child);
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

    /// The updater said something, untrusted, received at `received` on
    /// the core's clock (seconds since 1970, UTC). Whether the view changed.
    /// The age of a check is the core's own: the times the updater reports
    /// are on its clock, and are not used (finding F-93).
    pub fn said(&mut self, message: FromUpdater, received: i64) -> bool {
        match message {
            FromUpdater::Latest { version, checked: _ } => {
                self.latest = version;
                self.last_success = Some(received);
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
            FromUpdater::Failed { reason, last_success: _ } => {
                self.downloading = None;
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

impl Core {
    /// Updates as the settings panel shows them (section 3.4 of the
    /// installation design), and the strip's offer, said once.
    pub(super) fn send_update(&mut self) -> Result<(), String> {
        if self.updates.strip
            && let Some(version) = self.updates.offer()
        {
            self.updates.later();
            self.say(format!("NeoTeletext {version} is available: Install it in Settings"))?;
        }
        let status = self.updates.status(self.settings.check_updates, neoteletext::calendar::utc_now());
        let view = interface_core::settings::UpdateView {
            version: self.updates.running().map(|version| version.to_string()).unwrap_or_default(),
            status: clean(&status, protocol::settings::MAX_UPDATE_STATUS),
            offer: self.updates.offer().map(|version| version.to_string()),
            checking: self.settings.check_updates,
        };
        self.send(&ToInterface::Update(view))
    }

    /// The window's first frame, a part of the version's health, or the
    /// user's acceptance of the version offered (sections 3.2 and 3.4).
    pub(super) fn update_intent(&mut self, intent: &ToCore) -> Result<(), String> {
        if matches!(intent, ToCore::InstallUpdate) {
            self.updates.install();
            return self.send_update();
        }
        self.updates.drawn();
        Ok(())
    }

    /// Starts the updater when the settings let it check (section 3.4).
    pub(super) fn start_updater(&mut self) {
        let Ok(broker) = self.broker().cloned() else { return };
        let proxy = self.settings.proxy.as_ref().map(stored::Proxy::url);
        self.updates.start(&broker, self.settings.check_updates, proxy.as_deref(), &self.events);
    }
}

/// Reads the updater's messages on a thread of its own, through the
/// relay every helper's messages take, so each is held to the updater's
/// own budget until the core has handled it (finding F-10).
fn listen(output: impl Read + Send + 'static, events: Sender<Event>) {
    relay(output, read_updater, events, Event::Updater, Event::UpdaterEnded);
}

/// One of the updater's messages, as the relay reads a helper's: none
/// once its pipe has ended, at a message's start or within one, as the
/// updater's end was read before.
fn read_updater(mut input: &mut dyn Read) -> std::io::Result<Option<FromUpdater>> {
    match read_from_updater(&mut input) {
        Ok(message) => Ok(Some(message)),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
