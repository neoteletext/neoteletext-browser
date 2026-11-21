//! Tests of the tabs, their history, the windows and the kept session.

use super::*;
use interface_core::channel::page_index;
use interface_core::library::{MAX_CLOSED, MAX_TAB_ENTRIES, MAX_TAB_LIST};
use neoteletext::session_file::Saved;
use protocol::fetcher::Entry;

#[test]
fn a_tab_stays_muted_on_its_next_channel_and_mute_all_reaches_every_tab() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    session.open("sport.invalid").unwrap();
    assert!(session.toggle_mute(0));
    assert!(!session.toggle_mute(2), "no such tab");
    let first = session.tab_infos()[0].channel;
    session.active = 0;
    session.navigate("weather.invalid", true).unwrap();
    let infos = session.tab_infos();
    assert!(infos[0].channel != first && infos[0].muted, "the tab's new channel is muted");
    assert!(!infos[1].muted);
    session.mute_all(true);
    assert!(session.tab_infos().iter().all(|tab| tab.muted));
    session.mute_all(false);
    assert!(session.tab_infos().iter().all(|tab| !tab.muted));
}

// REQ-6641: a tab is titled by the name its channel gave, cleaned and
// cut to a title, and by its host when the channel gave none, gave a blank
// one, or has not said it yet; a new channel in the tab starts unnamed.
#[test]
fn a_tab_is_titled_by_its_channels_name_else_its_host() {
    let mut session = Session::new(Bookmarks::default());
    session.open("https://app.neottx.com/demosite/").unwrap();
    assert_eq!(session.tab_infos()[0].title, "app.neottx.com", "not yet named");
    assert!(session.tabs[0].channel.set_name("NeoTeletext demo site"));
    assert_eq!(session.tab_infos()[0].title, "NeoTeletext demo site");
    assert!(!session.tabs[0].channel.set_name("NeoTeletext demo site"), "the same name changes nothing");
    session.tabs[0].channel.set_name("De\u{202E}mo\u{200B} ");
    assert_eq!(session.tab_infos()[0].title, "Demo", "reordering and hidden characters removed, spaces trimmed");
    session.tabs[0].channel.set_name(&"x".repeat(MAX_TAB_TITLE * 2));
    assert_eq!(session.tab_infos()[0].title.len(), MAX_TAB_TITLE, "cut to a title");
    session.tabs[0].channel.set_name(" \u{200B} ");
    assert_eq!(session.tab_infos()[0].title, "app.neottx.com", "a blank name is none");
    session.tabs[0].channel.set_name("ReGIS test");
    session.navigate("https://app.neottx.com/regis/", true).unwrap();
    assert_eq!(session.tab_infos()[0].title, "app.neottx.com", "another channel is unnamed until it says");
}

// REQ-714, DEC-1527: two tabs on one channel are two channels here, each
// with its own number and so its own fetcher, connection and decoder, and
// going to another address leaves the other tab's untouched.
#[test]
fn two_tabs_on_one_channel_share_nothing() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    session.open("news.invalid").unwrap();
    let ids = [session.tabs[0].channel.id, session.tabs[1].channel.id];
    assert_ne!(ids[0], ids[1]);
    session.navigate("news.invalid/200", true).unwrap();
    assert_eq!(session.tabs[0].channel.id, ids[0], "the other tab");
    assert!(!ids.contains(&session.tabs[1].channel.id), "still nobody else's");
}

// DEC-1722: the address field repairs what a viewer types, and an address
// reached any other way, as a bookmark is, is held to the map.
#[test]
fn a_typed_channel_needs_no_final_slash_and_a_bookmark_does() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    session.navigate("app.neottx.com/home", true).unwrap();
    assert_eq!(session.active_tab().and_then(Tab::address).unwrap().text, "https://app.neottx.com/home/");
    assert!(session.navigate("app.neottx.com/other", false).is_err());
}

#[test]
fn history_goes_back_and_forward_and_a_new_address_drops_forward() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    session.navigate("sport.invalid", true).unwrap();
    assert!(session.strip().back && !session.strip().forward);
    assert_eq!(session.go(false), Went::Channel);
    assert_eq!(session.strip().identity, "news.invalid");
    assert!(session.strip().forward);
    session.navigate("weather.invalid", true).unwrap();
    assert!(!session.strip().forward, "forward history is dropped");
    assert_eq!(session.go(true), Went::Nowhere);
}

#[cfg(feature = "links")]
#[test]
fn a_link_within_the_channel_keeps_it_open_and_is_recorded_as_the_pages() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid/teletext/").unwrap();
    let channel = session.tabs[0].channel.id;
    assert_eq!(session.show_page(0x302).map(|address| address.text.as_str()), Some("https://news.invalid/teletext/302"));
    assert!(session.tabs[0].history[1].by_page && !session.tabs[0].history[0].by_page);
    assert_eq!(session.tabs[0].channel.id, channel, "the same channel, still open");
    assert_eq!(session.go(false), Went::Page(None));
    assert_eq!(session.go(true), Went::Page(Some(0x302)));
    assert_eq!(session.tabs[0].channel.id, channel);
    // A link to another channel opens it, recorded as the page's.
    session.follow(parse("https://sport.invalid/401").unwrap()).unwrap();
    assert!(session.tabs[0].history[2].by_page);
    assert_ne!(session.tabs[0].channel.id, channel);
    assert_eq!(session.tabs[0].channel.web.as_deref(), Some("https://sport.invalid/"), "the fragment is never fetched");
}

#[test]
fn a_page_is_recorded_once_with_its_title_cleaned() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    let channel = &mut session.tabs[0].channel;
    assert!(channel.web.is_some(), "a web channel has a fetcher");
    assert_eq!(channel.arrived(0x100, "Head\u{202E}lines").as_deref(), Some("Headlines"));
    assert_eq!(channel.arrived(0x100, "Again"), None);
    assert_eq!(channel.arrived(0x1A0, "Hexadecimal"), None);
    assert_eq!(channel.view().counts(), (1, 1));
}

