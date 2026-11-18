// Tests of `hyperlinks.md` sections 6 to 12 and requirements 17.3 to
// 17.12, and of the findings of `hyperlinks-security.md` section 13.

use super::destination::{Descriptor, INITIAL_PAGE};
use super::*;
use crate::address;
use protocol::links::{LinkKind, MAX_SERVICE_NAME, MAX_TARGET};

const CHANNEL: u32 = 7;
const OPEN: PageKey = PageKey { channel: CHANNEL, page: 0x100 };
const START: u64 = 10_000;
const LATER: u64 = START + DWELL_MILLISECONDS;

fn home() -> Address {
    address::parse("https://news.invalid/teletext/").unwrap()
}

fn declared(row: u8, column: u8, length: u8, target: &str) -> Candidate {
    Candidate { row, column, length, double: false, light: false, target: target.into() }
}

fn spot(version: u32, column: u8, row: u8) -> Spot {
    Spot { key: OPEN, version, column, row }
}

/// Links with page 100 open and the window focused, whose image 1 has
/// `candidates`, decided at `START`.
fn links_with(candidates: Vec<Candidate>, numbers: Vec<NumberLink>) -> Links {
    let mut links = Links::default();
    links.opened(Some(OPEN));
    links.found(OPEN, candidates, numbers, Vec::new(), Fastext::default());
    let address = home();
    links.image(OPEN, 1, &Channel { id: CHANNEL, address: &address }, START);
    links
}

fn runs(links: &Links) -> Vec<(u8, u8, LinkKind)> {
    links.page_links(OPEN, CHANNEL).runs.iter().map(|run| (run.row, run.column, run.kind)).collect()
}

#[test]
fn targets_parse_in_every_form_of_the_tti_link_extension() {
    assert_eq!(parse_target("302"), Some(Target::Page(0x302)));
    assert_eq!(parse_target("302:0001"), Some(Target::SubPage(0x302, 1)), "a sub-page");
    assert_eq!(parse_target("302:3F7F"), Some(Target::Page(0x302)), "3F7F names none");
    assert_eq!(parse_target("sport:302"), Some(Target::Service { name: "sport".into(), page: 0x302 }));
    assert_eq!(parse_target("sport:302:0001"), Some(Target::Service { name: "sport".into(), page: 0x302 }));
    assert_eq!(parse_target("abc:302"), Some(Target::Service { name: "abc".into(), page: 0x302 }), "a name is not a page");
    let Some(Target::Channel(address)) = parse_target("https://weather.invalid/300.t42") else { panic!() };
    assert_eq!(address.text, "https://weather.invalid/300", "a page's file name has one spelling (REQ-1693 item 3)");
    assert_eq!(parse_target("web:https://www.example.invalid/story"), Some(Target::Web("https://www.example.invalid/story".into())));
}

/// REQ-1672 item 13: the link scheme is for what opens the browser from
/// outside. In a page it is no link, so a page cannot launder a navigation
/// through it.
#[test]
fn a_page_cannot_link_through_the_scheme() {
    let link = address::OUTSIDE_SCHEME;
    let capitals = link.to_ascii_uppercase();
    for target in
        [format!("{link}https://a.invalid/"), format!("{capitals}https://a.invalid/"), format!("{link}home"), "neoteletext://https://a.invalid/".to_owned()]
    {
        assert_eq!(parse_target(&target), None, "{target}");
    }
}

/// REQ-300, REQ-313: a page with a hexadecimal digit is reached by a link,
/// so a target names one, of this channel or of a service; a number the
/// standard reserves (Annex A.10) is none.
#[test]
fn a_target_names_a_hexadecimal_page() {
    assert_eq!(parse_target("1A0"), Some(Target::Page(0x1A0)));
    assert_eq!(parse_target("8fc"), Some(Target::Page(0x8FC)), "either case");
    assert_eq!(parse_target("1A0:0002"), Some(Target::SubPage(0x1A0, 2)));
    assert_eq!(parse_target("sport:2B5"), Some(Target::Service { name: "sport".into(), page: 0x2B5 }));
}

#[test]
fn every_other_target_is_refused() {
    let long = format!("web:https://example.invalid/{}", "a".repeat(MAX_TARGET));
    for text in [
        "",
        "1FF",
        "1FE",
        "2FD",
        "1F0",
        "1BE",
        "30",
        "3022",
        "902",
        "302:1",
        "a:b:c:d",
        "sport team:302",
        "spo,rt:302",
        "302 ",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,x",
        "ms-settings:display",
        "steam://run/1",
        "web:file:///etc/passwd",
        "web:javascript:alert(1)",
        "web:ftp://example.invalid/",
        "web:https://bank.invalid@evil.invalid/",
        "web:https://example.invalid/a\\b",
        "web:https://exa%6Dple.invalid/",
        "web:https://example.invalid/%zz",
        "web:https://example.invalid/%4",
        "web:https://example.invalid/<script>",
        "web:https://example.invalid:99999/",
        "web:https:///nohost",
        "web:-https://example.invalid/",
        "web:https://bücher.invalid/",
        "https://example.invalid@evil.invalid/",
        long.as_str(),
    ] {
        assert_eq!(parse_target(text), None, "{text:?} must be refused");
    }
    let name = format!("{}:302", "s".repeat(MAX_SERVICE_NAME + 1));
    assert_eq!(parse_target(&name), None, "a name over 32 bytes");
}

