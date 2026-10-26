//! Tests of cookies in the running browser (requirements 21.1 to 21.8).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::super::test_support::Test;
use super::*;
use protocol::cookies::CookiesRequest;

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

/// Finding F-27: a fetcher learns whether a host was reached securely only
/// for its own channel's host, from its own tab's jar: not for another
/// host, and not from the private jar for an ordinary tab.
#[test]
fn a_fetcher_learns_only_its_own_hosts_secure_reach() {
    let mut test = Test::new("reached");
    let news = test.tab("https://news.example/teletext/", false);
    let other = test.tab("https://other.example/teletext/", false);
    test.set(news, "news.example", "id=1");
    let reached = |test: &mut Test, channel: u32, host: &str| {
        let question = FromFetcher::ReachedQuestion { id: 4, host: host.into() };
        let Some(ToFetcher::ReachedAnswer { id: 4, reached }) = test.core.cookie_reply(channel, question).unwrap() else { panic!("no answer") };
        reached
    };
    assert!(reached(&mut test, news, "news.example"), "its own host");
    assert!(!reached(&mut test, other, "news.example"), "another channel's host");
    assert!(!reached(&mut test, 999, "news.example"), "no such channel");
    let private = test.tab("https://secret.example/teletext/", true);
    test.set(private, "secret.example", "p=1");
    assert!(reached(&mut test, private, "secret.example"), "the private tab's own");
    let ordinary = test.tab("https://secret.example/teletext/", false);
    assert!(!reached(&mut test, ordinary, "secret.example"), "an ordinary tab learns nothing of the private jar");
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

/// Decision 113: started with `--no-keychain`, the core never asks the
/// broker for the key, says cookies are kept for the run only, and leaves
/// the kept cookies' file as it was for the next ordinary start.
#[test]
fn with_no_keychain_the_key_is_not_asked_and_the_file_is_left_alone() {
    let mut test = Test::new("no-keychain");
    let file = test.folder.join(kept::FILE_NAME);
    let before = b"sealed cookies of an ordinary start".to_vec();
    std::fs::write(&file, &before).unwrap();
    test.core.settings.cookies = Mode::Keep;
    test.core.cookies.no_keychain = true;
    test.core.cookies_at_start();
    assert_eq!(test.core.cookies.key, None, "no key was had");
    let notice = test.core.cookies.for_this_run.clone().unwrap_or_default();
    assert!(notice.contains(NO_KEYCHAIN), "the panel says why: {notice}");
    let news = test.tab("https://news.example/teletext/", false);
    test.set(news, "news.example", "keep=1; Max-Age=3600");
    assert_eq!(test.ask(news, "news.example"), "keep=1", "cookies work for the run");
    test.core.keep_cookies();
    test.core.cookies_at_exit();
    assert_eq!(std::fs::read(&file).unwrap(), before, "the file is neither read into the run nor replaced");
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
    assert!(restored.import(&opened, now_unix()).is_empty());
    assert_eq!(restored.listed()[0].count, 1, "the session cookie is not written");
    test.core.cookie_mode_changed(protocol::settings::CookieMode::Close);
    assert_eq!(kept::load(&test.folder).unwrap(), None, "on close: no file");
    test.set(news, "news.example", "more=1; Max-Age=3600");
    test.core.keep_cookies();
    assert_eq!(kept::load(&test.folder).unwrap(), None, "and none written");
}