/// TODO-6748, DEC-6638 item 3: a picture's title that comes after its page
/// arrived names the page in place of its rows' title, cleaned; the same
/// title again changes nothing; a page the contents file names keeps the
/// file's title; a page not yet arrived takes its title with its arrival.
#[test]
fn a_pictures_title_after_its_page_arrived_names_the_page_unless_the_index_does() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    let channel = &mut session.tabs[0].channel;
    channel.set_contents(&[Entry::Page { page: 0x101, title: "Headlines".into(), subpages: 0 }]);
    assert_eq!(channel.picture_titled(0x104, "Weather map"), None, "not yet arrived");
    assert_eq!(channel.arrived(0x104, "ROW TEXT").as_deref(), Some("ROW TEXT"));
    assert_eq!(channel.picture_titled(0x104, "Weather\u{202E} map").as_deref(), Some("Weather map"));
    assert_eq!(channel.page_title(0x104), Some("Weather map"));
    assert_eq!(channel.picture_titled(0x104, "Weather map"), None, "the same again");
    assert_eq!(channel.picture_titled(0x104, "  "), None, "nothing left of it");
    assert_eq!(channel.arrived(0x101, "Something else").as_deref(), Some("Headlines"));
    assert_eq!(channel.picture_titled(0x101, "Picture"), None, "the index's title is the channel's word");
    assert_eq!(channel.page_title(0x101), Some("Headlines"));
}

/// Decision 111 and decision 112 item 4: a page's count of sub-pages is
/// kept, to the standard's 8,190, and told again only when it changes.
#[test]
fn a_pages_sub_pages_are_kept_and_told_when_they_change() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    let subpages = &mut session.tabs[0].channel.subpages;
    assert_eq!(subpages.stored(0x210, 2), Some(2));
    assert_eq!(subpages.stored(0x210, 2), None, "the same again");
    assert_eq!(subpages.stored(0x210, 1), Some(0), "one sub-page again");
    assert_eq!(subpages.stored(0x210, 1), None);
    assert_eq!(subpages.stored(0x210, u16::MAX), Some(8_190), "never past the standard's maximum");
}

#[test]
fn the_contents_file_lists_pages_and_names_sections() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    let channel = &mut session.tabs[0].channel;
    channel.set_contents(&[
        Entry::Section("News".into()),
        Entry::Page { page: 0x100, title: "Front page".into(), subpages: 0 },
        Entry::Page { page: 0x101, title: "Headlines".into(), subpages: 0 },
        Entry::Section("Sport".into()),
        Entry::Page { page: 0x300, title: "Sport index".into(), subpages: 0 },
    ]);
    let view = channel.view();
    assert_eq!(view.counts(), (3, 0), "listed, none arrived yet");
    assert_eq!((view.banks[0].as_str(), view.banks[2].as_str()), ("News", "Sport"));
    // The page's own title does not replace the contents file's.
    assert_eq!(channel.arrived(0x101, "Something else").as_deref(), Some("Headlines"));
    assert_eq!(channel.view().counts(), (3, 1));
}

/// DEC-6358 items 1 and 2: a page's count of sub-pages is the index's, and
/// goes to the interface in the page's own entry. Fault 11: a page a live
/// index drops is listed again when it next arrives.
#[test]
fn the_index_declares_each_pages_sub_pages_and_a_dropped_page_can_return() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    let channel = &mut session.tabs[0].channel;
    channel.live = true;
    let index = |pages: &[(u16, u16)]| -> Vec<Entry> {
        pages.iter().map(|&(page, subpages)| Entry::Page { page, title: format!("Set {page:03X}"), subpages }).collect()
    };
    channel.set_contents(&index(&[(0x100, 1), (0x101, 10), (0x102, 5)]));
    let counts = |channel: &ChannelState| -> Vec<u16> { channel.view().pages.iter().flatten().map(|page| page.subpages).collect() };
    assert_eq!(counts(channel), [0, 10, 5], "each page's count beside its title");
    // What the decoder holds of a page changes nothing the channel declared.
    assert_eq!(channel.subpages.stored(0x101, 1), None);
    assert_eq!(counts(channel), [0, 10, 5]);
    // The page arrives, then the index drops it, then it arrives again.
    assert!(channel.arrived(0x102, "Set 102").is_some());
    channel.set_contents(&index(&[(0x100, 1), (0x101, 10)]));
    assert_eq!(counts(channel), [0, 10], "no longer listed");
    assert_eq!(channel.arrived(0x102, "Back again").as_deref(), Some("Back again"), "listed again when it arrives");
}

#[test]
fn closing_tabs_keeps_a_valid_active_tab() {
    let mut session = Session::new(Bookmarks::default());
    for host in ["a.invalid", "b.invalid", "c.invalid"] {
        session.open(host).unwrap();
    }
    assert_eq!(session.active, 2);
    session.close(2);
    assert_eq!(session.active, 1);
    session.close(0);
    assert_eq!(session.active, 0);
    assert_eq!(session.strip().identity, "b.invalid");
}

/// Requirement 20.16: the star is filled for the address the strip
/// shows, page and all.
#[test]
fn bookmarks_follow_the_address() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid/302").unwrap();
    assert!(!session.strip().bookmarked);
    session.bookmarks.add(neoteletext::bookmarks::OTHER, "", "news.invalid/302").unwrap();
    assert!(session.strip().bookmarked);
    session.navigate("news.invalid", true).unwrap();
    assert!(!session.strip().bookmarked, "the channel without the page is another address");
    assert_eq!(session.go(false), Went::Page(Some(0x302)), "the same channel");
    assert!(session.strip().bookmarked);
}