#[test]
fn a_web_address_is_rebuilt_from_its_parts() {
    assert_eq!(web_address("HTTPS://WWW.Example.Invalid").as_deref(), Some("https://www.example.invalid/"));
    assert_eq!(web_address("http://example.invalid:8080?q=1#top").as_deref(), Some("http://example.invalid:8080/?q=1#top"));
    assert_eq!(web_address("https://example.invalid/a%20b/c?x=1&y=2").as_deref(), Some("https://example.invalid/a%20b/c?x=1&y=2"));
    for text in ["https://example.invalid/ x", "https://example.invalid/\"x", "https://example.invalid/é", "https://example.invalid:/", "https://.invalid/"] {
        assert_eq!(web_address(text), None, "{text:?}");
    }
    // Rebuilt, it always starts with its scheme, so no opener can read it
    // as an option.
    assert!(web_address("https://-example.invalid/").is_some_and(|address| address.starts_with("https://")));
}

#[test]
fn targets_are_classified_from_the_target_alone() {
    let address = home();
    let channel = Channel { id: CHANNEL, address: &address };
    let descriptor = Descriptor::new(Vec::new(), vec![("sport".into(), address::parse("https://sport.invalid/").unwrap())], Vec::new());
    let decide = |text: &str| descriptor.resolve(&parse_target(text).unwrap(), &channel);
    assert_eq!(decide("302"), Some(Destination::Page(0x302)));
    assert_eq!(decide("https://news.invalid/teletext/302.tti"), Some(Destination::Page(0x302)), "a page file of this channel");
    assert_eq!(decide("https://news.invalid/teletext/302.t42"), Some(Destination::Page(0x302)));
    assert_eq!(decide("302:0002"), Some(Destination::SubPage(0x302, 2)));
    assert_eq!(decide("https://news.invalid/teletext/401/0002"), Some(Destination::SubPage(0x401, 2)), "this channel's");
    assert_eq!(decide("https://news.invalid/teletext/401"), Some(Destination::Page(0x401)), "this channel at a page");
    assert_eq!(decide("https://news.invalid/teletext/"), Some(Destination::Page(INITIAL_PAGE)));
    let Some(Destination::Channel(other)) = decide("https://news.invalid/other/302.tti") else { panic!() };
    assert_eq!(other.host, "news.invalid", "another folder of the same host is another channel");
    // A named service is another channel at its own address
    // (`hyperlinks-security.md` finding 1), and an unknown one leads nowhere.
    let Some(Destination::Channel(sport)) = decide("sport:302") else { panic!() };
    assert_eq!((sport.channel_text(), sport.page), ("https://sport.invalid/", Some(0x302)));
    assert_eq!(decide("weather:302"), None);
    assert_eq!(decide("web:https://www.example.invalid/").map(|destination| destination.kind()), Some(LinkKind::Web));
}

#[test]
fn a_file_channels_links_to_the_web_leave_it() {
    // An absolute path on this platform.
    let file = address::parse(if cfg!(windows) { r"C:\teletext\recording.t42" } else { "/teletext/recording.t42" }).unwrap();
    let channel = Channel { id: CHANNEL, address: &file };
    let descriptor = Descriptor::default();
    assert_eq!(descriptor.resolve(&Target::Page(0x302), &channel), Some(Destination::Page(0x302)));
    let target = parse_target("https://news.invalid/teletext/302.tti").unwrap();
    assert!(matches!(descriptor.resolve(&target, &channel), Some(Destination::Channel(_))));
}

#[test]
fn links_are_checked_and_decided() {
    let links = links_with(
        vec![
            declared(0, 10, 3, "302"),
            declared(24, 10, 3, "302"),
            declared(5, 10, 3, "302"),
            declared(7, 0, 12, "web:https://www.example.invalid/"),
            declared(9, 0, 5, "nonsense target"),
            declared(11, 4, 10, "https://weather.invalid/"),
            // A run inside an earlier one: the earlier wins.
            declared(11, 8, 3, "303"),
        ],
        Vec::new(),
    );
    assert_eq!(runs(&links), [(5, 10, LinkKind::Page), (7, 0, LinkKind::Web), (11, 4, LinkKind::Channel)]);
}

#[test]
fn a_pages_number_links_and_the_channels_aliases_remap_typed_numbers_and_make_no_runs() {
    let mut links = Links::default();
    links.opened(Some(OPEN));
    links.descriptor(
        CHANNEL,
        &[Alias { page: 0x500, target: "https://weather.invalid/".into() }, Alias { page: 0x501, target: "web:https://www.example.invalid/".into() }],
        &[],
    );
    links.found(
        OPEN,
        Vec::new(),
        vec![
            NumberLink { number: 0x302, target: "https://sport.invalid/".into() },
            NumberLink { number: 0x303, target: "web:https://www.example.invalid/".into() },
        ],
        Vec::new(),
        Fastext::default(),
    );
    let address = home();
    let page = links.image(OPEN, 1, &Channel { id: CHANNEL, address: &address }, START);
    assert!(page.runs.is_empty(), "an LK line changes what typing the number does; it draws no link (decision 116)");
    assert_eq!(page.remapped, [0x302, 0x500], "302 follows its LK line, 500 its alias; a web page is never the target of a typed number");
}

