// Tests of `hyperlinks.md` sections 6 to 12 and requirements 17.3 to
// 17.12, and of the findings of `hyperlinks-security.md` section 13.

use super::destination::{Descriptor, INITIAL_PAGE};
use super::*;
use crate::address;
use protocol::links::{LinkKind, MAX_SERVICE_NAME, MAX_TARGET, PromptKind};

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
    assert_eq!(parse_target("302:0001"), Some(Target::Page(0x302)), "a sub-page");
    assert_eq!(parse_target("sport:302"), Some(Target::Service { name: "sport".into(), page: 0x302 }));
    assert_eq!(parse_target("sport:302:0001"), Some(Target::Service { name: "sport".into(), page: 0x302 }));
    assert_eq!(parse_target("abc:302"), Some(Target::Service { name: "abc".into(), page: 0x302 }), "a name is not a page");
    let Some(Target::Channel(address)) = parse_target("https://weather.invalid/300.t42") else { panic!() };
    assert_eq!(address.text, "https://weather.invalid/300.t42");
    assert_eq!(parse_target("web:https://www.example.invalid/story"), Some(Target::Web("https://www.example.invalid/story".into())));
}

#[test]
fn every_other_target_is_refused() {
    let long = format!("web:https://example.invalid/{}", "a".repeat(MAX_TARGET));
    for text in [
        "",
        "1A0",
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
    assert_eq!(decide("https://news.invalid/teletext/#401"), Some(Destination::Page(0x401)), "this channel at a page");
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

#[test]
fn leaving_and_the_web_are_asked_about_and_answered_only_after_half_a_second() {
    let mut links = links_with(vec![declared(5, 0, 7, "https://weather.invalid/#302"), declared(7, 0, 7, "web:http://www.example.invalid/")], Vec::new());
    links.point(Some(spot(1, 1, 5)));
    let Ok(Follow::Ask(leave)) = links.click(spot(1, 1, 5), Some(CHANNEL), LATER) else { panic!() };
    assert_eq!((leave.kind, leave.secure), (PromptKind::Leave, true));
    assert_eq!(leave.destination, "Page 302 of weather.invalid");
    assert_eq!(links.confirm(leave.id, LATER + PROMPT_DELAY_MILLISECONDS - 1), Err(Refusal::TooSoon));
    // A new question replaces the first, whose answer then confirms nothing.
    links.point(Some(spot(1, 1, 7)));
    let Ok(Follow::Ask(web)) = links.click(spot(1, 1, 7), Some(CHANNEL), LATER) else { panic!() };
    assert_eq!((web.kind, web.secure), (PromptKind::Web, false));
    assert_eq!(links.confirm(leave.id, LATER + PROMPT_DELAY_MILLISECONDS), Err(Refusal::NoPrompt));
    assert_eq!(links.confirm(web.id, LATER + PROMPT_DELAY_MILLISECONDS), Ok(Follow::HandOff("http://www.example.invalid/".into())));
    assert_eq!(links.prompt(), None, "answered");
    // Cancelling has no effect but to close the question.
    links.point(Some(spot(1, 1, 5)));
    let Ok(Follow::Ask(again)) = links.click(spot(1, 1, 5), Some(CHANNEL), LATER) else { panic!() };
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
    let Ok(Follow::Ask(prompt)) = links.typed(0x500, None, active, START) else { panic!() };
    assert_eq!(prompt.destination, "Page 302 of sport.invalid");
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
/// a service the descriptor lists, by its place, goes through the strip's
/// question; a key does nothing while row 24 was not shown.
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
    links.found(OPEN, Vec::new(), Vec::new(), Vec::new(), Fastext { keys, row24: true });
    let address = home();
    links.image(OPEN, 1, &Channel { id: CHANNEL, address: &address }, START);
    assert_eq!(links.fastext_keys(OPEN), [true, true, false, false, true], "the unknown seventh service leads nowhere");
    assert_eq!(links.fastext(FastextKey::Red, Some(CHANNEL), START), Ok((Follow::Page(0x200), Some(3))));
    let Ok((Follow::Ask(prompt), None)) = links.fastext(FastextKey::Green, Some(CHANNEL), START) else { panic!("another channel is asked about") };
    assert_eq!(prompt.kind, PromptKind::Leave);
    assert_eq!(links.fastext(FastextKey::Yellow, Some(CHANNEL), START), Err(Refusal::NotALink));
    assert_eq!(links.fastext(FastextKey::Red, Some(CHANNEL + 1), START), Err(Refusal::NotOpen), "only the active channel's open page");
    links.found(OPEN, Vec::new(), Vec::new(), Vec::new(), Fastext { keys, row24: false });
    links.image(OPEN, 2, &Channel { id: CHANNEL, address: &address }, START);
    assert_eq!(links.fastext_keys(OPEN), [false; 5]);
    assert_eq!(links.fastext(FastextKey::Red, Some(CHANNEL), START), Err(Refusal::NotALink), "row 24 hidden: no key acts");
}

/// Finding F-86: a channel owns a page file only inside its own folder,
/// after its `/`, so a channel opened at `/a` does not own `/a302.tti`.
/// The review thought it a false positive; it held for a channel address
/// with no `/` at its end.
#[test]
fn a_channel_owns_page_files_only_in_its_folder() {
    let folder = address::parse("https://news.invalid/a/").unwrap();
    assert!(folder.channel_text().ends_with('/'));
    let bare = address::parse("https://news.invalid/a").unwrap();
    let sibling = address::parse("https://news.invalid/a302.tti").unwrap();
    let inside = address::parse("https://news.invalid/a/302.tti").unwrap();
    let owned =
        |channel: &Address, target: &Address| Descriptor::default().resolve(&Target::Channel(target.clone()), &Channel { id: CHANNEL, address: channel });
    assert_eq!(owned(&folder, &inside), Some(Destination::Page(0x302)));
    assert_eq!(owned(&folder, &sibling), Some(Destination::Channel(sibling.clone())), "a sibling of the folder");
    assert_eq!(owned(&bare, &sibling), Some(Destination::Channel(sibling.clone())), "a name that only begins as the channel's");
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

/// REQ-INT-118 and INPUT.md section 7.3: the decoder's controls are
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
