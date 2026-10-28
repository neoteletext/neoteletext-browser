//! Tests of what the address field may open.

use super::*;

#[test]
fn typed_hosts_become_https_addresses() {
    let address = parse("  Sport.Invalid ").unwrap();
    assert_eq!(address.text, "https://sport.invalid/");
    assert_eq!(address.host, "sport.invalid");
    assert!(address.encrypted);
    let address = parse("http://news.invalid:8080/teletext/?page=100").unwrap();
    assert_eq!(address.text, "http://news.invalid:8080/teletext/?page=100");
    assert!(!address.encrypted);
}

#[cfg(unix)]
#[test]
fn files_are_opened_by_full_path() {
    let address = parse("file:///Users/someone/teefax.t42").unwrap();
    assert_eq!(address.file.as_deref(), Some(Path::new("/Users/someone/teefax.t42")));
    assert_eq!(address.host, "teefax.t42");
    assert_eq!(parse("/Users/someone/pages").unwrap().text, "file:///Users/someone/pages");
    assert!(parse("file://relative/path").is_err());
}

#[cfg(windows)]
#[test]
fn windows_files_are_opened_by_drive_path() {
    let address = parse(r"C:\t42\channel.t42").unwrap();
    assert_eq!(address.file.as_deref(), Some(Path::new(r"C:\t42\channel.t42")));
    assert_eq!((address.text.as_str(), address.host.as_str()), ("file:///C:/t42/channel.t42", "channel.t42"));
    assert_eq!(parse("file:///C:/t42/channel.t42").unwrap().file.as_deref(), Some(Path::new("C:/t42/channel.t42")));
    assert!(parse(r"\t42\channel.t42").is_err(), "a path without its drive is not full");
    // A share would make the trusted core open a network connection;
    // a device path, a pipe. Before this both were file addresses.
    assert!(parse(r"\\server\share\channel.t42").is_err());
    assert!(parse(r"\\.\pipe\teletext").is_err());
    assert!(parse("file:////server/share/channel.t42").is_err());
}

#[cfg(unix)]
#[test]
fn a_windows_path_is_not_a_file_elsewhere() {
    assert!(parse(r"C:\t42\channel.t42").is_err());
}

#[test]
fn a_fragment_names_a_page_of_the_channel_and_is_never_fetched() {
    let address = parse("weather.invalid/teletext/#302").unwrap();
    assert_eq!(address.page, Some(0x302));
    assert_eq!(address.text, "https://weather.invalid/teletext/#302");
    assert_eq!(address.channel_text(), "https://weather.invalid/teletext/");
    let with_subpage = parse("weather.invalid/#302:0001").unwrap();
    assert_eq!((with_subpage.page, with_subpage.subpage), (Some(0x302), Some(1)), "decision 67: the sub-page is kept");
    assert_eq!(parse("weather.invalid/#302:3F7F").unwrap().subpage, None, "3F7F means any");
    // Decision 119 (the engine's REQ-NUM-002 and REQ-NUM-006): every value
    // of the thirteen bits names a sub-page, and nothing outside them.
    assert_eq!(parse("weather.invalid/#302:3F7E").unwrap().subpage, Some(0x3F7E));
    assert_eq!(parse("weather.invalid/#302:0A5C").unwrap().subpage, Some(0x0A5C));
    assert_eq!(parse("weather.invalid/#302:0080").unwrap().subpage, None, "bit 7 is control bit C4's, not the sub-code's");
    assert_eq!(parse("weather.invalid/#302:1F80").unwrap().subpage, None);
    assert_eq!(parse("weather.invalid/#302").unwrap().subpage, None);
    assert_eq!(parse("weather.invalid#302").unwrap().text, "https://weather.invalid/#302");
    for fragment in ["#1A0", "#30", "#3022", "#302:1", "#section", "#902"] {
        let address = parse(&format!("weather.invalid/{fragment}")).unwrap();
        assert_eq!(address.page, None, "{fragment} is not a display page");
    }
    let at = address.with_page(0x100).unwrap();
    assert_eq!((at.text.as_str(), at.page), ("https://weather.invalid/teletext/#100", Some(0x100)));
    assert!(at.same_channel(&address) && !at.same_channel(&parse("weather.invalid/other/").unwrap()));
}

/// Requirement 19.7: the start page, and no other `neoteletext:` address.
#[test]
fn the_start_page_is_the_one_built_in_address() {
    let address = parse("neoteletext:home").unwrap();
    assert!(address.is_start_page());
    assert_eq!((address.text.as_str(), address.file.as_deref(), address.encrypted), (START_PAGE, None, false));
    assert_eq!(parse(" NeoTeletext:Home ").unwrap().text, START_PAGE);
    let at = parse("neoteletext:home#201").unwrap();
    assert_eq!((at.text.as_str(), at.page), ("neoteletext:home#201", Some(0x201)));
    assert!(at.is_start_page() && at.same_channel(&address));
    assert_eq!(parse("neoteletext:home#junk").unwrap().text, START_PAGE, "a fragment that is not a page is dropped");
    for typed in ["neoteletext:settings", "neoteletext:", "neoteletext:home/x", "neoteletext://home", "neoteletext:homex"] {
        assert!(parse(typed).is_err(), "{typed:?} should be refused");
    }
    assert!(!parse("https://home.example/").unwrap().is_start_page());
}

#[test]
fn misleading_and_unsupported_addresses_are_refused() {
    for typed in [
        "",
        "javascript:alert(1)",
        "https://bank.example@evil.invalid/",
        "https://exa mple.invalid/",
        "https://example.invalid/\u{202E}gpj",
        "https://bücher.invalid/",
        "https://example.invalid:99999/",
        "https://.invalid/",
    ] {
        assert!(parse(typed).is_err(), "{typed:?} should be refused");
    }
}

/// Finding F-78: an address is never longer than `MAX_ADDRESS` once
/// written in full, with its scheme, its slash and a page, so every one
/// opened can be kept in history, bookmarks and the session, which refuse
/// a longer line. Before, a 2045 to 2048-byte address became longer and
/// lost its visit, its star and its restored tab.
#[test]
fn an_address_is_within_its_bound_written_in_full() {
    let max = protocol::MAX_ADDRESS;
    for length in max - 12..=max {
        let typed = format!("a.example/{}", "x".repeat(length - "a.example/".len()));
        if let Ok(address) = parse(&typed) {
            assert!(address.text.len() <= max, "{length} typed, {} written", address.text.len());
            if let Some(paged) = address.with_page(0x302) {
                assert!(paged.text.len() <= max, "{length} typed, {} with a page", paged.text.len());
            }
        }
    }
    let longest = parse(&format!("https://a.example/{}", "x".repeat(max - "https://a.example/".len()))).unwrap();
    assert_eq!(longest.text.len(), max);
    assert!(longest.with_page(0x302).is_none(), "no room for the page");
}