#[test]
fn a_click_follows_only_the_link_the_strip_shows_once_it_has_settled() {
    let mut links = links_with(vec![declared(5, 10, 3, "302"), declared(7, 0, 4, "303")], Vec::new());
    let active = Some(CHANNEL);
    assert_eq!(links.click(spot(1, 11, 5), active, LATER), Err(Refusal::Unshown), "the strip shows nothing yet");
    let shown = links.point(Some(spot(1, 11, 5))).unwrap();
    assert_eq!((shown.kind, shown.text.as_str()), (LinkKind::Page, "Page 302"));
    assert_eq!(links.click(spot(1, 1, 7), active, LATER), Err(Refusal::Unshown), "the strip shows another link");
    assert_eq!(links.click(spot(1, 11, 5), active, LATER - 1), Err(Refusal::Changed), "within half a second of appearing");
    assert_eq!(links.click(spot(1, 11, 5), active, LATER), Ok(Follow::Page(0x302)));
    assert_eq!(links.click(spot(2, 11, 5), active, LATER), Err(Refusal::Stale), "an image never sent");
    assert_eq!(links.click(spot(1, 20, 5), active, LATER), Err(Refusal::NotALink));
    assert_eq!(links.click(spot(1, 11, 5), Some(CHANNEL + 1), LATER), Err(Refusal::NotOpen), "not the active channel");
    links.opened(None);
    assert_eq!(links.click(spot(1, 11, 5), active, LATER), Err(Refusal::NotOpen), "no page open");
}

#[test]
fn a_link_that_changes_under_the_pointer_is_not_followed() {
    // Bait and switch (`hyperlinks-security.md` section 8.1): the page
    // changes the link's target without changing its image.
    let mut links = links_with(vec![declared(5, 10, 3, "302")], Vec::new());
    links.point(Some(spot(1, 11, 5)));
    links.found(OPEN, vec![declared(5, 10, 3, "web:https://evil.invalid/")], Vec::new(), Vec::new(), Fastext::default());
    let address = home();
    let switched = START + 2 * DWELL_MILLISECONDS;
    let changed = links.redecide(OPEN, &Channel { id: CHANNEL, address: &address }, switched);
    assert_eq!(changed.map(|page| page.runs[0].kind), Some(LinkKind::Web));
    // The strip follows the change at once.
    let repointed = links.repoint().expect("the destination changed");
    assert_eq!(repointed.map(|pointing| pointing.kind), Some(LinkKind::Web));
    assert_eq!(links.click(spot(1, 11, 5), Some(CHANNEL), switched + DWELL_MILLISECONDS - 1), Err(Refusal::Changed));
    assert!(matches!(links.click(spot(1, 11, 5), Some(CHANNEL), switched + DWELL_MILLISECONDS), Ok(Follow::Ask(_))));
}

#[test]
fn an_unchanged_link_keeps_its_age_across_new_images() {
    let mut links = links_with(vec![declared(5, 10, 3, "302")], Vec::new());
    let address = home();
    // A flash redraws the page: a new image, the same link.
    links.image(OPEN, 2, &Channel { id: CHANNEL, address: &address }, LATER - 1);
    links.point(Some(spot(2, 11, 5)));
    assert_eq!(links.click(spot(2, 11, 5), Some(CHANNEL), LATER), Ok(Follow::Page(0x302)));
    // The first image is still known; a fifth image pushes it out.
    for version in 3..=5 {
        links.image(OPEN, version, &Channel { id: CHANNEL, address: &address }, LATER);
    }
    assert_eq!(links.click(spot(1, 11, 5), Some(CHANNEL), LATER), Err(Refusal::Stale));
}

// REQ-1670: another channel is followed with no question, whatever the
// window's focus, since only a hand-off leaves the browser.
#[test]
fn a_link_to_another_channel_is_followed_with_no_question() {
    let mut links = links_with(vec![declared(5, 0, 7, "https://weather.invalid/302")], Vec::new());
    links.point(Some(spot(1, 1, 5)));
    links.focused(false);
    let Ok(Follow::Channel(address)) = links.click(spot(1, 1, 5), Some(CHANNEL), LATER) else { panic!() };
    assert_eq!((address.channel_text(), address.page), ("https://weather.invalid/", Some(0x302)));
    assert_eq!(links.prompt(), None);
}

// REQ-1660 item 3: the popover's text, for each kind of link: complete, and
// only for the link pointed at.
#[test]
fn a_pointed_link_says_where_it_leads_in_full() {
    let long = format!("https://weather.invalid/?q={}", "a".repeat(protocol::MAX_ADDRESS - 40));
    let mut links =
        links_with(vec![declared(5, 0, 7, "sport:302"), declared(7, 0, 7, "web:https://www.example.invalid/a?b=c#d"), declared(9, 0, 7, "401")], Vec::new());
    links.descriptor(CHANNEL, &[], &[Service { name: "sport".into(), address: long.clone() }]);
    let address = home();
    links.redecide(OPEN, &Channel { id: CHANNEL, address: &address }, START);
    let complete = |links: &mut Links, row| links.point(Some(spot(1, 1, row))).map(|pointing| pointing.complete);
    assert_eq!(complete(&mut links, 5), Some("https://weather.invalid/302".to_owned()), "another channel's whole address");
    assert_eq!(complete(&mut links, 7), Some("https://www.example.invalid/a?b=c#d (opens in your web browser)".to_owned()));
    assert_eq!(complete(&mut links, 9), Some("Page 401".to_owned()));
    assert_eq!(complete(&mut links, 20), None, "nothing where no link is");
}

// REQ-1660 item 3: a link within the channel shows its sub-page when it has
// one, and follows it.
#[test]
fn a_link_to_a_sub_page_says_so_and_goes_there() {
    let mut links = links_with(vec![declared(5, 0, 7, "401:0002")], Vec::new());
    let pointing = links.point(Some(spot(1, 1, 5))).unwrap();
    assert_eq!(pointing.complete, "Page 401, sub-page 0002");
    assert_eq!(links.click(spot(1, 1, 5), Some(CHANNEL), LATER), Ok(Follow::SubPage(0x401, 2)));
}

