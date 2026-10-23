//! Tests of the bookmarks and their file (requirements 20.15 to 20.20).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::file::{FILE_NAME, VERSION_LINE};
use super::*;

fn tree() -> (Bookmarks, u32, u32, u32) {
    let mut bookmarks = Bookmarks::default();
    let news = bookmarks.add(BAR, "News Example", "https://news.example/teletext/").unwrap();
    let weather = bookmarks.add_folder(OTHER, "Weather").unwrap();
    let page = bookmarks.add(weather, "", "https://news.example/teletext/#302").unwrap();
    (bookmarks, news, weather, page)
}

/// Requirement 20.15: the two folders, names, folders, the address's page kept.
#[test]
fn bookmarks_live_in_folders() {
    let (bookmarks, news, weather, page) = tree();
    assert_eq!(bookmarks.children(ROOT), [BAR, OTHER]);
    assert_eq!(bookmarks.children(BAR), [news]);
    assert_eq!(bookmarks.item(page).unwrap().name, "https://news.example/teletext/#302", "an empty name is the address");
    assert_eq!(bookmarks.path(page), [OTHER, weather]);
    assert_eq!(bookmarks.depth(page), 2);
    assert_eq!(bookmarks.find("https://news.example/teletext/#302"), Some(page));
    assert_eq!(bookmarks.search("WEATHER"), [weather]);
    assert_eq!(
        bookmarks.listed(),
        [
            Listed::Bookmark { name: "News Example".into(), address: "https://news.example/teletext/".into() },
            Listed::Folder("Weather".into()),
            Listed::Bookmark { name: "https://news.example/teletext/#302".into(), address: "https://news.example/teletext/#302".into() },
        ]
    );
}

/// Requirement 20.20: a file of exactly the most bookmarks is read
/// whole, and one more is dropped. Before, the reader counted the two
/// folders always there and dropped the last two of 10 000.
#[test]
fn the_most_bookmarks_are_read_whole() {
    use std::fmt::Write as _;
    let mut text = format!("{VERSION_LINE}\n");
    for n in 0..=MAX_ITEMS {
        let _ = writeln!(text, "bookmark\t{}\t{OTHER}\tName\thttps://b{n}.example/", n + 3);
    }
    let (bookmarks, dropped) = Bookmarks::read(&text);
    assert_eq!((bookmarks.len(), dropped.len()), (MAX_ITEMS, 1));
}

/// Requirement 20.15: every bound.
#[test]
fn bookmarks_are_bounded() {
    let mut bookmarks = Bookmarks::default();
    assert!(bookmarks.add(BAR, "x", "not an address").is_err());
    assert!(bookmarks.add(999, "x", "https://a.example/").is_err());
    let mut folder = OTHER;
    for _ in 0..MAX_DEPTH {
        folder = bookmarks.add_folder(folder, "deep").unwrap();
    }
    assert!(bookmarks.add_folder(folder, "too deep").is_err());
    let mut bookmarks = Bookmarks::default();
    for n in 0..MAX_ITEMS {
        bookmarks.add(OTHER, "", &format!("https://b{n}.example/")).unwrap();
    }
    assert!(bookmarks.add(OTHER, "", "https://one-more.example/").is_err());
    assert_eq!(
        bookmarks.add(BAR, &"n".repeat(MAX_NAME + 10), "https://x.example/").map(|_| ()),
        Err(format!("At most {MAX_ITEMS} bookmarks and folders are kept"))
    );
    let mut bookmarks = Bookmarks::default();
    let long = bookmarks.add(BAR, &"n".repeat(MAX_NAME + 10), "https://x.example/").unwrap();
    assert_eq!(bookmarks.item(long).unwrap().name.len(), MAX_NAME);
}

