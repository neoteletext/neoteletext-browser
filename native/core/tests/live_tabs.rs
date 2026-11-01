//! Running tabs on a live channel, on a real socket: the real fetcher
//! against the reference channel, `neoteletext-site terminal`, whose
//! standard error says what each connection selected and when it ended
//! (TODO-1682, REQ-714, DEC-1527).
#![cfg(all(feature = "interaction", feature = "links"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "a test")]

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use interface_core::canvas::PageKey;
use neoteletext::links::{Channel, DWELL_MILLISECONDS, Follow, Links};
use neoteletext_core::limits::SUBCODE_ANY;
use neoteletext_input::event::{Event, Item, Select};
use neoteletext_input::session::Session;
use protocol::links::{Candidate, Spot};

mod reference_channel;
use reference_channel::{Site, Tab, alone, message, programs, said_until, send};

/// How long anything may take before the test gives up.
const LIMIT: Duration = Duration::from_secs(10);
/// How long a tab is watched for a page it must not be sent.
const NOTHING_FOR: Duration = Duration::from_millis(300);
/// How many tabs are closed. With `send_last` taken out, the first of them
/// failed on 31 October 2026.
const TAB_CLOSES: usize = 8;
const TYPING: u16 = 0x200;
const LINKS: u16 = 0x500;
const INDEX: u16 = 0x100;

/// The programs, built once however many tests run.
fn built() -> &'static (PathBuf, PathBuf) {
    static PROGRAMS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    PROGRAMS.get_or_init(programs)
}

fn select(tab: &Tab, session: &mut Session, page: u16) {
    let select = Select::new(page, SUBCODE_ANY).unwrap();
    send(&tab.input(), &alone(message(session, &Item::Event(Event::Select(select, "")))));
}

/// What the site was asked for, up to the push connection's request.
fn asked_until_push(site: &Site, heard: &std::sync::mpsc::Receiver<String>, query: &str) -> Vec<String> {
    let tab = Tab::open_at(&built().1, &format!("http://127.0.0.1:{}/terminal/{query}", site.port));
    let said = said_until(heard, "GET /terminal/push", Instant::now() + LIMIT);
    drop(tab);
    said
}

// DEC-1730: an address's query goes with the selection, in SELECT, so the
// push connection's address is the same whatever the address holds, and the
// channel's files are asked for without it.
#[test]
fn an_address_query_stays_off_the_push_connection() {
    let (site, heard) = Site::start(&built().0);
    let said = asked_until_push(&site, &heard, "?id=7&b=%20x");
    assert!(said.last().unwrap().ends_with("GET /terminal/push"), "{said:?}");
    assert!(said.iter().any(|line| line.ends_with("GET /terminal/channel.json")), "{said:?}");
}

// REQ-1693 item 6, DEC-1715: the longest page parameters go whole in SELECT,
// through the fetcher and a real socket, to the channel.
#[test]
fn the_longest_page_parameters_reach_the_channel_in_select() {
    let (site, heard) = Site::start(&built().0);
    let tab = Tab::open(&built().1, site.port);
    let mut session = tab.session(Instant::now() + LIMIT);
    let parameters = format!("a={}", "b".repeat(neoteletext_input::MAX_PAGE_PARAMETERS - "a=".len()));
    let select = Select::new(TYPING, SUBCODE_ANY).unwrap();
    send(&tab.input(), &alone(message(&mut session, &Item::Event(Event::Select(select, &parameters)))));
    let said = said_until(&heard, "SELECT 200/3F7F", Instant::now() + LIMIT);
    assert!(said.last().unwrap().ends_with(&format!("SELECT 200/3F7F {parameters}")), "the parameters came whole");
}

// TODO-1682 item 1, DEC-1538 item 3: a tab that closes tells the channel it
// has left: SELECT of page 0 reaches the channel before the connection
// closes, though the fetcher's input closes with it, as `Fetcher::stop`
// closes it. Whether the fetcher reads the two together is a matter of
// moments, so it is tried many times.
#[test]
fn select_of_page_0_reaches_the_channel_before_the_connection_closes() {
    let (site, heard) = Site::start(&built().0);
    for _ in 0..TAB_CLOSES {
        let tab = Tab::open(&built().1, site.port);
        let mut session = tab.session(Instant::now() + LIMIT);
        select(&tab, &mut session, TYPING);
        tab.next_page(Instant::now() + LIMIT);
        let select_nothing = Select::new(0, SUBCODE_ANY).unwrap();
        tab.send_and_close(&alone(message(&mut session, &Item::Event(Event::Select(select_nothing, "")))));
        let said = said_until(&heard, "closed", Instant::now() + LIMIT);
        assert!(said.iter().any(|line| line.contains("SELECT 000/3F7F")), "the channel heard SELECT of page 0 first: {said:?}");
    }
}