// REQ-1660 item 1: a coloured key's prompt on row 24 is a link to where the
// key leads, with no prompt while row 24 is not shown; a click follows it.
#[test]
fn a_keys_prompt_says_where_it_leads() {
    use protocol::links::FastextTarget;
    let keys = [Some(FastextTarget { page: 0x200, subcode: Some(3), service: None }), None, None, None, None];
    let prompts = [Some((2, 8)), None, Some((12, 8)), None];
    let address = home();
    let channel = Channel { id: CHANNEL, address: &address };
    let mut links = Links::default();
    links.opened(Some(OPEN));
    links.found(OPEN, Vec::new(), Vec::new(), Vec::new(), Fastext { keys, row24: true, linked: true, prompts });
    let runs = links.image(OPEN, 1, &channel, START).runs;
    assert_eq!(runs.iter().map(|run| (run.row, run.column, run.length)).collect::<Vec<_>>(), [(24, 2, 8)], "the yellow key leads nowhere");
    let pointing = links.point(Some(spot(1, 3, 24))).unwrap();
    assert_eq!(pointing.complete, "Page 200, sub-page 0003");
    assert_eq!(links.click(spot(1, 3, 24), Some(CHANNEL), LATER), Ok(Follow::SubPage(0x200, 3)));
    links.found(OPEN, Vec::new(), Vec::new(), Vec::new(), Fastext { keys, row24: false, linked: true, prompts });
    assert!(links.image(OPEN, 2, &channel, LATER).runs.is_empty(), "a boxed page's row 24 acts on nothing");
}

/// A page with all eight of its text links and all four of its keys'
/// prompts keeps every one. The prompts used to be counted against the
/// text links, so a front page of eight channels with four prompts had
/// only its first four channels linked (found on the NeoTeletext channel's
/// front page, 16 November 2026).
#[test]
fn a_page_keeps_its_eight_text_links_beside_its_four_prompts() {
    use protocol::links::{FastextTarget, MAX_PROMPT_RUNS, MAX_TEXT_LINKS};
    let candidates: Vec<Candidate> = (0..8_u8).map(|place| declared(5 + place * 2, 1, 8, &format!("https://example.invalid/channel{place}/"))).collect();
    let page = |page| Some(FastextTarget { page, subcode: None, service: None });
    let keys = [page(0x200), page(0x300), page(0x400), page(0x500), None];
    let prompts = [Some((1, 8)), Some((11, 8)), Some((21, 8)), Some((31, 8))];
    let address = home();
    let mut links = Links::default();
    links.opened(Some(OPEN));
    links.found(OPEN, candidates, Vec::new(), Vec::new(), Fastext { keys, row24: true, linked: true, prompts });
    let runs = links.image(OPEN, 1, &Channel { id: CHANNEL, address: &address }, START).runs;
    let rows: Vec<u8> = runs.iter().map(|run| run.row).collect();
    assert_eq!(rows, [5, 7, 9, 11, 13, 15, 17, 19, 24, 24, 24, 24], "eight text links, then the four prompts");
    assert_eq!(runs.len(), MAX_TEXT_LINKS + MAX_PROMPT_RUNS);
    // The last text link leads where it says, as the first does.
    assert!(links.point(Some(spot(1, 3, 19))).is_some(), "the pointer over the eighth link");
    let Ok(Follow::Channel(last)) = links.click(spot(1, 3, 19), Some(CHANNEL), LATER) else { panic!("the eighth link is followed") };
    assert_eq!(last.channel_text(), "https://example.invalid/channel7/");
}

// REQ-1660 item 4: a character that would disguise an address cannot be in
// a link's address at all, so none reaches the popover.
#[test]
fn an_address_with_hidden_characters_is_not_a_link() {
    for target in
        ["https://weather.invalid/\u{202E}a", "https://weather.invalid/\u{200B}", "https://weather.invalid/\u{7}", "web:https://www.example.invalid/\u{202E}"]
    {
        assert_eq!(parse_target(target), None, "{target:?}");
    }
}

// REQ-808: only a `web:` link asks.
#[test]
fn the_web_is_asked_about_and_answered_only_after_half_a_second() {
    let mut links = links_with(vec![declared(7, 0, 7, "web:http://www.example.invalid/"), declared(9, 0, 7, "web:https://www.example.invalid/")], Vec::new());
    links.point(Some(spot(1, 1, 7)));
    let Ok(Follow::Ask(first)) = links.click(spot(1, 1, 7), Some(CHANNEL), LATER) else { panic!() };
    assert!(!first.secure);
    assert_eq!(first.destination, "http://www.example.invalid/");
    assert_eq!(links.confirm(first.id, LATER + PROMPT_DELAY_MILLISECONDS - 1), Err(Refusal::TooSoon));
    // A new question replaces the first, whose answer then confirms nothing.
    links.point(Some(spot(1, 1, 9)));
    let Ok(Follow::Ask(web)) = links.click(spot(1, 1, 9), Some(CHANNEL), LATER) else { panic!() };
    assert!(web.secure);
    assert_eq!(links.confirm(first.id, LATER + PROMPT_DELAY_MILLISECONDS), Err(Refusal::NoPrompt));
    assert_eq!(links.confirm(web.id, LATER + PROMPT_DELAY_MILLISECONDS), Ok(Follow::HandOff("https://www.example.invalid/".into())));
    assert_eq!(links.prompt(), None, "answered");
    // Cancelling has no effect but to close the question.
    links.point(Some(spot(1, 1, 7)));
    let Ok(Follow::Ask(again)) = links.click(spot(1, 1, 7), Some(CHANNEL), LATER) else { panic!() };
    assert!(!links.cancel(again.id + 1));
    assert!(links.cancel(again.id));
    assert_eq!(links.confirm(again.id, LATER + PROMPT_DELAY_MILLISECONDS), Err(Refusal::NoPrompt));
}

