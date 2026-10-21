//! The start page, `neoteletext:home` (`settings-home-console-proxy.md`
//! section 2.2, requirement 19.7): a channel the trusted core writes
//! itself as TTI and gives to a decoder as any channel's pages, with no
//! network. Page 100 tells how to open a channel and lists the bookmarks;
//! pages 201 to 299 are one for each bookmark, and opening one opens it.
//!
//! Its text is built only from addresses the address rules accepted, and
//! each is written as printable ASCII cut to its row, so no address can
//! end a TTI line early or carry a control code.

use std::fmt::Write as _;

use protocol::fetcher::Entry;

/// The start page's channel name.
pub const NAME: &str = "NeoTeletext";
/// The page that tells how to open a channel.
pub const FRONT_PAGE: u16 = 0x100;
/// The first bookmark's page, and the last: decimal page numbers 201 to
/// 299, so a bookmark is chosen by typing its number.
const FIRST_BOOKMARK: u16 = 201;
const LAST_BOOKMARK: u16 = 299;
/// The most bookmarks the start page has pages for.
pub const MAX_LISTED: usize = (LAST_BOOKMARK - FIRST_BOOKMARK + 1) as usize;
/// A row's columns (EN 300 706 section 9.3).
const COLUMNS: usize = 40;
/// The rows of page 100 that list bookmarks: rows 9 to 23, row 24 left
/// for the line about the rest.
const LIST_FIRST_ROW: usize = 9;
const LIST_ROWS: usize = 15;
/// Spacing attributes (EN 300 706 Table 26) are written in TTI as an
/// escape followed by the code plus 0x40: these letters are alpha colours.
const ESCAPE: char = '\x1b';
const WHITE: char = 'G';
const YELLOW: char = 'C';
const CYAN: char = 'F';
const GREEN: char = 'B';
/// A bookmark's row on page 100: its colour, its number, a space, a
/// colour and the address.
const LIST_PREFIX: usize = 1 + 3 + 1 + 1;
/// The characters the English G0 set shows as themselves (EN 300 706
/// Table 36): the rest of printable ASCII is drawn as other symbols there,
/// so an address shows them as `?`.
fn shown_as_itself(character: char) -> bool {
    character == ' ' || (character.is_ascii_graphic() && !matches!(character, '#' | '[' | '\\' | ']' | '^' | '_' | '`' | '{' | '|' | '}' | '~'))
}

/// The page number of the bookmark at `index`, from 201.
#[must_use]
pub fn bookmark_page(index: usize) -> Option<u16> {
    let decimal = u16::try_from(index).ok()?.checked_add(FIRST_BOOKMARK).filter(|&decimal| decimal <= LAST_BOOKMARK)?;
    let digit = |power: u16| decimal.checked_div(power).and_then(|value| value.checked_rem(10)).unwrap_or(0);
    Some((digit(100) << 8) | (digit(10) << 4) | digit(1))
}

/// The bookmark that page `page` of the start page opens.
#[must_use]
pub fn bookmark_at(page: u16, bookmarks: &[String]) -> Option<&str> {
    (0..bookmarks.len().min(MAX_LISTED)).find(|&index| bookmark_page(index) == Some(page)).and_then(|index| bookmarks.get(index)).map(String::as_str)
}

/// An address as a row shows it: without `https://`, printable ASCII the
/// set shows as itself, cut to `room` columns.
fn shown(address: &str, room: usize) -> String {
    let address = address.strip_prefix("https://").unwrap_or(address);
    let mut text: String = address.chars().map(|character| if shown_as_itself(character) { character } else { '?' }).take(room).collect();
    if address.chars().count() > room {
        text.pop();
        text.push('.');
    }
    text
}

fn page(out: &mut String, number: u16, description: &str, rows: &[(usize, String)]) {
    let _ = write!(out, "DE,{description}\r\nPN,{number:03X}00\r\nSC,0000\r\nPS,8000\r\n");
    let _ = write!(out, "OL,0,        {ESCAPE}{CYAN}{NAME} {number:03X}\r\n");
    for (row, text) in rows {
        let _ = write!(out, "OL,{row},{text}\r\n");
    }
}

