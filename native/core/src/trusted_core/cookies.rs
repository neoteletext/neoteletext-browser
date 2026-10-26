//! Cookies in the running browser (`cookies.md`, decision 62): fetchers'
//! questions answered from the trusted core's store, each answering only
//! with its own channel's cookies (requirement 21.1); the cookies panel's
//! views and requests (requirement 21.6); the settings (requirement 21.7);
//! and the persistent cookies kept, sealed under a key from the platform's
//! secret store, which the broker reads (requirement 21.4).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use neoteletext::calendar::utc_now;
use neoteletext::cookies::{self as kept, Channel, Mode, Refused, Store};
use neoteletext_web::cookie::Site;
use panel::Row;
use protocol::console::Level;
use protocol::fetcher::{CookieSite, FromFetcher, ToFetcher, write_to_fetcher};

use super::Core;
use super::numbers::Numbers;
use super::run::NO_KEYCHAIN;

/// How long after a change the cookies are written: a burst of changes is
/// one write (`cookies.md` section 6).
const WRITE_AFTER: Duration = Duration::from_secs(1);

/// The cookies' state in the trusted core.
#[derive(Default)]
pub(crate) struct Cookies {
    /// Every cookie.
    pub(crate) store: Store,
    /// The cookie file's key, once the broker gave it.
    key: Option<[u8; kept::KEY_BYTES]>,
    /// Why cookies are kept for this run only, when they are.
    pub(crate) for_this_run: Option<String>,
    /// Started with `--no-keychain` (decision 113): the broker is never
    /// asked for the key, so the platform's secret store is not touched.
    pub(crate) no_keychain: bool,
    /// When the changed cookies are next written.
    pub(crate) due: Option<Instant>,
    numbers: Numbers<Row>,
    panel_open: bool,
    query: String,
    first: u32,
    notice: String,
    /// Channels the strip has said are blocked, once each.
    said_blocked: BTreeSet<String>,
}

fn now_unix() -> u64 {
    u64::try_from(utc_now()).unwrap_or(0)
}

const fn site(site: CookieSite) -> Site {
    match site {
        CookieSite::SameService => Site::SameService,
        CookieSite::CrossServiceNavigation => Site::CrossServiceNavigation,
        CookieSite::CrossOriginForm => Site::CrossOriginForm,
    }
}

const fn mode(mode: protocol::settings::CookieMode) -> Mode {
    match mode {
        protocol::settings::CookieMode::Keep => Mode::Keep,
        protocol::settings::CookieMode::Close => Mode::Close,
        protocol::settings::CookieMode::Block => Mode::Block,
    }
}

/// The panel's view of a mode, for the settings.
pub(crate) const fn mode_view(mode: Mode) -> protocol::settings::CookieMode {
    match mode {
        Mode::Keep => protocol::settings::CookieMode::Keep,
        Mode::Close => protocol::settings::CookieMode::Close,
        Mode::Block => protocol::settings::CookieMode::Block,
    }
}

impl Core {
    /// The channel a fetcher serves, and whether its tab is private.
    fn fetcher_channel(&self, channel: u32) -> Option<(Channel, bool)> {
        let tab = self.session.all_tabs().find(|tab| tab.channel.id == channel)?;
        Some((Channel::of(tab.address()?.channel_text())?, tab.private))
    }

    /// Answers a fetcher at once: it waits for the answer.
    fn answer(&mut self, channel: u32, message: &ToFetcher) {
        if let Some(input) = self.fetchers.get_mut(&channel).and_then(super::processes::Fetcher::input) {
            let _ = write_to_fetcher(input, message);
            let _ = std::io::Write::flush(input);
        }
    }

    /// A fetcher's cookie message (`cookies.md` section 9), answered at once
    /// when it is a question.
    pub(crate) fn cookie_message(&mut self, channel: u32, message: FromFetcher) -> Result<(), String> {
        if let Some(answer) = self.cookie_reply(channel, message)? {
            self.answer(channel, &answer);
        }
        Ok(())
    }