#[test]
fn the_web_is_handed_over_only_with_the_focus_and_at_most_every_two_seconds() {
    let mut links = links_with(vec![declared(7, 0, 7, "web:https://www.example.invalid/")], Vec::new());
    links.point(Some(spot(1, 1, 7)));
    links.focused(false);
    assert_eq!(links.click(spot(1, 1, 7), Some(CHANNEL), LATER), Err(Refusal::Unfocused));
    links.focused(true);
    let Ok(Follow::Ask(first)) = links.click(spot(1, 1, 7), Some(CHANNEL), LATER) else { panic!() };
    let answered = LATER + PROMPT_DELAY_MILLISECONDS;
    links.focused(false);
    assert_eq!(links.confirm(first.id, answered), Err(Refusal::Unfocused));
    links.focused(true);
    assert!(matches!(links.confirm(first.id, answered), Ok(Follow::HandOff(_))));
    let Ok(Follow::Ask(second)) = links.click(spot(1, 1, 7), Some(CHANNEL), answered) else { panic!() };
    let soon = answered + PROMPT_DELAY_MILLISECONDS;
    assert_eq!(links.confirm(second.id, soon), Err(Refusal::TooOften));
    assert!(matches!(links.confirm(second.id, answered + HANDOFF_INTERVAL_MILLISECONDS), Ok(Follow::HandOff(_))));
}

#[test]
fn a_typed_number_follows_the_pages_link_or_the_channels_alias() {
    let mut links = Links::default();
    links.opened(Some(OPEN));
    links.descriptor(
        CHANNEL,
        &[Alias { page: 0x500, target: "sport:302".into() }],
        &[Service { name: "sport".into(), address: "https://sport.invalid/".into() }],
    );
    links.found(OPEN, Vec::new(), vec![NumberLink { number: 0x302, target: "401".into() }], Vec::new(), Fastext::default());
    let address = home();
    links.image(OPEN, 1, &Channel { id: CHANNEL, address: &address }, START);
    let active = Some(CHANNEL);
    assert_eq!(links.typed(0x302, Some((OPEN, 1)), active, START), Ok(Follow::Page(0x401)));
    let Ok(Follow::Channel(sport)) = links.typed(0x500, None, active, START) else { panic!() };
    assert_eq!((sport.channel_text(), sport.page), ("https://sport.invalid/", Some(0x302)));
    assert_eq!(links.typed(0x302, None, active, START), Err(Refusal::NotALink), "an LK line counts only while its page is open");
    assert_eq!(links.typed(0x302, Some((OPEN, 9)), active, START), Err(Refusal::Stale));
    assert_eq!(links.typed(0x302, Some((PageKey { page: 0x101, ..OPEN }, 1)), active, START), Err(Refusal::NotOpen));
}

#[test]
fn a_descriptor_alias_to_the_web_is_dropped() {
    let mut links = Links::default();
    links.descriptor(CHANNEL, &[Alias { page: 0x501, target: "web:https://www.example.invalid/".into() }], &[]);
    assert_eq!(links.aliases(CHANNEL), [] as [u16; 0]);
}

#[test]
fn a_channel_that_closes_takes_its_links() {
    let mut links = links_with(vec![declared(5, 10, 3, "302")], Vec::new());
    links.channel_gone(CHANNEL);
    assert_eq!(links.pages_of(CHANNEL), [] as [PageKey; 0]);
}

#[test]
fn nothing_but_a_confirmation_hands_a_web_page_over() {
    // `hyperlinks-security.md` section 9.4: every other way in, driven with
    // web links present, never reaches the system's web browser.
    let web = "web:https://www.example.invalid/";
    let mut links = links_with(vec![declared(7, 0, 7, web), declared(5, 10, 3, "302")], vec![NumberLink { number: 0x302, target: web.into() }]);
    let address = home();
    let channel = Channel { id: CHANNEL, address: &address };
    let mut seen = Vec::new();
    links.descriptor(CHANNEL, &[Alias { page: 0x501, target: web.into() }], &[]);
    links.found(OPEN, vec![declared(7, 0, 7, web)], Vec::new(), Vec::new(), Fastext::default());
    seen.push(links.image(OPEN, 2, &channel, LATER).runs.len());
    links.redecide(OPEN, &channel, LATER);
    links.point(Some(spot(2, 1, 7)));
    links.repoint();
    let mut answers = vec![
        links.typed(0x501, None, Some(CHANNEL), LATER * 2),
        links.typed(0x302, Some((OPEN, 2)), Some(CHANNEL), LATER * 2),
        links.click(spot(2, 1, 7), Some(CHANNEL), LATER * 2),
    ];
    let Some(Ok(Follow::Ask(prompt))) = answers.last().cloned() else { panic!("{answers:?}") };
    links.focused(false);
    links.focused(true);
    answers.push(links.confirm(prompt.id.wrapping_add(1), LATER * 3));
    answers.push(links.confirm(prompt.id, LATER * 2));
    assert!(answers.iter().all(|answer| !matches!(answer, Ok(Follow::HandOff(_)))), "{answers:?}");
    assert!(links.cancel(prompt.id));
    assert!(matches!(links.confirm(prompt.id, LATER * 3), Err(Refusal::NoPrompt)), "a declined question confirms nothing");
    assert_eq!(seen, [1]);
}

