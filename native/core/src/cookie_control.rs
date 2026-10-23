//! Cookies in the running browser (`cookies.md`, decision 62): fetchers'
//! questions answered from the trusted core's store, each answering only
//! with its own channel's cookies (requirement 21.1); the cookies panel's
//! views and requests (requirement 21.6); the settings (requirement 21.7);
//! and the persistent cookies kept, sealed under a key from the platform's
//! secret store, which the broker reads (requirement 21.4).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use neoteletext::cookies::{self as kept, Channel, Mode, Refused, Store};
use neoteletext_web::cookie::Site;
use protocol::cookies::{CookieRow, CookiesRequest, CookiesView, MAX_LABEL, MAX_NOTICE, MAX_USED, MAX_VIEW_ROWS};
use protocol::fetcher::{CookieSite, FromFetcher, ToFetcher, write_to_fetcher};
use protocol::{ToInterface, clean};

use crate::Core;
use crate::library::Numbers;
use protocol::console::Level;

/// How long after a change the cookies are written: a burst of changes is
/// one write (`cookies.md` section 6).
const WRITE_AFTER: Duration = Duration::from_secs(1);

/// A row of the panel's view, by what it names.
#[derive(Clone, Debug)]
enum Row {
    /// An origin, with all its channels.
    Site(String),
    /// A channel.
    Channel(String),
}

