//! An address of the longest length, in the files that keep one
//! (neoteletext-common's REQ-1661 item 3). The session file's is in the
//! session's tests.

use crate::address::parse;
use crate::bookmarks::{BAR, Bookmarks};
use crate::history::{History, Visit};
use crate::settings::{MAX_BLOCKED, Settings, checked_address};

/// RFC 9110 section 4.1's 8,000 octets, written out so that these tests say
/// the requirement, not whatever the constant holds.
const LONGEST: usize = 8_000;
const HEAD: &str = "https://a.example/";
/// What makes a channel's address long: its folder is at most the engine's
/// 256 bytes (`MAX_FOLDER_PATH`), and the query holds the rest.
const QUERY: &str = "?q=";
const PAGE: &str = "302/0002";
const NOW: i64 = 1_793_000_000;

/// A channel's address, with the query that makes it the longest once `PAGE`
/// is added.
fn channel() -> String {
    format!("{HEAD}{QUERY}{}", "x".repeat(LONGEST.saturating_sub(HEAD.len()).saturating_sub(QUERY.len()).saturating_sub(PAGE.len())))
}

fn longest() -> String {
    format!("{HEAD}{PAGE}{}", channel().strip_prefix(HEAD).unwrap_or_default())
}

#[test]
fn the_address_rules_take_the_longest_and_no_more() {
    let address = longest();
    assert_eq!(address.len(), LONGEST);
    assert_eq!(parse(&address).unwrap().text, address);
    assert!(parse(&format!("{address}0")).is_err());
    assert_eq!(protocol::MAX_ADDRESS, LONGEST);
}

#[test]
fn the_settings_keep_the_longest_home_and_every_blocked_channel() {
    let home = checked_address(&channel()).unwrap();
    assert_eq!(home, channel());
    // A blocked channel is a folder, its key ending in `/`, at the longest a
    // channel's address may be.
    let folder_text = LONGEST.saturating_sub(HEAD.len()).saturating_sub(PAGE.len()).saturating_sub("0000/".len());
    let cookie_block = (0..MAX_BLOCKED).map(|index| format!("{HEAD}{index:04}{}/", "x".repeat(folder_text))).collect();
    let settings = Settings { home, cookie_block, ..Settings::default() };
    let (read, dropped) = Settings::read(&settings.text());
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(read, settings);
}

#[test]
fn a_bookmark_keeps_the_longest() {
    let mut bookmarks = Bookmarks::default();
    bookmarks.add(BAR, "Long", &longest()).unwrap();
    let (read, dropped) = Bookmarks::read(&bookmarks.text());
    assert!(dropped.is_empty(), "{dropped:?}");
    assert!(read.find(&longest()).is_some());
}

#[test]
fn history_keeps_the_longest() {
    let mut history = History::default();
    let visit = Visit { address: longest(), channel: "Long".into(), title: "TITLE".into(), typed: true, reload: false };
    history.record(&visit, NOW, 0).unwrap();
    let (read, dropped) = History::read(&history.text());
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(read.query("").first().map(|row| row.address.as_str()), Some(longest().as_str()));
}