/// Decision 67, the engine's REQ-NAV-003 and REQ-SEC-024: a coloured key
/// follows its page of this channel at once, with the sub-page it names;
/// a service the descriptor lists, by its place, is followed at once
/// (REQ-1670); a key does nothing while row 24 was not shown.
#[test]
fn fastext_keys_follow_as_links_do() {
    use protocol::links::FastextTarget;
    let target = |page, subcode, service| Some(FastextTarget { page, subcode, service });
    let keys = [target(0x200, Some(3), None), target(0x300, None, Some(1)), None, target(0x400, None, Some(7)), target(0x100, None, None)];
    let mut links = Links::default();
    links.opened(Some(OPEN));
    links.descriptor(
        CHANNEL,
        &[],
        &[Service { name: "news".into(), address: "https://news.invalid/".into() }, Service { name: "sport".into(), address: "https://sport.invalid/".into() }],
    );
    links.found(OPEN, Vec::new(), Vec::new(), Vec::new(), Fastext { keys, row24: true, linked: true, prompts: [None; 4] });
    let address = home();
    links.image(OPEN, 1, &Channel { id: CHANNEL, address: &address }, START);
    assert_eq!(links.fastext_keys(OPEN), [true, true, false, false, true], "the unknown seventh service leads nowhere");
    assert_eq!(links.fastext(FastextKey::Red, Some(CHANNEL), START), Ok((Follow::Page(0x200), Some(3))));
    let Ok((Follow::Channel(sport), None)) = links.fastext(FastextKey::Green, Some(CHANNEL), START) else { panic!("another channel, at once") };
    assert_eq!((sport.channel_text(), sport.page), ("https://sport.invalid/", Some(0x300)));
    assert_eq!(links.fastext(FastextKey::Yellow, Some(CHANNEL), START), Err(Refusal::NotALink));
    assert_eq!(links.fastext(FastextKey::Red, Some(CHANNEL + 1), START), Err(Refusal::NotOpen), "only the active channel's open page");
    links.found(OPEN, Vec::new(), Vec::new(), Vec::new(), Fastext { keys, row24: false, linked: true, prompts: [None; 4] });
    links.image(OPEN, 2, &Channel { id: CHANNEL, address: &address }, START);
    assert_eq!(links.fastext_keys(OPEN), [false; 5]);
    assert_eq!(links.fastext(FastextKey::Red, Some(CHANNEL), START), Err(Refusal::NotALink), "row 24 hidden: no key acts");
}

/// Finding F-86, and REQ-1693 item 3: a page's file name reads as the page,
/// in the channel whose folder holds it, and a name that only begins as a
/// channel's is no address at all. An address a page alone cannot say is
/// not followed as a page: it opens as a navigation (REQ-1693 item 6).
#[test]
fn a_channel_owns_a_page_only_in_its_folder() {
    let folder = address::parse("https://news.invalid/a/").unwrap();
    assert!(folder.channel_text().ends_with('/'));
    assert!(address::parse("https://news.invalid/a").is_err());
    assert!(address::parse("https://news.invalid/a302.tti").is_err());
    let inside = address::parse("https://news.invalid/a/302.tti").unwrap();
    let other = address::parse("https://news.invalid/302.tti").unwrap();
    let owned =
        |channel: &Address, target: &Address| Descriptor::default().resolve(&Target::Channel(target.clone()), &Channel { id: CHANNEL, address: channel });
    assert_eq!(owned(&folder, &inside), Some(Destination::Page(0x302)));
    assert_eq!(owned(&folder, &other), Some(Destination::Channel(other.clone())), "a page of another folder");
    for target in ["https://news.invalid/a/302?b=1", "https://news.invalid/a/wall", "https://news.invalid/a/contents"] {
        let target = address::parse(target).unwrap();
        assert_eq!(owned(&folder, &target), Some(Destination::Channel(target.clone())));
    }
}

/// Finding F-41 (decision 100), the core's side: a web address holding a
/// character a shell gives meaning to is refused here, as the web opener
/// refuses it, so every web target the core parses is one the opener
/// accepts.
#[test]
fn a_web_address_the_opener_would_refuse_is_not_a_target() {
    for address in ["https://a.example/x;$(id)", "https://a.example/it's", "https://a.example/(x)", "https://a.example/hey!", "https://a.example/a;b"] {
        assert_eq!(web_address(address), None, "{address}");
        assert_eq!(parse_target(&format!("web:{address}")), None, "{address}");
    }
    let plain = "https://a.example/news/story?id=3&page=2#top";
    assert_eq!(web_address(plain).as_deref(), Some(plain));
    assert!(web_opener::acceptable(plain));
}

fn area(row: u8, column: u8, width: u8, height: u8) -> Area {
    Area { row, column, width, height }
}

/// Links with page 100 open, whose image 1 has these controls and one
/// link to page 302 on row 10.
fn controls_with(controls: Vec<Area>) -> Links {
    let mut links = Links::default();
    links.opened(Some(OPEN));
    links.found(OPEN, vec![declared(10, 0, 5, "302")], Vec::new(), controls, Fastext::default());
    let address = home();
    links.image(OPEN, 1, &Channel { id: CHANNEL, address: &address }, START);
    links
}

/// REQ-INT-118 and DOC-1844: the decoder's controls are
/// checked again: rows 1 to 23, the 40 columns, a cell at least, on no
/// link's cell or an earlier control's, at most `MAX_CONTROLS`.
#[test]
fn controls_are_checked_again() {
    let found = vec![
        area(20, 1, 9, 1),
        area(20, 10, 9, 1),
        area(5, 2, 30, 3),
        area(20, 5, 9, 1), // over the first
        area(10, 3, 5, 1), // over the link on row 10
        area(0, 0, 5, 1),  // the header
        area(23, 0, 5, 2), // into the Fastext row
        area(4, 35, 6, 1), // past column 39
        area(6, 0, 0, 1),  // no width
    ];
    assert_eq!(controls_with(found).page_links(OPEN, CHANNEL).controls, [area(20, 1, 9, 1), area(20, 10, 9, 1), area(5, 2, 30, 3)]);
    let many: Vec<Area> = (0..40u8).map(|at| area(1 + at % 23, 2 * (at / 23), 1, 1)).collect();
    assert_eq!(controls_with(many).page_links(OPEN, CHANNEL).controls.len(), MAX_CONTROLS);
}

