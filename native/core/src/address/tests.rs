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

// DEC-1722: the address field adds a channel's missing final slash.
#[test]
fn the_address_field_adds_a_channels_final_slash() {
    let typed = |text: &str| parse_typed(text).unwrap().text;
    assert_eq!(typed("app.neottx.com/home"), "https://app.neottx.com/home/");
    assert_eq!(typed("https://app.neottx.com/a/home"), "https://app.neottx.com/a/home/");
    assert_eq!(typed("app.neottx.com/home?who=me"), "https://app.neottx.com/home/?who=me");
    assert_eq!(typed("app.neottx.com/home/302"), "https://app.neottx.com/home/302");
    assert_eq!(typed("app.neottx.com/home/"), "https://app.neottx.com/home/");
}

// DEC-1722 item 3: nothing else is repaired, and `parse`, which reads links,
// bookmarks and redirects, adds no slash.
#[test]
fn only_the_address_field_adds_a_slash_and_only_to_a_folder() {
    assert_eq!(parse("app.neottx.com/home").unwrap_err(), NOT_A_CHANNEL);
    assert_eq!(parse_typed("app.neottx.com/home/page.txt").unwrap_err(), NOT_A_CHANNEL);
    assert_eq!(parse_typed("app.neottx.com/home/channel.json").unwrap_err(), parse("app.neottx.com/home/channel.json").unwrap_err());
    assert_eq!(parse_typed("app.neottx.com/home/#G02").unwrap_err(), parse("app.neottx.com/home/#G02").unwrap_err());
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
fn a_path_names_a_page_of_the_channel_and_the_channel_is_its_folder() {
    let address = parse("weather.invalid/teletext/302").unwrap();
    assert_eq!(address.page, Some(0x302));
    assert_eq!(address.text, "https://weather.invalid/teletext/302");
    assert_eq!(address.channel_text(), "https://weather.invalid/teletext/");
    let with_subpage = parse("weather.invalid/302/0001").unwrap();
    assert_eq!((with_subpage.page, with_subpage.subpage), (Some(0x302), Some(1)), "decision 67: the sub-page is kept");
    // Decision 119 (the engine's REQ-NUM-002 and REQ-NUM-006): every value
    // of the thirteen bits names a sub-page.
    assert_eq!(parse("weather.invalid/302/3F7E").unwrap().subpage, Some(0x3F7E));
    assert_eq!(parse("weather.invalid/302/0A5C").unwrap().subpage, Some(0x0A5C));
    assert_eq!(parse("weather.invalid/302").unwrap().subpage, None);
    assert_eq!(parse("weather.invalid/1A0").unwrap().page, Some(0x1A0), "every page the standard defines");
    assert!(parse("weather.invalid/#302").is_err(), "DEC-1694: a fragment is refused");
    let at = address.with_page(0x100).unwrap();
    assert_eq!((at.text.as_str(), at.page), ("https://weather.invalid/teletext/100", Some(0x100)));
    assert!(at.same_channel(&address) && !at.same_channel(&parse("weather.invalid/other/").unwrap()));
}

/// REQ-1693's Verification: every `open` and `refuse` row of the engine's
/// one table, by the address rules. A `refuse` row fails on an address that
/// opens, and an `open` row on a spelling not given as the row says.
#[test]
fn the_conformance_table_holds_for_the_address_rules() {
    let (mut opened, mut refused) = (0_usize, 0_usize);
    for line in neoteletext_web::map::CONFORMANCE.lines().filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let columns: Vec<&str> = line.split('\t').collect();
        match columns.as_slice() {
            ["open", address, canonical] => {
                let read = parse(address).unwrap_or_else(|why| panic!("{address}: {why}"));
                assert_eq!(read.text, *canonical, "{address}");
                assert_eq!(parse(canonical).as_ref(), Ok(&read), "{canonical} reads as it is written");
                opened += 1;
            }
            ["refuse", address, _why] => {
                assert!(parse(address).is_err(), "{address}");
                refused += 1;
            }
            ["answer", ..] => {}
            _ => panic!("a row of the table: {line}"),
        }
    }
    assert_eq!((opened, refused), (30, 33), "the table's rows, all of them read");
}