/// Requirement 20.9: a closed tab comes back where it was, with its
/// whole history; a private one is not kept; at most 25.
#[test]
fn closed_tabs_reopen_where_they_were() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    session.navigate("sport.invalid", true).unwrap();
    session.open_private("secret.invalid").unwrap();
    session.close(1);
    session.close(0);
    assert_eq!(session.closed.len(), 1, "the private tab is not kept");
    assert_eq!(session.closed[0].title, "sport.invalid");
    session.open("weather.invalid").unwrap();
    session.reopen(None).unwrap();
    assert_eq!(session.strip().identity, "sport.invalid");
    assert!(session.strip().back, "its history came back with it");
    assert_eq!(session.go(false), Went::Channel);
    assert_eq!(session.strip().identity, "news.invalid");
    assert!(session.reopen(None).is_err());
    for n in 0..30 {
        session.open(&format!("b{n}.invalid")).unwrap();
        session.close(session.active);
    }
    assert_eq!(session.closed.len(), MAX_CLOSED);
    let id = session.closed[3].id;
    session.reopen(Some(id)).unwrap();
    assert_eq!(session.strip().identity, "b26.invalid");
}

/// Requirements 20.21 and 20.22: the session kept is every tab but the
/// private ones, in order, each with its list, place and mute; restored,
/// it is the same, with the same tab active, each tab waiting for its
/// page to draw.
#[test]
fn the_session_is_kept_and_restored() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    session.navigate("sport.invalid", true).unwrap();
    session.go(false);
    session.open_private("secret.invalid").unwrap();
    session.open("weather.invalid").unwrap();
    session.toggle_mute(2);
    session.tabs[0].history[0].title = "NEWS".into();
    let window = Some(WindowGeometry { x: 5, y: 6, width: 700, height: 500, maximised: true });
    session.geometry = window;
    let saved = session.saved();
    let kept = &saved.windows[0];
    assert_eq!(kept.geometry, window);
    let lists: Vec<Vec<&str>> = kept.tabs.iter().map(|tab| tab.entries.iter().map(|entry| entry.address.as_str()).collect()).collect();
    assert_eq!(lists, [vec!["https://news.invalid/", "https://sport.invalid/"], vec!["https://weather.invalid/"]]);
    assert_eq!((kept.tabs[0].position, kept.tabs[0].entries[0].title.as_str()), (0, "NEWS"));
    assert_eq!((kept.active, kept.tabs[1].muted), (1, true));

    let mut restored = Session::new(Bookmarks::default());
    assert_eq!(restored.restore(&saved), 2);
    assert_eq!(restored.saved(), saved, "restored as it was");
    assert_eq!(restored.active, 1);
    assert!(restored.tabs.iter().all(|tab| tab.pending == Some(Pending { typed: false, drawn: false, reload: false })));
    restored.active = 0;
    assert_eq!(restored.go(true), Went::Channel, "its forward list came back with it");
    assert_eq!(restored.strip().identity, "sport.invalid");

    session.active = 1;
    assert_eq!(session.saved().windows[0].active, 0, "a private active tab keeps the one before it");
    session.close(2);
    session.close(0);
    assert!(session.saved().windows.is_empty(), "only a private tab: none kept");
}

/// Requirement 20.10: the lists one way, the nearest first, and going
/// several steps at once.
/// A session with one tab of `addresses`, its place on the last.
fn listed(addresses: &[&str]) -> Session {
    let mut session = Session::new(Bookmarks::default());
    session.open(addresses[0]).unwrap();
    for address in &addresses[1..] {
        session.navigate(address, true).unwrap();
    }
    session
}

// DEC-1684 item 4: the buttons pass over what is marked, the lists do not;
// Forward as Back.
#[test]
fn the_buttons_pass_over_a_marked_entry() {
    let mut session = listed(&["a.invalid", "b.invalid", "c.invalid"]);
    session.tabs[0].history[1].passed_over = true;
    assert_eq!(session.go(false), Went::Channel);
    assert_eq!(session.tabs[0].position, 0, "past the marked one");
    assert_eq!(session.go(true), Went::Channel);
    assert_eq!(session.tabs[0].position, 2, "past it again");
    assert_eq!(session.tab_list(false).len(), 2, "it stays in the list");
    assert_eq!(session.go_to(false, 1), Went::Channel, "and the list goes to it");
    assert_eq!(session.tabs[0].position, 1);
}

// DEC-1684 item 8: with every earlier entry marked, Back goes nowhere, and
// stays enabled for its list.
#[test]
fn with_every_earlier_entry_marked_back_does_nothing() {
    let mut session = listed(&["a.invalid", "b.invalid"]);
    session.tabs[0].history[0].passed_over = true;
    assert_eq!(session.go(false), Went::Nowhere);
    assert_eq!(session.tabs[0].position, 1);
    assert!(session.strip().back, "enabled, so that its list opens");
}

// DEC-1684 item 8: a full list drops its oldest marked entry first.
#[test]
fn a_full_list_drops_its_oldest_marked_entry_first() {
    let mut session = listed(&["first.invalid"]);
    for number in 1..MAX_TAB_ENTRIES {
        session.navigate(&format!("page{number}.invalid"), true).unwrap();
    }
    session.tabs[0].history[3].passed_over = true;
    session.tabs[0].history[5].passed_over = true;
    let (third, fifth) = (session.tabs[0].history[3].address.text.clone(), session.tabs[0].history[5].address.text.clone());
    session.navigate("one-more.invalid", true).unwrap();
    let history = &session.tabs[0].history;
    assert_eq!(history.len(), MAX_TAB_ENTRIES);
    assert!(history.iter().all(|visit| visit.address.text != third), "the oldest marked");
    assert!(history.iter().any(|visit| visit.address.text == fifth));
    assert_eq!(history[0].address.host, "first.invalid", "the oldest of all stays");
    session.tabs[0].history.iter_mut().for_each(|visit| visit.passed_over = false);
    session.navigate("and-another.invalid", true).unwrap();
    assert_ne!(session.tabs[0].history[0].address.host, "first.invalid", "with none marked, the oldest of all");
}

// DEC-1684 item 9: a saved tab keeps its marks.
#[test]
fn a_saved_tab_keeps_its_marks() {
    let mut session = listed(&["a.invalid", "b.invalid"]);
    session.tabs[0].history[0].passed_over = true;
    let saved = session.saved();
    let mut restored = Session::new(Bookmarks::default());
    restored.restore(&saved);
    assert!(restored.tabs[0].history[0].passed_over);
    assert!(!restored.tabs[0].history[1].passed_over);
}

