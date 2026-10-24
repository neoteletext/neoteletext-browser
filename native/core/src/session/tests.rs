//! Tests of the tabs, their history, the windows and the kept session.

use super::*;
use interface_core::channel::page_index;
use interface_core::library::{MAX_CLOSED, MAX_TAB_ENTRIES, MAX_TAB_LIST};
use neoteletext::restore::Saved;
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
    assert_eq!(session.show_page(0x302).map(|address| address.text.as_str()), Some("https://news.invalid/teletext/#302"));
    assert!(session.tabs[0].history[1].by_page && !session.tabs[0].history[0].by_page);
    assert_eq!(session.tabs[0].channel.id, channel, "the same channel, still open");
    assert_eq!(session.go(false), Went::Page(None));
    assert_eq!(session.go(true), Went::Page(Some(0x302)));
    assert_eq!(session.tabs[0].channel.id, channel);
    // A link to another channel opens it, recorded as the page's.
    session.follow(parse("https://sport.invalid/#401").unwrap()).unwrap();
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

#[test]
fn the_contents_file_lists_pages_and_names_sections() {
    let mut session = Session::new(Bookmarks::default());
    session.open("news.invalid").unwrap();
    let channel = &mut session.tabs[0].channel;
    channel.set_contents(&[
        Entry::Section("News".into()),
        Entry::Page { page: 0x100, title: "Front page".into() },
        Entry::Page { page: 0x101, title: "Headlines".into() },
        Entry::Section("Sport".into()),
        Entry::Page { page: 0x300, title: "Sport index".into() },
    ]);
    let view = channel.view();
    assert_eq!(view.counts(), (3, 0), "listed, none arrived yet");
    assert_eq!((view.banks[0].as_str(), view.banks[2].as_str()), ("News", "Sport"));
    // The page's own title does not replace the contents file's.
    assert_eq!(channel.arrived(0x101, "Something else").as_deref(), Some("Headlines"));
    assert_eq!(channel.view().counts(), (3, 1));
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
    session.open("news.invalid/#302").unwrap();
    assert!(!session.strip().bookmarked);
    session.bookmarks.add(neoteletext::bookmarks::OTHER, "", "news.invalid/#302").unwrap();
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
    session.navigate("sport.invalid/#401", false).unwrap();
    session.tabs[0].pending = None;
    assert_eq!(session.go(false), Went::Page(None));
    assert_eq!(session.tabs[0].pending, Some(Pending { typed: false, drawn: true, reload: false }), "the same channel, already drawing");
    session.open_private("news.invalid").unwrap();
    assert!(session.tab_infos()[1].private && session.strip().private);
}

/// Requirement 20.10, as Chrome's Back list: twelve entries, then,
/// beyond them, up to five chapter stops where the host changes; a
/// tab keeps at most 50 entries, the oldest dropped.
#[test]
fn back_lists_twelve_then_chapter_stops_and_a_tab_keeps_fifty() {
    let mut session = Session::new(Bookmarks::default());
    session.open("a.invalid").unwrap();
    // a, then b×3, c×3, d×3, e×3, f×3, g×3, h×3: 22 entries, the last h.
    for host in ["b", "c", "d", "e", "f", "g", "h"] {
        for page in ["#100", "#200", "#300"] {
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
        session.navigate(&format!("z.invalid/#{:03}", 100 + page), false).unwrap();
    }
    let tab = &session.tabs[0];
    assert_eq!((tab.history.len(), tab.position), (MAX_TAB_ENTRIES, MAX_TAB_ENTRIES - 1), "the oldest dropped");
}

/// Requirement 20.6, as Chrome: removing history removes those entries
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

/// Requirement 20.9, as Chrome: the recently closed tabs, with each
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
    session.navigate("sport.invalid/#401", false).unwrap();
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
    // The address shown, entered again or linked to, is a reload, as in
    // Chrome: no new entry.
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
/// reopen together in a window of their own, as Chrome reopens a closed
/// window.
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
    for number in 0..FILE_TABS {
        session.open(&format!("/recordings/{number}.t42")).unwrap();
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