/// Requirement 20.17: remove and undo, move without cycles, sort, edit.
#[test]
fn bookmarks_are_managed() {
    let (mut bookmarks, news, weather, page) = tree();
    let removed = bookmarks.remove(weather).unwrap();
    assert_eq!(removed.name, "Weather");
    assert!(bookmarks.item(page).is_none(), "its contents went with it");
    assert!(bookmarks.restore(removed));
    assert_eq!(bookmarks.path(page), [OTHER, weather]);
    assert!(bookmarks.remove(BAR).is_none());
    assert!(bookmarks.move_to(weather, weather, 0).is_err(), "not into itself");
    bookmarks.move_to(news, weather, 0).unwrap();
    assert_eq!(bookmarks.children(weather), [news, page]);
    bookmarks.move_to(news, weather, 2).unwrap();
    assert_eq!(bookmarks.children(weather), [page, news]);
    bookmarks.sort(weather);
    assert_eq!(bookmarks.children(weather), [page, news], "by name: https… before News");
    bookmarks.edit(page, "Weather page", "").unwrap();
    assert_eq!(bookmarks.item(page).unwrap().address.as_deref(), Some("https://news.example/teletext/#302"));
    assert!(bookmarks.edit(page, "x", "not an address").is_err());
    bookmarks.edit(weather, "Forecasts", "ignored").unwrap();
    assert_eq!(bookmarks.item(weather).unwrap().address, None);
}

/// Requirements 20.19 and 20.20: written, read back the same; written
/// whole; readable by the user alone.
#[test]
fn bookmarks_round_trip_through_their_file() {
    let (bookmarks, ..) = tree();
    let (read, dropped) = Bookmarks::read(&bookmarks.text());
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(read.text(), bookmarks.text());
    assert_eq!(read.listed(), bookmarks.listed());
    let folder = std::env::temp_dir().join(format!("neoteletext-bookmarks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    bookmarks.save(&folder).unwrap();
    assert_eq!(Bookmarks::load(&folder).unwrap().0.text(), bookmarks.text());
    let first = bookmarks.export(&folder, "2026-10-22").unwrap();
    let second = bookmarks.export(&folder, "2026-10-22").unwrap();
    assert!(first.ends_with("bookmarks-export-2026-10-22.txt") && second.ends_with("bookmarks-export-2026-10-22-2.txt"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&folder);
}

/// Requirement 20.20: each rule; a broken line is dropped with what is
/// inside it; no cycle can be read.
#[test]
fn the_file_is_read_strictly() {
    let text = [
        VERSION_LINE,
        "folder\t1\t0\tMy bar",
        "folder\t7\t2\tWeather",
        "bookmark\t8\t7\tForecast\thttps://news.example/teletext/#302",
        "folder\t9\t10\tBefore its folder",
        "folder\t10\t9\tA cycle",
        "bookmark\t11\t10\tInside the cycle\thttps://a.example/",
        "bookmark\t7\t2\tA number taken\thttps://a.example/",
        "bookmark\t12\t8\tInside a bookmark\thttps://a.example/",
        "bookmark\t13\t2\tNot an address\tnot an address",
        "bookmark\t14\t2\tHidden\u{202E}\thttps://a.example/",
        "bookmark\t15\t2\t\thttps://a.example/",
        "bookmark\t16\t2\tExtra\thttps://a.example/\textra",
        "elephant\t17\t2\tWhat",
    ]
    .join("\n");
    let (bookmarks, dropped) = Bookmarks::read(&text);
    assert_eq!(bookmarks.item(BAR).unwrap().name, "My bar");
    assert_eq!(bookmarks.len(), 2, "{:?}", bookmarks.listed());
    assert_eq!(dropped.len(), 10, "{dropped:?}");
    assert!(Bookmarks::read("no version\nfolder\t5\t2\tX").0.is_empty());
    let mut deep = vec![VERSION_LINE.to_owned()];
    let mut parent = OTHER;
    for n in 0..=MAX_DEPTH {
        let id = 100 + u32::try_from(n).unwrap();
        deep.push(format!("folder\t{id}\t{parent}\tdeep"));
        parent = id;
    }
    let (bookmarks, dropped) = Bookmarks::read(&deep.join("\n"));
    assert_eq!((bookmarks.len(), dropped.len()), (MAX_DEPTH, 1), "one folder too deep");
}

/// Bookmarks read from another file go into "Imported", whole.
#[test]
fn an_import_goes_into_its_own_folder() {
    let (mut bookmarks, ..) = tree();
    let (other, ..) = tree();
    assert_eq!(bookmarks.import(&other), Ok(3));
    let imported = bookmarks.children(OTHER).last().copied().unwrap();
    assert_eq!(bookmarks.item(imported).unwrap().name, IMPORTED_NAME);
    assert_eq!(bookmarks.children(imported).len(), 2);
}