#[test]
fn back_and_forward_lists_go_several_steps() {
    let mut session = Session::new(Bookmarks::default());
    session.open("a.invalid").unwrap();
    for host in ["b.invalid", "c.invalid", "d.invalid"] {
        session.navigate(host, true).unwrap();
    }
    let back: Vec<&str> = session.tab_list(false).iter().map(|(_, visit)| visit.address.host.as_str()).collect();
    assert_eq!(back, ["c.invalid", "b.invalid", "a.invalid"]);
    assert!(session.tab_list(true).is_empty());
    assert_eq!(session.go_to(false, 3), Went::Channel);
    assert_eq!(session.strip().identity, "a.invalid");
    assert_eq!(session.go_to(false, 1), Went::Nowhere);
    assert_eq!(session.go_to(true, 0), Went::Nowhere);
    assert_eq!(session.tab_list(true).len(), 3);
}

/// Requirement 20.1: each navigation waits to be recorded; typed in the
/// field or not; within a channel, at once.
#[test]
fn navigations_wait_to_be_recorded() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid/teletext/").unwrap();
    assert_eq!(session.tabs[0].pending, Some(Pending { typed: false, drawn: false, reload: false }));
    session.tabs[0].pending = None;
    session.navigate("sport.invalid", true).unwrap();
    assert_eq!(session.tabs[0].pending, Some(Pending { typed: true, drawn: false, reload: false }));
    session.navigate("sport.invalid/401", false).unwrap();
    session.tabs[0].pending = None;
    assert_eq!(session.go(false), Went::Page(None));
    assert_eq!(session.tabs[0].pending, Some(Pending { typed: false, drawn: true, reload: false }), "the same channel, already drawing");
    session.open_private("news.invalid").unwrap();
    assert!(session.tab_infos()[1].private && session.strip().private);
}

/// Requirement 20.10 (`history-and-bookmarks.md` section 16): twelve entries, then,
/// beyond them, up to five chapter stops where the host changes; a
/// tab keeps at most 50 entries, the oldest dropped.
#[test]
fn back_lists_twelve_then_chapter_stops_and_a_tab_keeps_fifty() {
    let mut session = Session::new(Bookmarks::default());
    session.open("a.invalid").unwrap();
    // a, then b×3, c×3, d×3, e×3, f×3, g×3, h×3: 22 entries, the last h.
    for host in ["b", "c", "d", "e", "f", "g", "h"] {
        for page in ["100", "200", "300"] {
            session.navigate(&format!("{host}.invalid/{page}"), false).unwrap();
        }
    }
    let hosts = |list: Vec<(usize, &Visit)>| list.into_iter().map(|(steps, visit)| (steps, visit.address.host.clone())).collect::<Vec<_>>();
    assert_eq!(session.tab_list(false).len(), MAX_TAB_LIST);
    // The twelfth back is at position 21 - 12 = 9, a d; the stops are
    // the nearest earlier entry of another host each time: c at 6,
    // b at 3, a at 0.
    assert_eq!(hosts(session.chapter_stops(false)), [(15, "c.invalid".into()), (18, "b.invalid".into()), (21, "a.invalid".into())]);
    assert!(session.chapter_stops(true).is_empty(), "nothing forward");
    assert_eq!(session.go_to(false, 21), Went::Channel);
    // Forward from a: twelve, then the last entry of each run of hosts
    // after the twelfth (position 12, an e): f ends at 15, g at 18, and
    // the last entry, an h, at 21.
    assert_eq!(hosts(session.chapter_stops(true)), [(15, "f.invalid".into()), (18, "g.invalid".into()), (21, "h.invalid".into())]);
    for page in 0..60 {
        session.navigate(&format!("z.invalid/{:03}", 100 + page), false).unwrap();
    }
    let tab = &session.tabs[0];
    assert_eq!((tab.history.len(), tab.position), (MAX_TAB_ENTRIES, MAX_TAB_ENTRIES - 1), "the oldest dropped");
}

/// Requirement 20.6 (`history-and-bookmarks.md` section 16): removing history removes those entries
/// from the tabs' lists but never the entry a tab is at, drops a closed
/// tab whose entry it was at, and leaves private tabs alone.
#[test]
fn removed_history_leaves_the_tabs_lists() {
    let mut session = Session::new(Bookmarks::default());
    let addresses = |history: &[Visit]| history.iter().map(|visit| visit.address.host.clone()).collect::<Vec<_>>();
    for (at, host) in [(100, "a"), (200, "b"), (300, "c")] {
        session.now = at;
        if session.tabs.is_empty() {
            session.open(&format!("{host}.invalid")).unwrap();
        } else {
            session.navigate(&format!("{host}.invalid"), false).unwrap();
        }
    }
    session.now = 400;
    session.open_private("p.invalid").unwrap();
    session.navigate("q.invalid", false).unwrap();
    // A closed tab at b, and one at c.
    for host in ["b", "c"] {
        session.now = 500;
        session.open(&format!("{host}.invalid")).unwrap();
        session.close(session.tabs.len() - 1);
    }
    assert_eq!(session.closed.len(), 2);
    session.active = 0;
    assert_eq!(session.go_to(false, 1), Went::Channel, "the first tab at b");
    // Everything from 150 on: the first tab keeps a and b, which it is
    // at; the closed tabs were at b and c from 500, so both go.
    assert!(session.remove_entries(|visit| visit.at >= 150 && visit.address.host != "a.invalid"));
    assert_eq!(addresses(&session.tabs[0].history), ["a.invalid", "b.invalid"]);
    assert_eq!(session.tabs[0].position, 1);
    assert_eq!(addresses(&session.tabs[1].history), ["p.invalid", "q.invalid"], "a private tab is not touched");
    assert!(session.closed.is_empty());
    assert!(!session.remove_entries(|visit| visit.host_is("z.invalid")), "nothing named, nothing changed");
}