/// The start page's pages, as TTI.
#[must_use]
pub fn pages(bookmarks: &[String]) -> Vec<u8> {
    let listed = bookmarks.len().min(MAX_LISTED);
    let mut front = vec![
        (1, format!("{ESCAPE}{YELLOW}{ESCAPE}\x4d{NAME}")),
        (3, format!("{ESCAPE}{WHITE}Type a channel's address above.")),
        (5, format!("{ESCAPE}{WHITE}Type a page's number to open it.")),
        (6, format!("{ESCAPE}{WHITE}Keys: Up and Down pages, Escape back.")),
    ];
    if listed == 0 {
        front.push((LIST_FIRST_ROW - 1, format!("{ESCAPE}{GREEN}No bookmarks yet: the star keeps one.")));
    } else {
        front.push((LIST_FIRST_ROW - 1, format!("{ESCAPE}{GREEN}Bookmarks")));
    }
    for (index, bookmark) in bookmarks.iter().take(listed.min(LIST_ROWS)).enumerate() {
        let Some(number) = bookmark_page(index) else { break };
        front.push((LIST_FIRST_ROW.saturating_add(index), format!("{ESCAPE}{YELLOW}{number:03X} {ESCAPE}{WHITE}{}", shown(bookmark, COLUMNS - LIST_PREFIX))));
    }
    if listed > LIST_ROWS {
        let last = bookmark_page(listed.saturating_sub(1)).unwrap_or(FRONT_PAGE);
        front.push((
            LIST_FIRST_ROW.saturating_add(LIST_ROWS),
            format!("{ESCAPE}{CYAN}More on pages {:03X} to {last:03X}", bookmark_page(LIST_ROWS).unwrap_or(last)),
        ));
    }
    let mut out = String::new();
    page(&mut out, FRONT_PAGE, NAME, &front);
    for (index, bookmark) in bookmarks.iter().take(listed).enumerate() {
        let Some(number) = bookmark_page(index) else { break };
        let rows = [
            (1, format!("{ESCAPE}{YELLOW}{ESCAPE}\x4dBookmark {number:03X}")),
            (3, format!("{ESCAPE}{WHITE}Opening")),
            (4, format!("{ESCAPE}{CYAN}{}", shown(bookmark, COLUMNS - 1))),
        ];
        page(&mut out, number, "Bookmark", &rows);
    }
    out.into_bytes()
}

/// The start page's contents, for the page list: page 100, then each
/// bookmark's page by its address.
#[must_use]
pub fn contents(bookmarks: &[String]) -> Vec<Entry> {
    let mut entries = vec![Entry::Section(NAME.to_owned()), Entry::Page { page: FRONT_PAGE, title: "Start".to_owned() }];
    entries.push(Entry::Section("Bookmarks".to_owned()));
    for (index, bookmark) in bookmarks.iter().take(MAX_LISTED).enumerate() {
        if let Some(page) = bookmark_page(index) {
            entries.push(Entry::Page { page, title: shown(bookmark, COLUMNS) });
        }
    }
    entries
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_take_the_decimal_pages_from_201() {
        assert_eq!(bookmark_page(0), Some(0x201));
        assert_eq!(bookmark_page(8), Some(0x209));
        assert_eq!(bookmark_page(9), Some(0x210));
        assert_eq!(bookmark_page(98), Some(0x299));
        assert_eq!(bookmark_page(99), None);
        let bookmarks: Vec<String> = (0..120).map(|n| format!("https://b{n}.example/")).collect();
        assert_eq!(bookmark_at(0x210, &bookmarks), Some("https://b9.example/"));
        assert_eq!(bookmark_at(0x20A, &bookmarks), None, "not a decimal page");
        assert_eq!(bookmark_at(0x100, &bookmarks), None);
        assert_eq!(bookmark_at(0x201, &[]), None);
    }

    /// Requirement 19.7: page 100 and a page a bookmark, in TTI.
    #[test]
    fn the_pages_tell_how_to_open_a_channel_and_list_the_bookmarks() {
        let text = String::from_utf8(pages(&["https://news.example/teletext/".into(), "neoteletext:home".into()])).unwrap();
        assert!(text.contains("PN,10000\r\n") && text.contains("PN,20100\r\n") && text.contains("PN,20200\r\n"), "{text}");
        assert!(text.contains("Type a channel's address above."));
        assert!(text.contains("201 \x1bGnews.example/teletext/"), "{text}");
        assert!(!text.contains("PN,20300"));
        let empty = String::from_utf8(pages(&[])).unwrap();
        assert!(empty.contains("No bookmarks yet"));
        assert_eq!(empty.matches("PN,").count(), 1);
    }

    /// Requirement 19.7: whatever an address holds, every line of TTI is
    /// one of the start page's own, and no address carries a control code
    /// or a line's end.
    #[test]
    fn a_hostile_address_cannot_break_out_of_its_line() {
        let hostile = [
            "https://a.example/\r\nPN,10000\r\nOL,1,forged".to_owned(),
            "https://a.example/\x1bA\x1bM".to_owned(),
            format!("https://a.example/{}", "x".repeat(4000)),
            "https://a.example/\u{202E}\u{0}é#_[]".to_owned(),
        ];
        let text = String::from_utf8(pages(&hostile)).unwrap();
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
        assert_eq!(numbers.len(), 1 + hostile.len());
    }

    #[test]
    fn the_contents_name_each_bookmark() {
        let entries = contents(&["https://news.example/".into()]);
        assert!(entries.contains(&Entry::Page { page: 0x201, title: "news.example/".into() }));
        assert!(entries.contains(&Entry::Page { page: FRONT_PAGE, title: "Start".into() }));
    }
}