/// The cookies' state in the trusted core.
#[derive(Default)]
pub(crate) struct Cookies {
    /// Every cookie.
    pub(crate) store: Store,
    /// The cookie file's key, once the broker gave it.
    key: Option<[u8; kept::KEY_BYTES]>,
    /// Why cookies are kept for this run only, when they are.
    pub(crate) for_this_run: Option<String>,
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
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
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
        if let Some(input) = self.fetchers.get_mut(&channel).and_then(|fetcher| fetcher.input.as_mut()) {
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
                let reached = self.cookies.store.reached_securely(&host, now);
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

    /// A request of the panel, strip input (requirement 21.6).
    pub(crate) fn cookies_request(&mut self, request: CookiesRequest) -> Result<(), String> {
        let row = |number: u32, cookies: &Cookies| cookies.numbers.get(number).cloned();
        match request {
            CookiesRequest::Panel(open) => {
                self.cookies.panel_open = open;
                self.cookies.notice.clear();
                if !open {
                    return Ok(());
                }
            }
            CookiesRequest::Query { text, first } => {
                self.cookies.query = clean(&text, protocol::cookies::MAX_QUERY);
                self.cookies.first = first;
            }
            CookiesRequest::Delete(number) => {
                let removed = match row(number, &self.cookies) {
                    Some(Row::Site(origin)) => self.cookies.store.delete_origin(&origin),
                    Some(Row::Channel(key)) => self.cookies.store.delete_channel(&key),
                    None => return self.send_cookies(),
                };
                self.cookies.notice = format!("{removed} cookies deleted");
                self.cookies_changed();
            }
            CookiesRequest::Block { number, blocked } => {
                let Some(Row::Channel(key)) = row(number, &self.cookies) else { return self.send_cookies() };
                self.cookies.store.block(&key, blocked);
                self.settings.cookie_block = self.cookies.store.blocked.iter().take(neoteletext::settings::MAX_BLOCKED).cloned().collect();
                self.save_settings();
                self.cookies.said_blocked.remove(&key);
                self.cookies.notice = if blocked { String::from("Blocked, and its cookies deleted") } else { String::from("Unblocked") };
                self.cookies_changed();
            }
            CookiesRequest::DeleteAll => {
                self.cookies.store.delete_all();
                self.cookies.notice = String::from("Every cookie deleted");
                self.cookies_changed();
            }
        }
        self.send_cookies()
    }

    /// The panel's view, while it is open: each site with cookies, and its
    /// channels, matching the search; blocked channels too, to unblock.
    pub(crate) fn send_cookies(&mut self) -> Result<(), String> {
        if !self.cookies.panel_open {
            return Ok(());
        }
        let offset = self.local_time().offset;
        let listed = self.cookies.store.listed();
        let (count, bytes) = listed.iter().fold((0_usize, 0_usize), |(count, bytes), row| (count.saturating_add(row.count), bytes.saturating_add(row.bytes)));
        let query = self.cookies.query.to_lowercase();
        let mut channels: Vec<(String, String, usize, usize, u64, bool)> = listed
            .into_iter()
            .map(|row| {
                let blocked = self.cookies.store.blocked.contains(&row.key);
                (row.origin, row.key, row.count, row.bytes, row.used_at, blocked)
            })
            .collect();
        for key in &self.cookies.store.blocked {
            if !channels.iter().any(|(_, listed, ..)| listed == key)
                && let Some(channel) = Channel::of(key)
            {
                channels.push((channel.origin().to_owned(), key.clone(), 0, 0, 0, true));
            }
        }
        channels.retain(|(_, key, ..)| query.is_empty() || key.to_lowercase().contains(&query));
        channels.sort();
        let used = |at: u64| {
            if at == 0 {
                String::new()
            } else {
                let day = neoteletext::calendar::local_day(i64::try_from(at).unwrap_or(i64::MAX), offset);
                clean(&neoteletext::calendar::day_words(day), MAX_USED)
            }
        };
        // Each origin, then its channels.
        let mut rows: Vec<(Row, CookieRow)> = Vec::new();
        let mut index = 0;
        while let Some((origin, ..)) = channels.get(index) {
            let origin = origin.clone();
            let members: Vec<&(String, String, usize, usize, u64, bool)> = channels.iter().skip(index).take_while(|(other, ..)| *other == origin).collect();
            let site_row = CookieRow {
                number: 0,
                site: true,
                label: clean(&origin, MAX_LABEL),
                count: u32::try_from(members.iter().map(|member| member.2).sum::<usize>()).unwrap_or(u32::MAX),
                bytes: u64::try_from(members.iter().map(|member| member.3).sum::<usize>()).unwrap_or(u64::MAX),
                used: used(members.iter().map(|member| member.4).max().unwrap_or(0)),
                blocked: false,
            };
            rows.push((Row::Site(origin.clone()), site_row));
            for (_, key, count, size, at, blocked) in &members {
                let folder = key.strip_prefix(origin.as_str()).unwrap_or(key);
                let row = CookieRow {
                    number: 0,
                    site: false,
                    label: clean(folder, MAX_LABEL),
                    count: u32::try_from(*count).unwrap_or(u32::MAX),
                    bytes: u64::try_from(*size).unwrap_or(u64::MAX),
                    used: used(*at),
                    blocked: *blocked,
                };
                rows.push((Row::Channel(key.clone()), row));
            }
            index = index.saturating_add(members.len());
        }
        let total = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        let first = self.cookies.first.min(total);
        self.cookies.numbers.fresh();
        let shown: Vec<CookieRow> = rows
            .into_iter()
            .skip(usize::try_from(first).unwrap_or(usize::MAX))
            .take(MAX_VIEW_ROWS)
            .map(|(row, mut shown)| {
                shown.number = self.cookies.numbers.number(row);
                shown
            })
            .collect();
        let notice = self.cookies.for_this_run.clone().filter(|_| self.cookies.notice.is_empty()).unwrap_or_else(|| self.cookies.notice.clone());
        let view = CookiesView {
            query: self.cookies.query.clone(),
            first,
            total,
            rows: shown,
            count: u32::try_from(count).unwrap_or(u32::MAX),
            bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
            notice: clean(&notice, MAX_NOTICE),
        };
        self.send(&ToInterface::Cookies(view))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;
    use crate::session::Session;
    use neoteletext::bookmarks::Bookmarks;
    use neoteletext::outbox::Outbox;
    use neoteletext::settings::Settings;
    use protocol::read_from_core;
    use std::io::Write;
    use std::sync::mpsc;

    /// What the core writes to the interface.
    struct Sink(mpsc::Sender<Vec<u8>>);

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.send(bytes.to_vec());
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Test {
        core: Core,
        written: mpsc::Receiver<Vec<u8>>,
        folder: std::path::PathBuf,
    }

    impl Test {
        fn new(name: &str) -> Self {
            let folder = std::env::temp_dir().join(format!("neoteletext-cookie-control-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).unwrap();
            let (sender, written) = mpsc::channel();
            let (events, _incoming) = mpsc::channel();
            let core =
                Core::new(Session::new(Bookmarks::default()), Vec::new(), (Settings::default(), Some(folder.clone())), events, Outbox::new(Sink(sender)), None);
            Self { core, written, folder }
        }

        fn last_view(&mut self) -> CookiesView {
            self.core.to_interface.flush().unwrap();
            let mut bytes = Vec::new();
            // The outbox writes on a thread of its own.
            while let Ok(chunk) = self.written.recv_timeout(Duration::from_millis(200)) {
                bytes.extend(chunk);
            }
            let mut input = bytes.as_slice();
            let mut last = None;
            while let Some(message) = read_from_core(&mut input).unwrap() {
                if let ToInterface::Cookies(view) = message {
                    last = Some(view);
                }
            }
            last.unwrap()
        }

        /// A tab on `address`, private or not; its channel's number.
        fn tab(&mut self, address: &str, private: bool) -> u32 {
            if private {
                self.core.session.open_private(address).unwrap();
            } else {
                self.core.session.open(address).unwrap();
            }
            self.core.session.tabs.last().unwrap().channel.id
        }

        fn set(&mut self, channel: u32, host: &str, line: &str) {
            let message = FromFetcher::CookieSet { host: host.into(), path: "/teletext/".into(), scope: None, secure: true, line: line.as_bytes().to_vec() };
            assert_eq!(self.core.cookie_reply(channel, message).unwrap(), None);
        }

        fn ask(&mut self, channel: u32, host: &str) -> String {
            let question =
                FromFetcher::CookieQuestion { id: 9, host: host.into(), path: "/teletext/100.tti".into(), secure: true, site: CookieSite::SameService };
            let Some(ToFetcher::CookieAnswer { id: 9, header }) = self.core.cookie_reply(channel, question).unwrap() else { panic!("no answer") };
            String::from_utf8(header).unwrap()
        }
    }

    impl Drop for Test {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }

    /// Requirements 21.1, 21.2 and 21.3: a fetcher is answered with its own
    /// channel's cookies alone; tabs on a channel share them; private tabs
    /// have their own, gone with the last private tab.
    #[test]
    fn each_fetcher_is_answered_with_its_channels_cookies() {
        let mut test = Test::new("answers");
        let news = test.tab("https://news.example/teletext/", false);
        let again = test.tab("https://news.example/teletext/#302", false);
        let other = test.tab("https://other.example/teletext/", false);
        test.set(news, "news.example", "id=1");
        assert_eq!(test.ask(news, "news.example"), "id=1");
        assert_eq!(test.ask(again, "news.example"), "id=1", "another tab on the channel");
        assert_eq!(test.ask(other, "news.example"), "", "another channel's fetcher asking for this host");
        assert_eq!(test.ask(other, "other.example"), "");
        test.set(other, "news.example", "planted=1");
        assert_eq!(test.ask(news, "news.example"), "id=1", "a fetcher cannot set another channel's cookie");
        assert_eq!(test.ask(999, "news.example"), "", "no such channel");

        let private = test.tab("https://news.example/teletext/", true);
        assert_eq!(test.ask(private, "news.example"), "", "the private session is apart");
        test.set(private, "news.example", "p=1");
        assert_eq!(test.ask(private, "news.example"), "p=1");
        assert_eq!(test.ask(news, "news.example"), "id=1");
        let index = test.core.session.tabs.iter().position(|tab| tab.private).unwrap();
        test.core.session.close(index);
        test.core.private_cookies_closed();
        let private = test.tab("https://news.example/teletext/", true);
        assert_eq!(test.ask(private, "news.example"), "", "gone with the last private tab");

        let reached = FromFetcher::ReachedQuestion { id: 4, host: "news.example".into() };
        assert_eq!(test.core.cookie_reply(news, reached).unwrap(), Some(ToFetcher::ReachedAnswer { id: 4, reached: true }));
    }

    /// Requirements 21.6 and 21.7: the panel lists sites and channels with no
    /// name or value, deletes by number, and blocking is kept in the
    /// settings and refuses the channel's cookies.
    #[test]
    fn the_panel_lists_deletes_and_blocks() {
        let mut test = Test::new("panel");
        let news = test.tab("https://news.example/teletext/", false);
        test.set(news, "news.example", "secret-name=secret-value");
        test.core.cookies_request(CookiesRequest::Panel(true)).unwrap();
        let view = test.last_view();
        let labels: Vec<(&str, bool, u32)> = view.rows.iter().map(|row| (row.label.as_str(), row.site, row.count)).collect();
        assert_eq!(labels, [("https://news.example", true, 1), ("/teletext/", false, 1)]);
        assert_eq!(view.count, 1);
        let shown = format!("{view:?}");
        assert!(!shown.contains("secret"), "no name or value reaches the interface: {shown}");

        let channel_row = view.rows[1].number;
        test.core.cookies_request(CookiesRequest::Block { number: channel_row, blocked: true }).unwrap();
        assert_eq!(test.core.settings.cookie_block, ["https://news.example/teletext/"]);
        assert_eq!(test.ask(news, "news.example"), "", "deleted when blocked");
        test.set(news, "news.example", "again=1");
        assert_eq!(test.ask(news, "news.example"), "", "refused while blocked");
        let view = test.last_view();
        assert!(view.rows.iter().any(|row| row.blocked && row.count == 0), "a blocked channel stays listed, to unblock");
        let blocked_row = view.rows.iter().find(|row| row.blocked).unwrap().number;
        test.core.cookies_request(CookiesRequest::Block { number: blocked_row, blocked: false }).unwrap();
        assert_eq!(test.core.settings.cookie_block, Vec::<String>::new());
        test.set(news, "news.example", "back=1");
        test.core.send_cookies().unwrap();
        let view = test.last_view();
        test.core.cookies_request(CookiesRequest::Delete(view.rows[0].number)).unwrap();
        assert_eq!(test.ask(news, "news.example"), "", "the site's cookies deleted");
        assert_eq!(test.last_view().notice, "1 cookies deleted");
        test.core.cookies_request(CookiesRequest::Delete(view.rows[0].number)).unwrap();
        assert_eq!(test.last_view().notice, "1 cookies deleted", "a number of an earlier view names nothing");
    }

    /// Requirement 21.4: without the key, as when the secret store refused
    /// it, no file is written at all, never one in the clear.
    #[test]
    fn without_the_key_nothing_is_written() {
        let mut test = Test::new("no-key");
        let news = test.tab("https://news.example/teletext/", false);
        test.set(news, "news.example", "keep=1; Max-Age=3600");
        assert_eq!(test.core.cookies.due, None);
        test.core.keep_cookies();
        test.core.cookies_at_exit();
        assert_eq!(kept::load(&test.folder).unwrap(), None);
    }

    /// Requirement 21.8: the history panel's clearing of cookies removes those
    /// made in its range.
    #[test]
    fn clearing_with_history_removes_cookies_of_the_range() {
        let mut test = Test::new("clear");
        let news = test.tab("https://news.example/teletext/", false);
        test.set(news, "news.example", "a=1");
        test.core.history_request(protocol::library::HistoryRequest::ClearCookies(protocol::library::ClearRange::Hour)).unwrap();
        assert_eq!(test.ask(news, "news.example"), "", "made a moment ago");
    }

    /// Requirement 21.4: kept cookies are written sealed once they settle,
    /// only persistent ones; "on close" removes the file.
    #[test]
    fn kept_cookies_are_written_sealed_and_removed_on_close() {
        let mut test = Test::new("kept");
        let key = [3_u8; kept::KEY_BYTES];
        test.core.cookies.key = Some(key);
        let news = test.tab("https://news.example/teletext/", false);
        test.set(news, "news.example", "keep=1; Max-Age=3600");
        test.set(news, "news.example", "session=1");
        assert!(test.core.cookies.due.is_some(), "written once the burst settles");
        test.core.keep_cookies();
        let sealed = kept::load(&test.folder).unwrap().unwrap();
        let opened = kept::open(&key, &sealed).unwrap();
        let mut restored = Store::new();
        assert_eq!(restored.import(&opened, now_unix()), Vec::<String>::new());
        assert_eq!(restored.listed()[0].count, 1, "the session cookie is not written");
        test.core.cookie_mode_changed(protocol::settings::CookieMode::Close);
        assert_eq!(kept::load(&test.folder).unwrap(), None, "on close: no file");
        test.set(news, "news.example", "more=1; Max-Age=3600");
        test.core.keep_cookies();
        assert_eq!(kept::load(&test.folder).unwrap(), None, "and none written");
    }
}