/// REQ-1693 items 1 and 6: what an address holds beyond its page, read once.
#[test]
fn an_address_holds_its_query_and_contents() {
    let channel = "https://weather.invalid/teletext/";
    let with = parse(&format!("{channel}302/0002?b=%41&a=1")).unwrap();
    assert_eq!(with.parameters(), Some("b=%41&a=1"), "as they came");
    assert_eq!(with.parameters_of(0x302, 2), "b=%41&a=1");
    assert_eq!(with.parameters_of(0x302, SUBCODE_ANY), "", "another selection");
    assert_eq!(with.parameters_of(0x303, 2), "", "another selection");
    let page = parse(&format!("{channel}302?a=1")).unwrap();
    assert_eq!(page.parameters_of(0x302, SUBCODE_ANY), "a=1");
    assert_eq!(page.with_page(0x303).unwrap().parameters(), None, "another page leaves them behind");
    assert_eq!(page.with_subpage(0x302, Some(1)).unwrap().parameters(), None);
    assert_eq!(page.with_wall().unwrap().parameters(), None);
    assert_eq!(page.with_parameters(Some("b=2")).unwrap().text, format!("{channel}302?b=2"));
    assert_eq!(page.with_parameters(None).unwrap().text, format!("{channel}302"));
    let contents = parse(&format!("{channel}contents")).unwrap();
    assert!(contents.contents && !contents.wall && !contents.is_bare());
    assert_eq!((contents.page, contents.parameters()), (None, None));
    assert_eq!(parse(&contents.text).unwrap(), contents);
    assert!(contents.same_channel(&page));
    assert!(parse(channel).unwrap().is_bare());
    let bare = parse(&format!("{channel}?city=x")).unwrap();
    assert!(bare.is_bare() && bare.same_channel(&page), "a query names no other channel");
    assert_eq!(bare.parameters_of(0x100, SUBCODE_ANY), "city=x", "DEC-1730: it goes with the arrival");
}

/// REQ-1693 and REQ-1690: the map's forms, written and read in one place.
#[test]
fn an_address_writes_and_reads_the_maps_forms() {
    let channel = parse("https://weather.invalid/teletext/").unwrap();
    let sub_page = channel.with_subpage(0x302, Some(2)).unwrap();
    assert_eq!(sub_page.text, "https://weather.invalid/teletext/302/0002", "a sub-page");
    assert_eq!((sub_page.page, sub_page.subpage, sub_page.wall), (Some(0x302), Some(2), false));
    assert_eq!(parse(&sub_page.text).unwrap(), sub_page, "it reads back as it was written");
    let page = sub_page.with_page(0x100).unwrap();
    assert_eq!((page.text.as_str(), page.subpage), ("https://weather.invalid/teletext/100", None));
    let wall = sub_page.with_wall().unwrap();
    assert_eq!(wall.text, "https://weather.invalid/teletext/wall");
    assert_eq!((wall.page, wall.subpage, wall.wall), (None, None, true));
    assert_eq!(parse(&wall.text).unwrap(), wall);
    assert!(wall.same_channel(&channel));
    assert!(parse("weather.invalid/#WALL").is_err(), "in small letters");
    assert!(parse("weather.invalid/#wall2").is_err());
}

/// REQ-1693 item 9, REQ-1690 item 9: a file's address names its place as a
/// channel's does, and a `#` in its path is the path's.
#[cfg(unix)]
#[test]
fn a_files_address_names_its_page() {
    let file = parse("/tmp/news.tti").unwrap();
    let page = file.with_page(0x302).unwrap();
    assert_eq!(page.text, "file:///tmp/news.tti#302");
    assert_eq!(page.file.as_deref(), file.file.as_deref(), "the same file");
    assert!(page.same_channel(&file));
    assert_eq!(parse(&page.text).unwrap(), page, "opening it opens that page");
    let sub_page = file.with_subpage(0x302, Some(2)).unwrap();
    assert_eq!(parse(&sub_page.text).unwrap(), sub_page);
    assert_eq!(parse(&file.with_wall().unwrap().text).unwrap().file, file.file);
    let odd = parse("/tmp/a#100%b.tti").unwrap();
    assert_eq!(odd.text, "file:///tmp/a%23100%25b.tti");
    assert_eq!(odd.file.as_deref(), Some(Path::new("/tmp/a#100%b.tti")));
    assert_eq!(odd.page, None);
    assert_eq!(parse(&odd.text).unwrap(), odd, "it reads back");
    assert_eq!(typed_path(&page.text, None).as_deref(), Some("/tmp/news.tti"), "a fragment is no part of the path");
    assert_eq!(parse("file:///tmp/news.tti#1fe").unwrap().text, "file:///tmp/news.tti#1FE");
    assert!(parse("file:///tmp/news.tti#contents").unwrap().contents);
    assert_eq!(parse("file:///tmp/news.tti#302?a=1").unwrap().parameters(), Some("a=1"), "kept, and they do nothing");
    for refused in ["#", "#G02", "#302/3F7F", "#wall?a=1", "#section"] {
        assert!(parse(&format!("file:///tmp/news.tti{refused}")).is_err(), "{refused}");
    }
    assert_eq!(parse("file:///tmp/302.tti").unwrap().page, None, "a local file's name is never read as a page's");
}

/// REQ-1690 item 10: a channel's address leaves room for the longest
/// fragment a tab can add, so that the tab's address can always follow it.
#[test]
fn a_channels_address_leaves_room_for_a_sub_page() {
    let head = "https://a.invalid/";
    let channel = |length| format!("{head}?q={}", "x".repeat(length));
    let fits = protocol::MAX_ADDRESS - head.len() - "?q=302/0002".len();
    let address = parse(&channel(fits)).unwrap();
    assert!(address.with_subpage(0x302, Some(2)).is_some());
    assert!(parse(&channel(fits + 1)).is_err(), "no room");
}