/// Requirement 20.9 (`history-and-bookmarks.md` section 16): the recently closed tabs, with each
/// entry's time, are kept with the session and come back with it.
#[test]
fn closed_tabs_are_kept_between_runs() {
    let mut session = Session::new(Bookmarks::default());
    session.now = 1_000;
    session.open("a.invalid").unwrap();
    session.open("b.invalid").unwrap();
    session.navigate("c.invalid", false).unwrap();
    session.close(1);
    let saved = session.saved();
    assert_eq!(saved.closed.len(), 1);
    let mut restored = Session::new(Bookmarks::default());
    restored.restore(&Saved::read(&saved.text()).0);
    let closed = &restored.closed[0];
    assert_eq!((closed.position, closed.title.as_str()), (1, "c.invalid"));
    assert_eq!(closed.history.iter().map(|visit| visit.at).collect::<Vec<_>>(), [1_000, 1_000]);
    restored.reopen(None).unwrap();
    assert_eq!(restored.active_tab().and_then(Tab::address).map(|address| address.host.as_str()), Some("c.invalid"));
}

/// Decision 69: a tab moves; the active tab stays active wherever it
/// goes and wherever the others go round it.
#[test]
fn tabs_move_and_the_active_one_stays_active() {
    let mut session = Session::new(Bookmarks::default());
    for host in ["a", "b", "c", "d"] {
        session.open(&format!("{host}.invalid")).unwrap();
    }
    let hosts = |session: &Session| session.tabs.iter().map(|tab| tab.address().unwrap().host.clone()).collect::<Vec<_>>();
    session.active = 1;
    assert!(session.move_tab(3, 0));
    assert_eq!(hosts(&session), ["d.invalid", "a.invalid", "b.invalid", "c.invalid"]);
    assert_eq!(session.active, 2, "b moved right with the others");
    assert!(session.move_tab(2, 3));
    assert_eq!((hosts(&session)[3].as_str(), session.active), ("b.invalid", 3), "the active tab itself");
    assert!(session.move_tab(0, 2));
    assert_eq!(session.active, 3, "a tab passing from before it to before it: unchanged");
    assert!(!session.move_tab(1, 1) && !session.move_tab(0, 9), "nowhere to go");
}

/// Requirement 11.6: a reload gives the tab a new channel at the same
/// address, its history as it was, and waits to record a reload.
#[test]
fn a_reload_opens_the_channel_afresh_where_the_tab_is() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid/teletext/").unwrap();
    session.navigate("sport.invalid/401", false).unwrap();
    assert_eq!(session.go(false), Went::Channel);
    let addresses = |tab: &Tab| tab.history.iter().map(|visit| visit.address.text.clone()).collect::<Vec<_>>();
    let (before, history, position) = (session.tabs[0].channel.id, addresses(&session.tabs[0]), session.tabs[0].position);
    session.tabs[0].pending = None;
    assert!(session.reload());
    let tab = &session.tabs[0];
    assert_ne!(tab.channel.id, before, "a new channel: a new decoder and fetcher");
    assert_eq!((addresses(tab), tab.position), (history.clone(), position), "the history as it was");
    assert_eq!(tab.address().map(|address| address.text.clone()), history.get(position).cloned());
    assert_eq!(tab.pending, Some(Pending { typed: false, drawn: false, reload: true }));
    assert!(!Session::new(Bookmarks::default()).reload(), "no tab, nothing to reload");
    // The address shown, entered again or linked to, is a reload
    // (`history-and-bookmarks.md` section 16): no new entry.
    let shown = session.tabs[0].address().map(|address| address.text.clone()).unwrap();
    session.tabs[0].pending = None;
    session.navigate(&shown, true).unwrap();
    assert_eq!((addresses(&session.tabs[0]), session.tabs[0].position), (history.clone(), position));
    assert_eq!(session.tabs[0].pending, Some(Pending { typed: false, drawn: false, reload: true }), "not typed, a reload");
    #[cfg(feature = "links")]
    {
        session.follow(parse(&shown).unwrap()).unwrap();
        assert_eq!(addresses(&session.tabs[0]), history, "a link to it is one too");
    }
}

/// Requirement 19.7: the start page is served from the bookmarks with
/// no fetcher, and its bookmark pages open them as the user's own
/// navigation; back returns to its front page.
#[test]
fn the_start_page_opens_its_bookmarks() {
    let mut bookmarks = Bookmarks::default();
    bookmarks.add(neoteletext::bookmarks::BAR, "News", "https://news.invalid/").unwrap();
    let mut session = Session::new(bookmarks);
    session.open(START_PAGE).unwrap();
    let channel = &session.tabs[0].channel;
    assert!(channel.web.is_none() && channel.problem.is_none(), "no network");
    assert_eq!(channel.view().pages[start_page_index(0x201)].as_ref().map(|page| page.title.as_str()), Some("News"));
    assert!(session.strip().built_in);
    let id = channel.id;
    assert_eq!(session.start_page_bookmark(id, 0x201).as_deref(), Some("https://news.invalid/"));
    assert_eq!(session.start_page_bookmark(id, 0x202), None);
    assert_eq!(session.start_page_bookmark(id.wrapping_add(1), 0x201), None, "another channel's page");
    session.tabs[0].history[0].address = parse("neoteletext:home#201").unwrap();
    session.open_from_start_page("https://news.invalid/").unwrap();
    assert_eq!(session.strip().identity, "news.invalid");
    assert!(!session.tabs[0].history[1].by_page, "the user's own navigation");
    assert_eq!(session.go(false), Went::Channel);
    assert_eq!(session.strip().address, START_PAGE, "back at the front page, not the bookmark's");
    let id = session.tabs[0].channel.id;
    assert_eq!(session.start_page_bookmark(id, 0x201).as_deref(), Some("https://news.invalid/"));
    session.navigate("news.invalid", true).unwrap();
    let id = session.tabs[0].channel.id;
    assert_eq!(session.start_page_bookmark(id, 0x201), None, "only on the start page");
}