    /// What a fetcher's cookie message does, and the answer it is owed.
    pub(crate) fn cookie_reply(&mut self, channel: u32, message: FromFetcher) -> Result<Option<ToFetcher>, String> {
        let now = now_unix();
        let owner = self.fetcher_channel(channel);
        match message {
            FromFetcher::CookieQuestion { id, host, path, secure, site: kind } => {
                let header =
                    owner.map_or_else(Vec::new, |(channel, private)| self.cookies.store.header_for(private, &channel, (&host, &path, secure), site(kind), now));
                return Ok(Some(ToFetcher::CookieAnswer { id, header }));
            }
            FromFetcher::ReachedQuestion { id, host } => {
                // Only of the asking channel's own host, from its tab's jar:
                // no fetcher learns where else the user has been (finding
                // F-27, `cookies-security.md`).
                let reached = owner.is_some_and(|(owner, private)| self.cookies.store.reached_securely(private, &owner, &host, now));
                return Ok(Some(ToFetcher::ReachedAnswer { id, reached }));
            }
            FromFetcher::CookieSet { host, path, secure, line, .. } => {
                let Some((owner, private)) = owner else { return Ok(None) };
                match self.cookies.store.store(private, &owner, (&host, &path, secure), &line, now) {
                    Ok(()) => self.cookies_changed(),
                    Err(Refused::Blocked) => {
                        if self.cookies.said_blocked.insert(owner.key.clone()) && self.active_channel() == Some(channel) {
                            self.say(String::from("This channel's cookies are blocked"))?;
                        }
                    }
                    // Said without the cookie: the console never shows one
                    // (requirement 19.8).
                    Err(refused) => {
                        let at = self.now();
                        self.console.log(Some(channel), Level::Warning, at, &format!("a cookie was refused: {refused:?}"));
                    }
                }
            }
            FromFetcher::ClearSiteData(host) => {
                if let Some((owner, private)) = owner {
                    self.cookies.store.clear_site(private, &owner, &host);
                    self.cookies_changed();
                    let at = self.now();
                    self.console.log(Some(channel), Level::Info, at, "the channel cleared its cookies (Clear-Site-Data)");
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// Persistent cookies changed: they are written soon, once, if kept;
    /// the panel shows them now.
    fn cookies_changed(&mut self) {
        if self.cookies.store.changed() && self.cookies.store.mode == Mode::Keep && self.cookies.key.is_some() && self.cookies.due.is_none() {
            self.cookies.due = Instant::now().checked_add(WRITE_AFTER);
        }
    }

    /// Writes the persistent cookies, sealed, when they changed.
    pub(crate) fn keep_cookies(&mut self) {
        self.cookies.due = None;
        let (Some(folder), Some(key)) = (self.settings_folder.clone(), self.cookies.key) else { return };
        if self.cookies.store.mode != Mode::Keep || !self.cookies.store.changed() {
            return;
        }
        match kept::save(&folder, &key, &self.cookies.store, now_unix()) {
            Ok(()) => self.cookies.store.written(),
            Err(error) => self.log(Level::Error, None, &format!("the cookies could not be kept in {}: {error}", folder.display())),
        }
    }

    /// At start: the settings' mode and blocked channels, and the kept
    /// cookies when they are kept and the key can be had.
    pub(crate) fn cookies_at_start(&mut self) {
        self.cookies.store.mode = self.settings.cookies;
        self.cookies.store.blocked = self.settings.cookie_block.iter().cloned().collect();
        let Some(folder) = self.settings_folder.clone() else { return };
        match self.settings.cookies {
            Mode::Close => {
                if let Err(error) = kept::remove(&folder) {
                    self.log(Level::Error, None, &format!("the kept cookies could not be removed: {error}"));
                }
            }
            Mode::Block => {}
            Mode::Keep => self.open_cookie_file(&folder),
        }
    }

    fn open_cookie_file(&mut self, folder: &std::path::Path) {
        // Without a key nothing is read or written (requirement 21.4): the
        // kept cookies stay as they are for the next ordinary start.
        if self.cookies.no_keychain {
            self.log(Level::Info, None, &format!("cookies are kept for this run only: started with {NO_KEYCHAIN}"));
            self.cookies.for_this_run = Some(format!("Cookies are kept for this run only: started with {NO_KEYCHAIN}"));
            return;
        }
        let key = match self.broker.as_ref().map(|broker| broker.cookie_key()) {
            Some(Ok(key)) => key,
            Some(Err(reason)) => {
                self.log(Level::Warning, None, &format!("cookies are kept for this run only: {reason}"));
                self.cookies.for_this_run = Some(format!("Cookies are kept for this run only: {reason}"));
                return;
            }
            None => return,
        };
        self.cookies.key = Some(key);
        match kept::load(folder) {
            Ok(None) => {}
            Ok(Some(sealed)) => match kept::open(&key, &sealed) {
                Some(text) => {
                    for dropped in self.cookies.store.import(&text, now_unix()) {
                        self.log(Level::Warning, None, &dropped);
                    }
                }
                None => self.log(Level::Error, None, "the kept cookies could not be opened with the key the secret store holds; they are replaced"),
            },
            Err(error) => self.log(Level::Error, None, &format!("the kept cookies could not be read: {error}")),
        }
    }

    /// At exit: kept cookies written, or none left behind.
    pub(crate) fn cookies_at_exit(&mut self) {
        match self.cookies.store.mode {
            Mode::Keep => self.keep_cookies(),
            Mode::Close => {
                if let Some(folder) = &self.settings_folder
                    && let Err(error) = kept::remove(folder)
                {
                    self.log(Level::Error, None, &format!("the cookies could not be removed: {error}"));
                }
            }
            Mode::Block => {}
        }
    }

    /// The settings' mode changed (requirement 21.7).
    pub(crate) fn cookie_mode_changed(&mut self, new: protocol::settings::CookieMode) {
        self.cookies.store.mode = mode(new);
        let Some(folder) = self.settings_folder.clone() else { return };
        match self.cookies.store.mode {
            Mode::Keep => {
                if self.cookies.key.is_none() && self.cookies.for_this_run.is_none() {
                    self.open_cookie_file(&folder);
                }
                self.cookies_changed();
            }
            Mode::Close => {
                if let Err(error) = kept::remove(&folder) {
                    self.log(Level::Error, None, &format!("the kept cookies could not be removed: {error}"));
                }
            }
            Mode::Block => {}
        }
    }

    /// The last private tab closed, in every window: the private cookies
    /// go (requirement 21.3).
    pub(crate) fn private_cookies_closed(&mut self) {
        if !self.session.all_tabs().any(|tab| tab.private) {
            self.cookies.store.close_private();
        }
    }

    /// Clears the ordinary cookies made since `since`, seconds since 1970,
    /// with history (requirement 21.8).
    pub(crate) fn clear_cookies_since(&mut self, since: Option<i64>) -> usize {
        let from = since.map_or(0, |since| u64::try_from(since).unwrap_or(0));
        let removed = self.cookies.store.delete_created(from, u64::MAX);
        self.cookies_changed();
        removed
    }
}

mod panel;

#[cfg(test)]
mod tests;