/// REQ-INT-119 and requirement 17.13: a look is drawn only on a control
/// or a link of the latest image of the open page; another page's, or
/// cells that are neither, are refused; moving to another page puts the
/// look out on the first.
#[test]
fn a_look_is_only_for_a_control_or_link_of_the_open_page() {
    let mut links = controls_with(vec![area(20, 1, 9, 1)]);
    let lit = |key, area, pressed| Some(Lit { key, area, pressed });
    assert_eq!(links.light(lit(OPEN, area(20, 1, 9, 1), false)), [(OPEN, Some((area(20, 1, 9, 1), false)))]);
    assert_eq!(links.light(lit(OPEN, area(20, 1, 9, 1), false)), [], "nothing new to draw");
    assert_eq!(links.light(lit(OPEN, area(20, 1, 9, 1), true)), [(OPEN, Some((area(20, 1, 9, 1), true)))]);
    assert_eq!(links.light(lit(OPEN, area(10, 0, 5, 1), false)), [(OPEN, Some((area(10, 0, 5, 1), false)))], "the link's cells");
    // Cells that are no control: the look goes out.
    assert_eq!(links.light(lit(OPEN, area(20, 1, 8, 1), false)), [(OPEN, None)]);
    assert_eq!(links.light(lit(OPEN, area(20, 1, 9, 1), false)).len(), 1);
    let other = PageKey { channel: CHANNEL, page: 0x200 };
    assert_eq!(links.light(lit(other, area(20, 1, 9, 1), false)), [(OPEN, None)], "a page not open");
    assert_eq!(links.light(lit(OPEN, area(20, 1, 9, 1), false)).len(), 1);
    links.opened(Some(other));
    assert_eq!(links.light(None), [(OPEN, None)], "the page closed");
}

/// REQ-INT-121: a HOVER may name only the first cell of a control of the
/// open page's latest image.
#[test]
fn a_hover_names_only_a_controls_first_cell() {
    let links = controls_with(vec![area(20, 1, 9, 1)]);
    assert!(links.is_control_at(OPEN, 1, 20));
    assert!(!links.is_control_at(OPEN, 2, 20), "inside it, not its first cell");
    assert!(!links.is_control_at(PageKey { channel: CHANNEL, page: 0x200 }, 1, 20), "not the open page");
}

/// The drawing links of page 100's sub-page 0, decided at `START`.
fn links_with_drawings(found: &[(u16, &str)]) -> Links {
    let mut links = Links::default();
    links.opened(Some(OPEN));
    let address = home();
    let found: Vec<(u16, String)> = found.iter().map(|(drawing, target)| (*drawing, (*target).to_owned())).collect();
    links.found_drawings(OPEN, 0, &found, &Channel { id: CHANNEL, address: &address }, START);
    links
}

/// REQ-6376: a drawing of a ReGIS picture that is a link shows where it
/// leads while the pointer is on it, and a click on it follows by the
/// rules of a click on a text link: only the destination the strip is
/// showing, only once it has led there half a second, only on the open
/// page of the active channel.
#[test]
fn a_click_on_a_linked_drawing_follows_as_a_text_link_does() {
    let mut links =
        links_with_drawings(&[(11, "200"), (12, "https://weather.invalid/teletext/300"), (13, "web:https://www.example.invalid/story"), (14, "not a target")]);
    let active = Some(CHANNEL);
    let on = |drawing: u16| (OPEN, 0, drawing);
    assert!(links.is_drawing_link(on(11)) && links.is_drawing_link(on(13)));
    assert!(!links.is_drawing_link(on(14)), "a target that does not parse is no link");
    assert!(!links.is_drawing_link((OPEN, 2, 11)), "another sub-page's picture has none of these");
    assert_eq!(links.click_drawing(on(11), active, LATER), Err(Refusal::Unshown), "the strip shows nothing yet");
    let shown = links.point_drawing(Some(on(11))).unwrap().unwrap();
    assert_eq!((shown.kind, shown.text.as_str()), (LinkKind::Page, "Page 200"));
    assert_eq!(links.point_drawing(Some(on(11))), None, "the same drawing again changes nothing");
    assert_eq!(links.click_drawing(on(12), active, LATER), Err(Refusal::Unshown), "the strip shows another drawing's destination");
    assert_eq!(links.click_drawing(on(11), active, LATER - 1), Err(Refusal::Changed), "within half a second of appearing");
    assert_eq!(links.click_drawing(on(11), None, LATER), Err(Refusal::NotOpen), "not the active channel");
    assert_eq!(links.click_drawing(on(11), active, LATER), Ok(Follow::Page(0x200)));
    // A drawing that is no link: the strip shows nothing, and a click follows nothing.
    assert_eq!(links.point_drawing(Some(on(9))), Some(None));
    assert_eq!(links.click_drawing(on(9), active, LATER), Err(Refusal::NotALink));
    // Another channel opens at once; a web page is asked about first.
    let _ = links.point_drawing(Some(on(12)));
    assert!(matches!(links.click_drawing(on(12), active, LATER), Ok(Follow::Channel(_))));
    let pointing = links.point_drawing(Some(on(13))).unwrap().unwrap();
    assert_eq!(pointing.kind, LinkKind::Web);
    assert!(matches!(links.click_drawing(on(13), active, LATER), Ok(Follow::Ask(_))), "the same question a text link to the web asks");
}

