//! Tests of the session file (requirements 20.21 to 20.23).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;
use crate::dropped::Dropped;

fn entry(address: &str) -> Entry {
    Entry { by_page: false, at: 1_792_656_300, address: address.into(), title: "News".into() }
}

fn tab(muted: bool, position: usize, entries: Vec<Entry>) -> Tab {
    Tab { muted, position, entries, window: 0 }
}

fn saved() -> Saved {
    Saved {
        windows: vec![
            Window {
                geometry: Some(WindowGeometry { x: -120, y: 80, width: 1440, height: 900, maximised: false }),
                active: 1,
                tabs: vec![
                    tab(false, 1, vec![entry("https://news.example/teletext/"), Entry { by_page: true, ..entry("https://news.example/teletext/#302") }]),
                    tab(true, 0, vec![entry(crate::address::START_PAGE)]),
                ],
            },
            Window { geometry: None, active: 0, tabs: vec![tab(false, 0, vec![entry("https://weather.example/")])] },
        ],
        closed: vec![
            tab(false, 0, vec![Entry { at: 0, ..entry("https://sport.example/") }]),
            Tab { window: 7, ..tab(false, 0, vec![entry("https://tv.example/")]) },
            Tab { window: 7, ..tab(true, 0, vec![entry("https://radio.example/")]) },
        ],
    }
}

/// Requirements 20.21 and 20.23: written, read back the same; written
/// whole, readable by the user alone.
#[test]
fn the_session_round_trips_through_its_file() {
    let saved = saved();
    assert_eq!(Saved::read(&saved.text()), (saved.clone(), Dropped::default()));
    let folder = std::env::temp_dir().join(format!("neoteletext-session-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    assert_eq!(Saved::load(&folder).unwrap(), (None, Dropped::default()), "no file: none");
    Saved::save(&folder, &saved.text()).unwrap();
    assert_eq!(Saved::load(&folder).unwrap().0, Some(saved.clone()));
    // A temporary file left by a crash before the rename is ignored.
    std::fs::write(folder.join(TEMPORARY_NAME), "neoteletext-session 1\nactive\t0").unwrap();
    assert_eq!(Saved::load(&folder).unwrap().0, Some(saved));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&folder);
}

/// Requirement 20.23: each rule; a broken tab goes with its entries; an
/// active place beyond the tabs is the first.
#[test]
fn the_file_is_read_strictly() {
    let text = [
        VERSION_LINE,
        "window\t10\t20\t0\t900\t0",
        "window\t10\t20\t1440\t900\t0",
        "active\t7",
        "tab\t0\t0",
        "entry\t0\t5\thttps://a.example/\tA",
        "tab\t2\t0",
        "entry\t0\t5\thttps://b.example/\tB",
        "tab\t0\t5",
        "entry\t0\t5\thttps://c.example/\tC",
        "tab\t0\t0",
        "entry\t0\t5\tnot an address\tD",
        "tab\t0\t0",
        "entry\t0\t5\thttps://e.example/\tE\u{202E}",
        "tab\t0\t0",
        "entry\t0\t5\thttps://f.example/\tF",
        "elephant",
        "closed\t0\t0",
        "entry\t0\t-1\thttps://g.example/\tG",
        "closed\t0\t0",
        "entry\t0\t9\thttps://h.example/\tH",
        "closed\t0\t0\t3",
        "entry\t0\t9\thttps://i.example/\tI",
        "tab\t0\t0\t3",
        "entry\t0\t9\thttps://j.example/\tJ",
        "window",
        "active\t0",
        "window\tx",
    ]
    .join("\n");
    let (saved, dropped) = Saved::read(&text);
    // The last window had no tab and is not one.
    let [window] = saved.windows.as_slice() else { panic!("one window: {:?}", saved.windows) };
    let addresses: Vec<&str> = window.tabs.iter().map(|tab| tab.entries[0].address.as_str()).collect();
    assert_eq!(addresses, ["https://a.example/", "https://f.example/"]);
    assert_eq!(window.geometry, Some(WindowGeometry { x: 10, y: 20, width: 1440, height: 900, maximised: false }));
    assert_eq!(window.active, 0, "seven is beyond the tabs");
    let line = |n: usize| format!("line {n} of the session file dropped");
    let tab = "a tab of the session file dropped".to_owned();
    let closed: Vec<(&str, i64, u32)> = saved.closed.iter().map(|tab| (tab.entries[0].address.as_str(), tab.entries[0].at, tab.window)).collect();
    assert_eq!(closed, [("https://h.example/", 9, 0), ("https://i.example/", 9, 3)], "a time before 1970 breaks its tab");
    // An open tab has no window number; a window line with one field is
    // broken.
    assert_eq!(
        dropped.lines(),
        [line(2), line(7), tab.clone(), tab.clone(), line(12), tab.clone(), line(14), tab.clone(), line(17), line(19), tab.clone(), line(24), tab, line(28)]
    );
    assert_eq!(Saved::read("no version\ntab\t0\t0").0, Saved::default());
    let (saved, dropped) = Saved::read("neoteletext-session 1\ntab\t0\t0\nentry\t0\thttps://a.example/\tA");
    assert_eq!((saved, dropped.len()), (Saved::default(), 1), "the first version is not read");
}

/// Section 15.1: a long list is kept around its place.
#[test]
fn a_long_list_is_kept_around_its_place() {
    let entries: Vec<Entry> = (0..200).map(|n| entry(&format!("https://p{n}.example/"))).collect();
    let tab = Tab::trimmed(false, 150, entries.clone());
    assert_eq!(tab.entries.len(), MAX_ENTRIES);
    assert_eq!(tab.entries[tab.position].address, "https://p150.example/");
    let tab = Tab::trimmed(false, 199, entries.clone());
    assert_eq!(tab.entries[tab.position].address, "https://p199.example/");
    let tab = Tab::trimmed(false, 0, entries);
    assert_eq!((tab.position, tab.entries.len()), (0, MAX_ENTRIES));
}