// REQ-714 verification 2 and 3, TODO-1534: two tabs on one channel have a
// connection and a store each. A page selected in one is sent only there,
// its SELECT reaches only its connection, and killing one leaves the other
// connected, with what it holds.
#[test]
fn two_tabs_on_one_channel_are_apart() {
    let (site, heard) = Site::start(&built().0);
    let mut first = Tab::open(&built().1, site.port);
    let mut first_session = first.session(Instant::now() + LIMIT);
    let second = Tab::open(&built().1, site.port);
    let mut second_session = second.session(Instant::now() + LIMIT);
    let (mut first_store, mut second_store) = (decoder::Engine::new(), decoder::Engine::new());

    select(&first, &mut first_session, TYPING);
    first_store.feed_whole(true, true, &first.next_page(Instant::now() + LIMIT));
    let said = said_until(&heard, "SELECT 200/3F7F", Instant::now() + LIMIT);
    let connection = said.last().and_then(|line| line.split(':').nth(1)).unwrap().to_owned();
    assert!(!second.page_within(NOTHING_FOR), "the first tab's page is not sent to the second");

    select(&second, &mut second_session, LINKS);
    second_store.feed_whole(true, true, &second.next_page(Instant::now() + LIMIT));
    let said = said_until(&heard, "SELECT 500/3F7F", Instant::now() + LIMIT);
    let other = said.last().and_then(|line| line.split(':').nth(1)).unwrap().to_owned();
    assert_ne!(connection, other, "each SELECT on its own connection");
    assert!(!first.page_within(NOTHING_FOR), "the second tab's page is not sent to the first");
    assert!(first_store.render(LINKS).is_none() && first_store.render(TYPING).is_some(), "each store holds its own");
    assert!(second_store.render(TYPING).is_none() && second_store.render(LINKS).is_some(), "each store holds its own");

    first.kill();
    let said = said_until(&heard, "closed", Instant::now() + LIMIT);
    assert!(said.last().unwrap().contains(&connection), "{said:?}");
    select(&second, &mut second_session, INDEX);
    second_store.feed_whole(true, true, &second.next_page(Instant::now() + LIMIT));
    assert!(second_store.render(INDEX).is_some() && second_store.render(LINKS).is_some(), "still connected, with what it held");
}

// TODO-1536 item 7, REQ-1494: page 500 is never fetched as a file; its links
// come in its LINKS record alone, and both kinds are followed.
#[test]
fn page_500_is_followed_by_its_links_record_alone() {
    const STARTED: u64 = 0;
    let (site, heard) = Site::start(&built().0);
    let tab = Tab::open(&built().1, site.port);
    let mut session = tab.session(Instant::now() + LIMIT);
    select(&tab, &mut session, LINKS);
    let frame = tab.next_page(Instant::now() + LIMIT);
    let mut store = decoder::Engine::new();
    store.feed_whole(true, true, &frame);
    let rendered = store.render(LINKS).expect("the page renders");
    let candidates: Vec<Candidate> = rendered.links.candidates.clone();
    let targets: Vec<&str> = candidates.iter().map(|candidate| candidate.target.as_str()).collect();
    assert!(targets.contains(&"web:https://example.com/") && targets.contains(&"200"), "both kinds of link: {targets:?}");

    let key = PageKey { channel: 1, page: LINKS };
    let address = neoteletext::address::parse(&format!("http://127.0.0.1:{}/terminal/", site.port)).unwrap();
    let mut links = Links::default();
    links.opened(Some(key));
    links.found(key, candidates.clone(), rendered.links.numbers.clone(), Vec::new(), rendered.links.fastext);
    links.image(key, 1, &Channel { id: key.channel, address: &address }, STARTED);
    for (target, followed) in [("200", true), ("web:https://example.com/", false)] {
        let link = candidates.iter().find(|candidate| candidate.target == target).unwrap();
        let spot = Spot { key, version: 1, column: link.column, row: link.row };
        links.point(Some(spot));
        let follow = links.click(spot, Some(key.channel), STARTED + DWELL_MILLISECONDS);
        if followed {
            assert_eq!(follow, Ok(Follow::Page(TYPING)), "a page-number link");
        } else {
            assert!(matches!(follow, Ok(Follow::Ask(_))), "a web link asks: {follow:?}");
        }
    }
    tab.close_input();
    let said = said_until(&heard, "closed", Instant::now() + LIMIT);
    assert!(said.iter().all(|line| !line.contains("500.tti")), "page 500 was never fetched as a file: {said:?}");
}