/// A sub-page's drawing links replace those it had; a drawing that led to
/// the same place keeps the time it began to, and one that now leads
/// elsewhere must settle again; the pointer off every drawing shows nothing.
#[test]
fn a_sub_pages_drawing_links_are_replaced_whole() {
    let mut links = links_with_drawings(&[(11, "200"), (12, "300")]);
    let address = home();
    let channel = Channel { id: CHANNEL, address: &address };
    let on = |drawing: u16| (OPEN, 0, drawing);
    links.found_drawings(OPEN, 0, &[(11, "200".to_owned()), (12, "400".to_owned())], &channel, LATER);
    let _ = links.point_drawing(Some(on(11)));
    assert_eq!(links.click_drawing(on(11), Some(CHANNEL), LATER), Ok(Follow::Page(0x200)), "it has led to page 200 since the start");
    let _ = links.point_drawing(Some(on(12)));
    assert_eq!(links.click_drawing(on(12), Some(CHANNEL), LATER), Err(Refusal::Changed), "it leads somewhere new");
    assert_eq!(links.click_drawing(on(12), Some(CHANNEL), LATER + DWELL_MILLISECONDS), Ok(Follow::Page(0x400)));
    links.found_drawings(OPEN, 0, &[], &channel, LATER);
    assert!(!links.is_drawing_link(on(11)), "an empty list removes them all");
    assert_eq!(links.point_drawing(None), Some(None));
    // More than a sub-page may have: the first 64 are kept.
    let many: Vec<(u16, String)> = (1..=70).map(|drawing| (drawing, "200".to_owned())).collect();
    links.found_drawings(OPEN, 0, &many, &channel, LATER);
    assert!(links.is_drawing_link(on(64)) && !links.is_drawing_link(on(65)));
    links.channel_gone(CHANNEL);
    assert!(!links.is_drawing_link(on(1)));
}

/// Renderer parity PAR-28 (REQ-6376 for NAPLPS objects): a `DRAWING_LINKS`
/// record for a sub-page drawn in NAPLPS makes the named objects links. The
/// object under the pointer comes from the NAPLPS pick record, of the
/// teletext raster; the hand shows over it though keys go to no channel,
/// the strip shows its destination, and a click on it follows only after
/// the dwell time. The path was already blind to the picture's language
/// (PAR-2 says a browser built before this work may already follow them);
/// this test holds it to that.
#[test]
fn a_click_on_a_linked_naplps_object_follows_after_the_dwell_time() {
    use crate::regis::PickRecords;
    use crate::regis_pointer::{Pointing, Say};
    use protocol::regis::{PictureLanguage, RegisPick, RegisPointer};

    let language = PictureLanguage::Naplps;
    let width = usize::from(language.width());
    // Object 5 over the raster's columns 100 to 149 of lines 200 to 249.
    let mut record = vec![0_u16; language.pixels()];
    for row in record.chunks_mut(width).skip(200).take(50) {
        row[100..150].fill(5);
    }
    let mut records = PickRecords::default();
    records.hold(OPEN, 0, 1, language, Some(RegisPick::of(&record, [(5, 100, 200)].into_iter(), language)));
    let mut links = links_with_drawings(&[(5, "200")]);
    let mut pointing = Pointing::default();
    pointing.pointer(RegisPointer { key: OPEN, at: Some((120, 220)), pressed: false, finger: false, span: 1 }, &records);
    let under = pointing.under(&records);
    assert_eq!(under, Some((OPEN, 0, 5)), "the object under the pointer, from the NAPLPS pick record");
    let linked = under.is_some_and(|under| links.is_drawing_link(under));
    assert!(linked);
    assert_eq!(pointing.decide(&records, None, linked), [Say::Over { key: OPEN, over: true }], "the hand, though keys go to no channel");
    let shown = links.point_drawing(under).unwrap().unwrap();
    assert_eq!((shown.kind, shown.text.as_str()), (LinkKind::Page, "Page 200"), "the strip shows its destination");
    // A press and a release on it.
    pointing.pointer(RegisPointer { key: OPEN, at: Some((120, 220)), pressed: true, finger: false, span: 1 }, &records);
    pointing.pointer(RegisPointer { key: OPEN, at: Some((121, 221)), pressed: false, finger: false, span: 1 }, &records);
    let (key, object) = pointing.take_click().unwrap();
    assert_eq!(links.click_drawing((key, 0, object), Some(CHANNEL), LATER - 1), Err(Refusal::Changed), "not within the dwell time");
    assert_eq!(links.click_drawing((key, 0, object), Some(CHANNEL), LATER), Ok(Follow::Page(0x200)));
}

/// Renderer parity PAR-28 and DEC-2385: hover, pressed looks and the
/// touch rule on a real touch screen, for a ReGIS drawing and a NAPLPS
/// object, cannot be checked unattended. Run by hand on a touch screen:
/// a finger on a linked drawing and on a linked object presses it, shows
/// its pressed look and no hover look, and a lift on it follows its link
/// after the dwell time. Record the result and the date in TODO-6778.
#[test]
#[ignore = "needs a real touch screen and a person: PAR-28's touch looks are checked by hand (DEVELOPMENT.md section 8)"]
fn touch_looks_on_regis_drawings_and_naplps_objects_are_checked_by_hand() {
    panic!("run by hand on a touch screen, as the comment says, and record the result in TODO-6778");
}
