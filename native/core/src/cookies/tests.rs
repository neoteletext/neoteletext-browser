//! Tests of the cookie store and its sealed file (requirements 21.1 to 21.11).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;

const NOW: u64 = 1_792_000_000;

fn news() -> Channel {
    Channel::of("https://news.example/teletext/channel.json").unwrap()
}

fn header(store: &mut Store, private: bool, channel: &Channel, path: &str) -> String {
    String::from_utf8(store.header_for(private, channel, ("news.example", path, true), Site::SameService, NOW)).unwrap()
}

/// Requirement 7.1: a channel is its scheme, host, port and folder.
#[test]
fn a_channel_is_its_folder_on_its_origin() {
    assert_eq!(news().key, "https://news.example/teletext/");
    assert_eq!(news().origin(), "https://news.example");
    assert_eq!(Channel::of("http://news.example:8080/a/b/100.tti").unwrap().key, "http://news.example:8080/a/b/");
    assert_eq!(Channel::of("file:///tmp/x.t42"), None);
    assert_eq!(Channel::of("neoteletext:home"), None);
}

/// Requirements 21.1 and 21.11: a channel's cookies go only to its own
/// requests: not to another folder, scheme or port of the same host.
#[test]
fn a_channels_cookies_stay_in_the_channel() {
    let mut store = Store::new();
    let news = news();
    store.store(false, &news, ("news.example", "/teletext/100.tti", true), b"id=1; Path=/", NOW).unwrap();
    assert_eq!(header(&mut store, false, &news, "/teletext/101.tti"), "id=1", "the path narrowed to the folder");
    assert_eq!(header(&mut store, false, &news, "/other/"), "", "another folder");
    let neighbour = Channel::of("https://news.example/other/").unwrap();
    assert_eq!(header(&mut store, false, &neighbour, "/other/"), "", "another channel on the origin");
    let plain = Channel::of("http://news.example/teletext/").unwrap();
    assert_eq!(String::from_utf8(store.header_for(false, &plain, ("news.example", "/teletext/", false), Site::SameService, NOW)).unwrap(), "");
    let port = Channel::of("https://news.example:8443/teletext/").unwrap();
    assert_eq!(header(&mut store, false, &port, "/teletext/"), "");
    // A fetcher reporting another channel's host is refused.
    assert_eq!(store.store(false, &news, ("evil.example", "/teletext/", true), b"x=1", NOW), Err(Refused::NotTheChannel));
    assert_eq!(store.store(false, &news, ("news.example", "/teletext/", false), b"x=1", NOW), Err(Refused::NotTheChannel));
}

/// Requirements 21.3 and 21.7: private cookies apart and gone with the
/// last private tab; blocking refuses and deletes.
#[test]
fn private_and_blocked_cookies() {
    let mut store = Store::new();
    let news = news();
    store.store(true, &news, ("news.example", "/teletext/", true), b"p=1", NOW).unwrap();
    assert_eq!(header(&mut store, false, &news, "/teletext/"), "", "not in the ordinary tabs");
    assert_eq!(header(&mut store, true, &news, "/teletext/"), "p=1");
    assert!(!store.changed(), "a private cookie is never to be written");
    store.close_private();
    assert_eq!(header(&mut store, true, &news, "/teletext/"), "");

    store.store(false, &news, ("news.example", "/teletext/", true), b"o=1", NOW).unwrap();
    store.block(&news.key, true);
    assert_eq!(header(&mut store, false, &news, "/teletext/"), "", "deleted when blocked");
    assert_eq!(store.store(false, &news, ("news.example", "/teletext/", true), b"o=2", NOW), Err(Refused::Blocked));
    store.block(&news.key, false);
    store.mode = Mode::Block;
    assert_eq!(store.store(false, &news, ("news.example", "/teletext/", true), b"o=3", NOW), Err(Refused::Blocked));
}

/// Requirements 21.6, 21.8 and 21.9: listing, deleting by channel,
/// site, time and all, and `Clear-Site-Data` for the channel alone.
#[test]
fn listing_and_deleting() {
    let mut store = Store::new();
    let news = news();
    let sport = Channel::of("https://news.example/sport/").unwrap();
    let other = Channel::of("https://other.example/tt/").unwrap();
    store.store(false, &news, ("news.example", "/teletext/", true), b"a=1", NOW).unwrap();
    store.store(false, &news, ("news.example", "/teletext/", true), b"b=22", NOW + 5).unwrap();
    store.store(false, &sport, ("news.example", "/sport/", true), b"c=1", NOW + 10).unwrap();
    store.store(false, &other, ("other.example", "/tt/", true), b"d=1", NOW + 20).unwrap();
    let listed = store.listed();
    let summary: Vec<(&str, &str, usize, u64)> = listed.iter().map(|row| (row.key.as_str(), row.origin.as_str(), row.count, row.used_at)).collect();
    assert_eq!(
        summary,
        [
            ("https://news.example/sport/", "https://news.example", 1, NOW + 10),
            ("https://news.example/teletext/", "https://news.example", 2, NOW + 5),
            ("https://other.example/tt/", "https://other.example", 1, NOW + 20)
        ]
    );
    store.clear_site(false, &sport, "news.example");
    assert_eq!(store.listed().len(), 2, "the sport channel alone");
    assert_eq!(store.delete_created(NOW + 1, NOW + 30), 2);
    assert_eq!(header(&mut store, false, &news, "/teletext/"), "a=1");
    assert_eq!(store.delete_origin("https://news.example"), 1);
    store.store(false, &other, ("other.example", "/tt/", true), b"e=1", NOW).unwrap();
    store.delete_all();
    assert_eq!(store.listed(), []);
    let plain = Channel::of("http://plain.example/tt/").unwrap();
    store.store(false, &plain, ("plain.example", "/tt/", false), b"f=1", NOW).unwrap();
    store.clear_site(false, &plain, "plain.example");
    assert_eq!(store.listed().len(), 1, "never over plain HTTP");
}

