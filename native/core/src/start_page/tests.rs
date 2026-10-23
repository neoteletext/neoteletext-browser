//! Tests of the start page, `neoteletext:home`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

use super::*;

/// Bookmarks named by their addresses.
fn named(addresses: &[String]) -> Vec<Listed> {
    addresses.iter().map(|address| Listed::Bookmark { name: address.clone(), address: address.clone() }).collect()
}

#[test]
fn bookmarks_take_the_decimal_pages_from_201() {
    assert_eq!(bookmark_page(0), Some(0x201));
    assert_eq!(bookmark_page(8), Some(0x209));
    assert_eq!(bookmark_page(9), Some(0x210));
    assert_eq!(bookmark_page(98), Some(0x299));
    assert_eq!(bookmark_page(99), None);
    let mut listed = named(&(0..120).map(|n| format!("https://b{n}.example/")).collect::<Vec<_>>());
    listed.insert(3, Listed::Folder("A folder takes no page".into()));
    assert_eq!(bookmark_at(0x210, &listed).as_deref(), Some("https://b9.example/"));
    assert_eq!(bookmark_at(0x20A, &listed), None, "not a decimal page");
    assert_eq!(bookmark_at(0x100, &listed), None);
    assert_eq!(bookmark_at(0x201, &[]), None);
}

/// Requirement 19.7: page 100 and a page a bookmark, in TTI; a
/// bookmark shows its name, and its page its address too.
#[test]
fn the_pages_tell_how_to_open_a_channel_and_list_the_bookmarks() {
    let listed = [
        Listed::Bookmark { name: "News Example".into(), address: "https://news.example/teletext/".into() },
        Listed::Folder("Weather".into()),
        Listed::Bookmark { name: "Start".into(), address: "neoteletext:home".into() },
    ];
    let text = String::from_utf8(pages(&listed)).unwrap();
    assert!(text.contains("PN,10000\r\n") && text.contains("PN,20100\r\n") && text.contains("PN,20200\r\n"), "{text}");
    assert!(text.contains("Type a channel's address above."));
    assert!(text.contains("201 \x1bGNews Example"), "{text}");
    assert!(text.contains("\x1bBWeather"), "the folder's name as a heading: {text}");
    assert!(text.contains("\x1bFnews.example/teletext/"), "the bookmark's page shows its address: {text}");
    assert!(!text.contains("PN,20300"));
    let empty = String::from_utf8(pages(&[])).unwrap();
    assert!(empty.contains("No bookmarks yet"));
    assert_eq!(empty.matches("PN,").count(), 1);
}

/// Requirement 19.7: whatever a name or address holds, every line of
/// TTI is one of the start page's own, and none carries a control code
/// or a line's end.
#[test]
fn a_hostile_address_cannot_break_out_of_its_line() {
    let hostile = [
        "https://a.example/\r\nPN,10000\r\nOL,1,forged".to_owned(),
        "https://a.example/\x1bA\x1bM".to_owned(),
        format!("https://a.example/{}", "x".repeat(4000)),
        "https://a.example/\u{202E}\u{0}é#_[]".to_owned(),
    ];
    let mut listed = named(&hostile);
    listed.push(Listed::Folder("\r\nPN,10000\r\nOL,1,forged\x1bA".into()));
    listed.push(Listed::Bookmark { name: "\r\nPN,10000\x1bM".into(), address: "https://b.example/".into() });
    let text = String::from_utf8(pages(&listed)).unwrap();
    for line in text.split("\r\n").filter(|line| !line.is_empty()) {
        let (code, rest) = line.split_once(',').unwrap();
        assert!(["DE", "PN", "SC", "PS", "OL"].contains(&code), "{line:?}");
        if code == "OL" {
            let (row, row_text) = rest.split_once(',').unwrap();
            assert!(row.parse::<usize>().unwrap() < 25);
            // Each escape is one of the page's own colours or double height.
            let mut escapes = row_text.split(ESCAPE).skip(1);
            assert!(escapes.all(|after| after.starts_with([WHITE, YELLOW, CYAN, GREEN, '\x4d'])), "{line:?}");
            assert!(row_text.chars().all(|character| character == ESCAPE || (' '..='~').contains(&character)), "{line:?}");
            assert!(row_text.chars().filter(|&character| character != ESCAPE).count() <= COLUMNS, "{line:?}");
        }
    }
    let numbers: Vec<&str> = text.split("\r\n").filter(|line| line.starts_with("PN,")).collect();
    assert_eq!(numbers.iter().filter(|line| **line == "PN,10000").count(), 1, "no forged page");
    assert_eq!(numbers.len(), 1 + hostile.len() + 1);
}

#[test]
fn the_contents_name_each_bookmark() {
    let entries = contents(&[Listed::Bookmark { name: "News".into(), address: "https://news.example/".into() }]);
    assert!(entries.contains(&Entry::Page { page: 0x201, title: "News".into() }));
    assert!(entries.contains(&Entry::Page { page: FRONT_PAGE, title: "Start".into() }));
}