/// TODO-1662: the address rules and the engine's parser agree on a host, and
/// what is fetched is the channel's folder. A `#` cannot hide the real host.
#[test]
fn the_address_rules_and_the_engines_parser_agree_on_an_address() {
    use neoteletext_web::url::Url;
    for (text, host, query, folder) in [
        ("https://a.example/news/302", "a.example", None, "https://a.example/news/"),
        ("https://a.example/news/302/0002?who=me", "a.example", Some("who=me"), "https://a.example/news/"),
        ("https://a.example/302", "a.example", None, "https://a.example/"),
        ("http://a.example:8080/news/?a=1&b=2", "a.example", Some("a=1&b=2"), "http://a.example:8080/news/"),
    ] {
        let ours = parse(text).unwrap();
        let theirs = Url::parse(text).unwrap();
        assert_eq!((ours.host.as_str(), theirs.host_text()), (host, host), "{text}");
        assert_eq!(theirs.query(), query, "{text}");
        assert_eq!(ours.channel_text(), folder, "{text}: what is fetched");
    }
    assert_eq!(Url::parse("https://evil.example#@good.example").unwrap().host_text(), "evil.example");
    assert!(parse("https://evil.example#@good.example").is_err(), "no place in a channel");
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
        let typed = format!("a.example/?q={}", "x".repeat(length - "a.example/?q=".len()));
        if let Ok(address) = parse(&typed) {
            assert!(address.text.len() <= max, "{length} typed, {} written", address.text.len());
            if let Some(paged) = address.with_page(0x302) {
                assert!(paged.text.len() <= max, "{length} typed, {} with a page", paged.text.len());
            }
        }
    }
    assert!(parse(&format!("https://a.example/?q={}", "x".repeat(max - "https://a.example/?q=".len()))).is_err(), "no room for a page");
}

/// REQ-1672 items 1 to 5: a link wraps a whole web address, and nothing else
/// opens.
#[test]
fn a_link_from_outside_unwraps_to_its_web_address() {
    for (link, address, page) in [
        ("neoteletext://https://news.example/302", "https://news.example/302", Some(0x302)),
        ("NEOTELETEXT://http://news.example/channel/?id=7", "http://news.example/channel/?id=7", None),
        ("neoteletext://https://news.example/1fe?a=1", "https://news.example/1FE?a=1", Some(0x1FE)),
        ("neoteletext://https://news.example/302.tti", "https://news.example/302", Some(0x302)),
        ("neoteletext://https://news.example/contents", "https://news.example/contents", None),
        ("neoteletext://https://news.example/wall", "https://news.example/wall", None),
        ("neoteletext://https://NEWS.Example:8443/", "https://news.example:8443/", None),
    ] {
        let opened = outside_link(link).unwrap();
        assert_eq!((opened.text.as_str(), opened.page), (address, page), "{link}");
    }
}

#[test]
fn a_link_from_outside_carries_nothing_else() {
    for link in [
        "neoteletext://home",
        "neoteletext://",
        "neoteletext:home",
        "neoteletext://file:///etc/passwd",
        "neoteletext://web:https://a.example/",
        "neoteletext://data:text/html,x",
        "neoteletext://javascript:alert(1)",
        "neoteletext://ftp://a.example/",
        "neoteletext://neoteletext://https://a.example/",
        "neoteletext://https://a.example/#not-a-page",
        "neoteletext://https://a.example/302:12",
        "neoteletext://https://a.example/channel.json",
        "neoteletext://https://a.example/302.tti#303",
        "neoteletext://https://a%2eexample/",
        "neoteletext://https://caf\u{e9}.example/",
        "neoteletext://https://user@a.example/",
        "neoteletext:// https://a.example/",
        "neoteletext://https://a.example/ ",
        "neoteletext://news.example/",
        "https://a.example/",
    ] {
        assert!(outside_link(link).is_err(), "{link}");
    }
    let long = format!("neoteletext://https://a.example/{}", "x".repeat(protocol::MAX_ADDRESS));
    assert!(outside_link(&long).is_err(), "longer than an address may be");
}

/// REQ-1701: a typed path is one by its form, and nothing is checked.
#[test]
fn a_typed_path_is_a_path_by_its_form_alone() {
    let home = Path::new(if cfg!(windows) { r"C:\\Users\\a" } else { "/home/a" });
    assert_eq!(typed_path("file:///x/y.t42", Some(home)).as_deref(), Some("/x/y.t42"));
    assert_eq!(typed_path("news.example", Some(home)), None);
    if !cfg!(windows) {
        assert_eq!(typed_path("/no/such/file.t42", Some(home)).as_deref(), Some("/no/such/file.t42"));
        assert_eq!(typed_path("~/x.t42", Some(home)).as_deref(), Some("/home/a/x.t42"));
        assert_eq!(typed_path("~", Some(home)).as_deref(), Some("/home/a"));
        assert_eq!(typed_path("~/x.t42", None), None, "no home folder known");
        assert_eq!(typed_path("~other/x.t42", Some(home)), None, "another user's is not named");
        assert!(parse("/no/such/file.t42").is_ok_and(|address| address.file.is_some()));
    }
}