/// Requirement 21.10: at most 180 cookies a channel and 3 000 in all, the
/// least recently used going first.
#[test]
fn cookies_are_bounded() {
    let mut store = Store::new();
    let news = news();
    for number in 0..=PER_CHANNEL {
        store.store(false, &news, ("news.example", "/teletext/", true), format!("n{number}=1").as_bytes(), NOW + number as u64).unwrap();
    }
    assert_eq!(store.listed()[0].count, PER_CHANNEL);
    assert!(!header(&mut store, false, &news, "/teletext/").split("; ").any(|pair| pair == "n0=1"), "the oldest went");
    let channels = TOTAL / PER_CHANNEL + 1;
    for channel in 0..channels {
        let address = format!("https://c{channel}.example/tt/");
        let other = Channel::of(&address).unwrap();
        for number in 0..PER_CHANNEL {
            store.store(false, &other, (&format!("c{channel}.example"), "/tt/", true), format!("n{number}=1").as_bytes(), NOW).unwrap();
        }
    }
    assert_eq!(store.listed().iter().map(|row| row.count).sum::<usize>(), TOTAL);
}

/// Requirements 21.4 and 21.5: the file round trips persistent cookies
/// only, with their channels, and is read strictly.
#[test]
fn the_file_keeps_persistent_cookies_of_named_channels() {
    let mut store = Store::new();
    let news = news();
    store.store(false, &news, ("news.example", "/teletext/", true), b"keep=1; Max-Age=3600; Secure", NOW).unwrap();
    store.store(false, &news, ("news.example", "/teletext/", true), b"session=1", NOW).unwrap();
    store.store(true, &news, ("news.example", "/teletext/", true), b"private=1; Max-Age=3600", NOW).unwrap();
    let text = store.export(NOW);
    let shown = String::from_utf8_lossy(&text);
    assert!(shown.starts_with("neoteletext-cookie-file 1\nchannel\tc1\thttps://news.example/teletext/\n\n"), "{shown}");
    assert!(!shown.contains("session") && !shown.contains("private"), "{shown}");
    let mut restored = Store::new();
    assert_eq!(restored.import(&text, NOW + 10), Vec::<String>::new());
    assert_eq!(header(&mut restored, false, &news, "/teletext/"), "keep=1");
    // Another channel made in the next run gets a tag of its own.
    let other = Channel::of("https://other.example/tt/").unwrap();
    restored.store(false, &other, ("other.example", "/tt/", true), b"x=1", NOW).unwrap();
    assert_eq!(header(&mut restored, false, &news, "/teletext/"), "keep=1");

    // Strictness: each broken table line dropped, stray cookies dropped.
    let mut broken = text.clone();
    let table_end = broken.windows(2).position(|pair| pair == b"\n\n").unwrap() + 1;
    broken.splice(table_end..table_end, b"channel\tc2\tnot an address\nchannel\tx9\thttps://a.example/\n".iter().copied());
    let mut strict = Store::new();
    assert_eq!(strict.import(&broken, NOW).len(), 2);
    assert_eq!(header(&mut strict, false, &news, "/teletext/"), "keep=1");
    let unnamed = String::from_utf8_lossy(&text).replace("channel\tc1\thttps://news.example/teletext/\n", "");
    let mut stray = Store::new();
    assert_eq!(stray.import(unnamed.as_bytes(), NOW), ["1 cookies of no channel, or a blocked one, dropped"]);
    assert_eq!(stray.listed(), []);
    assert_eq!(Store::new().import(b"garbage", NOW).len(), 1);
    // A store that never kept a cookie writes a file it can read.
    assert_eq!(Store::new().import(&Store::new().export(NOW), NOW), Vec::<String>::new());
}

/// Requirement 21.4: the file on the disk is sealed: no cookie in the
/// clear, unreadable under another key or once changed, mode 0600.
#[test]
fn the_file_is_sealed() {
    let key = [7_u8; KEY_BYTES];
    let mut store = Store::new();
    let news = news();
    store.store(false, &news, ("news.example", "/teletext/", true), b"secret-value=42; Max-Age=3600", NOW).unwrap();
    let folder = std::env::temp_dir().join(format!("neoteletext-cookies-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    assert_eq!(load(&folder).unwrap(), None);
    save(&folder, &key, &store, NOW).unwrap();
    let sealed = load(&folder).unwrap().unwrap();
    assert!(!sealed.windows(b"secret-value".len()).any(|window| window == b"secret-value"), "no cookie in the clear");
    assert!(!sealed.windows(b"news.example".len()).any(|window| window == b"news.example"), "no channel in the clear");
    let mut restored = Store::new();
    assert_eq!(restored.import(&open(&key, &sealed).unwrap(), NOW), Vec::<String>::new());
    assert_eq!(header(&mut restored, false, &news, "/teletext/"), "secret-value=42");
    assert_eq!(open(&[8_u8; KEY_BYTES], &sealed), None, "another key");
    let mut changed = sealed.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert_eq!(open(&key, &changed), None, "changed");
    assert_ne!(seal(&key, b"x").unwrap(), seal(&key, b"x").unwrap(), "a fresh nonce each time");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
    }
    remove(&folder).unwrap();
    assert_eq!(load(&folder).unwrap(), None);
    let _ = std::fs::remove_dir_all(&folder);
}