/// `multiple-windows.md`: each window keeps its own tabs and active
/// tab; switching sets one aside and brings the other back.
#[test]
fn windows_keep_their_own_tabs() {
    let mut session = Session::new(Bookmarks::default());
    session.open("a.invalid").unwrap();
    session.open("b.invalid").unwrap();
    session.active = 0;
    let first = session.window;
    let second = session.new_window(false).unwrap();
    assert!(session.tabs.is_empty() && session.window == second && session.window_count() == 2);
    session.open("c.invalid").unwrap();
    let c = session.tabs[0].channel.id;
    assert_eq!(session.window_of(c), Some(second));
    assert!(session.switch_to(first));
    assert_eq!((session.tabs.len(), session.active), (2, 0), "the first window as it was");
    assert_eq!(session.window_of(c), Some(second));
    assert_eq!(session.all_tabs().count(), 3);
    assert!(!session.switch_to(99), "no such window");
    assert_eq!(session.window_ids(), [first, second]);
    // A private window's tabs are private.
    let private = session.new_window(true).unwrap();
    session.open("d.invalid").unwrap();
    assert!(session.tabs[0].private);
    session.switch_to(first);
    session.open("e.invalid").unwrap();
    assert!(!session.tabs[2].private, "only in the private window");
    assert_eq!(session.window_ids(), [first, second, private]);
}

/// A tab moves between windows; a window closes with its tabs, which
/// reopen together in a window of their own (`multiple-windows.md`
/// section 5).
#[test]
fn a_closed_window_reopens_whole() {
    let mut session = Session::new(Bookmarks::default());
    session.open("a.invalid").unwrap();
    let first = session.window;
    let second = session.new_window(false).unwrap();
    session.open("b.invalid").unwrap();
    session.open("c.invalid").unwrap();
    let moved = session.take_tab(1).unwrap();
    session.switch_to(first);
    session.put_tab(moved, 0);
    assert_eq!(session.active_tab().and_then(Tab::address).map(|address| address.host.as_str()), Some("c.invalid"));
    session.switch_to(second);
    session.open("d.invalid").unwrap();
    assert!(session.close_window().len() == 2 && session.window == first && session.window_count() == 1);
    let groups: Vec<(&str, bool)> = session.closed.iter().map(|closed| (closed.title.as_str(), closed.window == second)).collect();
    assert_eq!(groups, [("d.invalid", true), ("b.invalid", true)]);
    assert!(session.close_window().is_empty(), "the last window does not close");
    session.reopen(None).unwrap();
    assert_eq!(session.window_count(), 2);
    let hosts: Vec<&str> = session.tabs.iter().map(|tab| tab.address().unwrap().host.as_str()).collect();
    assert_eq!(hosts, ["b.invalid", "d.invalid"], "in their order, in a new window");
    assert!(session.closed.is_empty());
}

/// Every window is kept with the session, and restored as windows.
#[test]
fn every_window_is_kept_and_restored() {
    let mut session = Session::new(Bookmarks::default());
    session.open("a.invalid").unwrap();
    session.geometry = Some(WindowGeometry { x: 0, y: 0, width: 800, height: 600, maximised: false });
    session.new_window(false).unwrap();
    session.open("b.invalid").unwrap();
    session.open("c.invalid").unwrap();
    session.active = 0;
    session.geometry = Some(WindowGeometry { x: 900, y: 0, width: 700, height: 500, maximised: false });
    session.new_window(true).unwrap();
    session.open("secret.invalid").unwrap();
    let saved = session.saved();
    assert_eq!(saved.windows.len(), 2, "a private window is not kept");
    let mut restored = Session::new(Bookmarks::default());
    assert_eq!(restored.restore(&Saved::read(&saved.text()).0), 3);
    assert_eq!(restored.window_count(), 2);
    assert_eq!(restored.saved(), saved);
    assert_eq!(restored.tabs.len(), 1, "the first window is the one at hand");
}

/// The session holds at most `MAX_TABS` tabs in all its windows.
#[test]
fn tabs_are_bounded_across_windows() {
    let mut session = Session::new(Bookmarks::default());
    for _ in 0..MAX_TABS / 2 {
        session.open("a.invalid").unwrap();
    }
    session.new_window(false).unwrap();
    for _ in 0..MAX_TABS / 2 {
        session.open("b.invalid").unwrap();
    }
    assert!(session.open("c.invalid").is_err());
    assert!(session.room_for_a_tab().is_err());
    assert_eq!(session.tab_count(), MAX_TABS);
}

fn start_page_index(page: u16) -> usize {
    page_index(page).unwrap()
}

/// Finding F-53: restored file tabs are not read at start, through the
/// broker, all at once: a file tab is read when it is first shown.
#[test]
fn a_restored_file_tab_is_read_only_when_first_shown() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    const FILE_TABS: usize = 64;
    let mut session = Session::new(Bookmarks::default());
    // A file address is an absolute path, which on Windows begins with a
    // drive: there, "/recordings/..." is read as a host name.
    let folder = if cfg!(windows) { r"C:\recordings\" } else { "/recordings/" };
    for number in 0..FILE_TABS {
        session.open(&format!("{folder}{number}.t42")).unwrap();
    }
    let saved = session.saved();
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&reads);
    let mut restored = Session::with_reader(
        Bookmarks::default(),
        Box::new(move |_: &Path| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(Source::from_bytes(b"PN,10000\n".to_vec()))
        }),
    );
    assert_eq!(restored.restore(&Saved::read(&saved.text()).0), FILE_TABS);
    assert_eq!(reads.load(Ordering::Relaxed), 0, "no file read at start");
    let channel = restored.tabs[3].channel.id;
    assert_eq!(restored.read_unread(channel), None);
    assert_eq!(reads.load(Ordering::Relaxed), 1, "the one shown");
    assert_eq!(restored.read_unread(channel), None);
    assert_eq!(reads.load(Ordering::Relaxed), 1, "read once");
    assert!(restored.tabs[3].channel.source.has_more());
}

