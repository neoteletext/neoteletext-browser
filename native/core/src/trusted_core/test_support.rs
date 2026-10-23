//! Helpers the trusted core's tests share (`00-conventions.md` section 5).

use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

use neoteletext::bookmarks::Bookmarks;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;
use protocol::cookies::CookiesView;
use protocol::fetcher::{CookieSite, FromFetcher, ToFetcher};
use protocol::library::{BookmarksView, HistoryView};
use protocol::{ToInterface, read_from_core};

use super::Core;
use crate::session::Session;

/// A trusted core for a test: no tabs, no helpers, the interface a
/// buffer.
#[cfg(unix)]
pub(super) fn test_core() -> Core {
    let (events, _incoming) = mpsc::channel();
    Core::new(Session::new(Bookmarks::default()), Vec::new(), (Settings::default(), None), events, Outbox::new(Vec::new()), None)
}

/// What the core writes to the interface, a chunk at a time.
pub(super) struct Sink(mpsc::Sender<Vec<u8>>);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.send(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) struct Test {
    pub(super) core: Core,
    pub(super) written: mpsc::Receiver<Vec<u8>>,
    pub(super) folder: std::path::PathBuf,
}

impl Test {
    pub(super) fn new(name: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("neoteletext-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let (sender, written) = mpsc::channel();
        let (events, _incoming) = mpsc::channel();
        let core =
            Core::new(Session::new(Bookmarks::default()), Vec::new(), (Settings::default(), Some(folder.clone())), events, Outbox::new(Sink(sender)), None);
        Self { core, written, folder }
    }

    /// Every message sent since the last call.
    pub(super) fn sent(&mut self) -> Vec<ToInterface> {
        self.core.to_interface.flush().unwrap();
        let mut bytes = Vec::new();
        while let Ok(chunk) = self.written.recv_timeout(Duration::from_millis(200)) {
            bytes.extend(chunk);
        }
        let mut input = bytes.as_slice();
        let mut messages = Vec::new();
        while let Some(message) = read_from_core(&mut input).unwrap() {
            messages.push(message);
        }
        messages
    }

    pub(super) fn last_history(&mut self) -> HistoryView {
        self.sent().into_iter().rev().find_map(|message| if let ToInterface::History(view) = message { Some(view) } else { None }).unwrap()
    }

    pub(super) fn last_bookmarks(&mut self) -> BookmarksView {
        self.sent().into_iter().rev().find_map(|message| if let ToInterface::Bookmarks(view) = message { Some(view) } else { None }).unwrap()
    }

    /// Opens `address` in a new tab and draws its first frame.
    pub(super) fn visit(&mut self, address: &str) {
        self.core.session.open(address).unwrap();
        let channel = self.core.session.tabs.last().unwrap().channel.id;
        self.core.record_pending(channel).unwrap();
    }

    pub(super) fn last_view(&mut self) -> CookiesView {
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
    pub(super) fn tab(&mut self, address: &str, private: bool) -> u32 {
        if private {
            self.core.session.open_private(address).unwrap();
        } else {
            self.core.session.open(address).unwrap();
        }
        self.core.session.tabs.last().unwrap().channel.id
    }

    pub(super) fn set(&mut self, channel: u32, host: &str, line: &str) {
        let message = FromFetcher::CookieSet { host: host.into(), path: "/teletext/".into(), scope: None, secure: true, line: line.as_bytes().to_vec() };
        assert_eq!(self.core.cookie_reply(channel, message).unwrap(), None);
    }

    pub(super) fn ask(&mut self, channel: u32, host: &str) -> String {
        let question = FromFetcher::CookieQuestion { id: 9, host: host.into(), path: "/teletext/100.tti".into(), secure: true, site: CookieSite::SameService };
        let Some(ToFetcher::CookieAnswer { id: 9, header }) = self.core.cookie_reply(channel, question).unwrap() else { panic!("no answer") };
        String::from_utf8(header).unwrap()
    }
}

impl Drop for Test {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}