/// RFC 9110 section 4.1's 8,000 octets, with a page after a channel's
/// address, written out so that these tests say the requirement, not
/// whatever the constant holds (neoteletext-common's REQ-1661 item 3).
fn longest() -> String {
    const LONGEST: usize = 8_000;
    const HEAD: &str = "https://a.example/";
    const PAGE: &str = "302/0002";
    format!("{HEAD}{PAGE}?q={}", "x".repeat(LONGEST.saturating_sub(HEAD.len()).saturating_sub("?q=".len()).saturating_sub(PAGE.len())))
}

#[test]
fn the_session_file_keeps_an_address_of_8000_octets() {
    let mut session = Session::new(Bookmarks::default());
    session.open(&longest()).unwrap();
    let (read, dropped) = Saved::read(&session.saved().text());
    assert!(dropped.is_empty(), "{dropped:?}");
    let mut restored = Session::new(Bookmarks::default());
    assert_eq!(restored.restore(&read), 1);
    assert_eq!(restored.active_tab().and_then(|tab| tab.address()).map(|address| address.text.as_str()), Some(longest().as_str()));
}

/// A tab's tip shows the address cut as a view's is, so that all the tabs
/// of a window at the longest still go in one message.
#[test]
fn a_tabs_tip_is_cut_as_a_view_is() {
    let mut session = Session::new(Bookmarks::default());
    session.open(&longest()).unwrap();
    let tip = session.tab_infos().remove(0).address;
    assert_eq!(tip, shown_address(&longest()));
    assert!(tip.len() <= protocol::library::MAX_SHOWN_ADDRESS);
    assert!(tip.starts_with("https://a.example/") && tip.ends_with('\u{2026}'), "{tip}");
}

// REQ-749, DEC-2340: deceptive and internationalised addresses. Each is
// refused, or shown in the field with its whole real host, in ASCII, as the
// emphasised part, so no letter of another script and no user name can pass
// for a familiar site. On the code before, a host typed in Unicode was
// refused with no word of how to open it.
#[test]
fn deceptive_and_internationalised_addresses_are_refused_or_shown_by_their_real_host() {
    use neoteletext::address::NOT_ASCII_HOST;
    let shown = [
        // A familiar name to the left of the real host: the whole host is the identity.
        ("bank.example.evil.invalid", "bank.example.evil.invalid"),
        // An internationalised name in its xn-- form stays in that form (a
        // Cyrillic "аррӏе"), never drawn in Unicode.
        ("https://xn--80ak6aa92e.com/", "xn--80ak6aa92e.com"),
        ("https://xn--bcher-kva.example/", "xn--bcher-kva.example"),
        // Capitals are the same host, shown in lower case.
        ("HTTPS://NEWS.INVALID/", "news.invalid"),
    ];
    for (typed, host) in shown {
        let mut session = Session::new(Bookmarks::default());
        session.open("start.invalid").unwrap();
        session.navigate(typed, true).unwrap();
        let strip = session.strip();
        assert_eq!(strip.identity, host, "{typed}");
        assert!(strip.address.contains(host) && strip.address.is_ascii(), "{typed}: {}", strip.address);
    }
    let unicode = [
        // A Cyrillic "а" in place of the Latin one.
        "https://\u{0430}pple.com/",
        "https://bücher.example/",
        // Fullwidth Latin letters.
        "https://\u{FF45}xample.invalid/",
        // A dotless i of Turkish.
        "https://\u{0131}nvalid.example/",
    ];
    for typed in unicode {
        assert_eq!(parse(typed).unwrap_err(), NOT_ASCII_HOST, "{typed}");
    }
    for typed in ["https://bank.example@evil.invalid/", "https://example.invalid/\u{202E}gpj", "https://exa mple.invalid/"] {
        let refused = parse(typed).unwrap_err();
        assert_ne!(refused, NOT_ASCII_HOST, "{typed}: refused for what it is");
    }
}

/// REQ-7164: the notice of a tab going to another channel, started where
/// the session gives a tab another channel and nowhere else.
mod navigating {
    #![allow(clippy::arithmetic_side_effects, reason = "a test's times")]

    use super::*;
    use std::time::{Duration, Instant};

    use crate::session::channel::NAVIGATING_NOTICE;

    /// A session with one tab on a channel, shown and its notice, if any,
    /// cleared, as after a page drew.
    fn on_news() -> Session {
        let mut session = Session::new(Bookmarks::default());
        session.open("https://news.invalid/teletext/").unwrap();
        assert_eq!(session.tabs[0].channel.navigating, None, "a tab newly opened says nothing");
        session
    }

    fn notice(session: &Session) -> Option<Navigating> {
        session.active_tab().and_then(|tab| tab.channel.navigating)
    }

    /// Asserts a notice started between `before` and now, lasting
    /// `NAVIGATING_NOTICE` from its start.
    fn started_since(session: &Session, before: Instant) {
        let after = Instant::now();
        let Some(Navigating { until: Some(until), ready: false }) = notice(session) else { panic!("{:?}", notice(session)) };
        assert!(until >= before + NAVIGATING_NOTICE && until <= after + NAVIGATING_NOTICE, "{until:?} is the notice's length after its start");
        assert!(session.strip().navigating, "the strip says so");
    }

    // REQ-7164 items 1 to 3: another channel's address, typed or opened.
    #[test]
    fn another_channels_address_starts_a_notice_of_one_second() {
        let mut session = on_news();
        let before = Instant::now();
        session.navigate("https://sport.invalid/", true).unwrap();
        started_since(&session, before);
        let mut session = on_news();
        let before = Instant::now();
        session.navigate("https://news.invalid/other/", false).unwrap();
        started_since(&session, before);
    }

    // REQ-7164 item 4: another page of the channel shown, the address shown
    // again, a reload, a page or sub-page chosen, a link within the channel
    // and the channel's own redirect to a page start nothing.
    #[test]
    fn nothing_within_the_channel_starts_a_notice() {
        let mut session = on_news();
        session.navigate("https://news.invalid/teletext/200", true).unwrap();
        assert_eq!(notice(&session), None, "another page of the channel, typed");
        session.navigate("https://news.invalid/teletext/200", true).unwrap();
        assert_eq!(notice(&session), None, "the address shown, entered again");
        assert!(session.reload());
        assert_eq!(notice(&session), None, "a reload");
        assert!(session.page_chosen(0x300));
        assert_eq!(notice(&session), None, "a page chosen");
        assert!(session.subpage_chosen(0x300, Some(2)));
        assert_eq!(notice(&session), None, "a sub-page chosen");
        #[cfg(feature = "links")]
        {
            assert!(session.show_page(0x400).is_some());
            assert_eq!(notice(&session), None, "a link within the channel");
        }
        let channel = session.tabs[0].channel.id;
        session.redirect_page(channel, 0x500, None, None);
        assert!(session.active_tab().and_then(Tab::address).is_some_and(|shown| shown.page == Some(0x500)), "redirected");
        assert_eq!(notice(&session), None, "the channel's own redirect to a page");
        assert!(!session.strip().navigating);
    }

    // REQ-7164 item 1: Back and Forward to another channel's entry start a
    // notice; within the channel they do not.
    #[test]
    fn back_and_forward_start_a_notice_only_across_channels() {
        let mut session = on_news();
        session.page_chosen(0x200);
        session.navigate("https://sport.invalid/", true).unwrap();
        session.tabs[0].channel.navigating = None;
        let before = Instant::now();
        assert_eq!(session.go(false), Went::Channel);
        started_since(&session, before);
        session.tabs[0].channel.navigating = None;
        assert!(matches!(session.go(false), Went::Page(_)), "back within the channel");
        assert_eq!(notice(&session), None, "within the channel");
        let before = Instant::now();
        assert_eq!(session.go_to(true, 2), Went::Channel);
        started_since(&session, before);
    }

    // REQ-7164 item 1: a link to another channel, and a bookmark chosen on
    // the start page.
    #[test]
    fn a_followed_link_and_a_start_page_bookmark_start_a_notice() {
        #[cfg(feature = "links")]
        {
            let mut session = on_news();
            let before = Instant::now();
            session.follow(parse("https://sport.invalid/401").unwrap()).unwrap();
            started_since(&session, before);
        }
        let mut session = Session::new(Bookmarks::default());
        session.open(START_PAGE).unwrap();
        let before = Instant::now();
        session.open_from_start_page("https://news.invalid/").unwrap();
        started_since(&session, before);
    }

    // REQ-7164 item 1: a tab newly made shows its channel from the start,
    // with nothing to leave: a new tab, a reopened one and a restored
    // session start no notice.
    #[test]
    fn a_tab_newly_made_has_no_notice() {
        let mut session = on_news();
        session.navigate("https://sport.invalid/", true).unwrap();
        session.open("https://weather.invalid/").unwrap();
        assert_eq!(notice(&session), None, "opened");
        session.close(0);
        session.reopen(None).unwrap();
        assert_eq!(notice(&session), None, "reopened");
        let saved = session.saved();
        let mut restored = Session::new(Bookmarks::default());
        assert!(restored.restore(&saved) > 0);
        assert!(restored.tabs.iter().all(|tab| tab.channel.navigating.is_none()), "restored");
    }

    // DEC-7166 item 1: after a redirect out of the channel the redirect's
    // second was the least time, so the notice that follows waits only on
    // the page.
    #[test]
    fn a_finished_redirect_leaves_a_notice_without_a_deadline() {
        let mut session = on_news();
        let channel = session.tabs[0].channel.id;
        session.redirect_tab(channel, parse("https://elsewhere.invalid/").unwrap(), Instant::now(), RedirectBy::Server).unwrap();
        session.finish_redirect(channel);
        assert_ne!(session.tabs[0].channel.id, channel, "the destination's channel");
        assert_eq!(notice(&session), Some(Navigating { until: None, ready: false }));
        assert!(session.strip().navigating);
    }

    // REQ-7164 item 3: a channel with nothing to read has no page to wait
    // for; the strip does not say it is on its way.
    #[test]
    fn a_channel_with_a_problem_does_not_say_it_is_on_its_way() {
        let mut session = on_news();
        session.navigate("https://sport.invalid/", true).unwrap();
        assert!(session.strip().navigating);
        session.tabs[0].channel.problem = Some("nothing to read".into());
        assert!(!session.strip().navigating);
    }

    fn state_with(notice: Navigating) -> ChannelState {
        let mut session = on_news();
        session.tabs[0].channel.navigating = Some(notice);
        session.tabs.remove(0).channel
    }

    // REQ-7164 item 3: the notice ends when both its second has passed and
    // a page is ready, whichever comes later, and not before.
    #[test]
    fn the_notice_ends_at_the_later_of_its_second_and_its_page() {
        let start = Instant::now();
        let deadline = start + NAVIGATING_NOTICE;
        let just_before = deadline.checked_sub(Duration::from_millis(1)).unwrap();

        // The second first, then the page.
        let mut channel = state_with(Navigating::starting(start));
        assert!(!channel.navigating_at(just_before), "a millisecond early");
        assert!(channel.navigating.is_some_and(|notice| notice.until.is_some()));
        assert!(!channel.navigating_at(deadline), "no page yet");
        assert_eq!(channel.navigating, Some(Navigating { until: None, ready: false }), "the deadline is gone, so nothing waits on time");
        assert!(channel.navigating_ready(), "the page ends it");
        assert_eq!(channel.navigating, None);
        assert!(!channel.navigating_ready() && !channel.navigating_at(deadline), "nothing left to end");

        // The page first, then the second: the page arriving sooner does not
        // shorten it.
        let mut channel = state_with(Navigating::starting(start));
        assert!(!channel.navigating_ready(), "the second is not over");
        assert!(!channel.navigating_at(just_before));
        assert!(channel.navigating.is_some());
        assert!(channel.navigating_at(deadline), "ends at the deadline");
        assert_eq!(channel.navigating, None);

        // After a redirect: no second, the page alone ends it.
        let mut channel = state_with(Navigating::after_redirect());
        assert!(!channel.navigating_at(deadline));
        assert!(channel.navigating_ready());
    }
}
