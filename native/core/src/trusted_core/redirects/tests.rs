//! REQ-701 and REQ-1686: a redirect that leaves a channel is followed as a
//! web browser follows one, after a second's notice, against the stand-in
//! broker the new channel's helpers need.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use std::time::{Duration, Instant};

use confine::broker::Program;
use confine::broker::stand_in::StandIn;
use protocol::ToCore;
use protocol::fetcher::FromFetcher;

use super::super::Core;
use super::super::test_support::core_on;
use super::REDIRECT_NOTICE;
use crate::session::MAX_REDIRECTS;

const BEFORE: &str = "https://before.invalid/teletext/";
const REDIRECTING: &str = "https://moved.invalid/teletext/";
const DESTINATION: &str = "https://new.invalid/teletext/";
const ONE_MILLISECOND: Duration = Duration::from_millis(1);

/// A tab that was on [`BEFORE`] and went to [`REDIRECTING`], whose fetcher
/// is starting.
fn opening() -> (Core, StandIn) {
    let (mut core, stand_in) = core_on(BEFORE);
    core.intent(ToCore::Navigate(REDIRECTING.into())).unwrap();
    (core, stand_in)
}

fn address(core: &Core) -> &str {
    &core.session.active_tab().unwrap().address().unwrap().text
}

fn channel(core: &Core) -> u32 {
    core.session.active_tab().unwrap().channel.id
}

fn redirect(core: &mut Core, location: &str) {
    core.fetched(channel(core), FromFetcher::Redirected(location.into())).unwrap();
}

/// The notice's second ends.
fn elapse(core: &mut Core) {
    let due = core.redirect_due_at().unwrap_or_else(Instant::now);
    core.finish_redirects(due).unwrap();
}

/// A redirect followed: said, and its second over.
fn hop(core: &mut Core, location: &str) {
    redirect(core, location);
    elapse(core);
}

/// Whether the strip says `words`, as a channel that failed to open does.
fn says(core: &Core, words: &str) -> bool {
    core.strip().notice.contains(words)
}

// REQ-1686 items 2, 4, 5 and 6: the trusted core names the destination, the
// address field keeps the old address, and nothing moves until the second
// is up, for exactly the named time; then the notice goes.
#[test]
fn a_redirect_shows_its_notice_for_a_second_then_moves() {
    let (mut core, _stand_in) = opening();
    let old = channel(&core);
    let said = Instant::now();
    redirect(&mut core, DESTINATION);
    assert_eq!(core.strip().redirecting, DESTINATION, "the notice names it");
    assert_eq!(core.strip().address, REDIRECTING, "the address field keeps the old address");
    let due = core.redirect_due_at().unwrap();
    assert!(due >= said + REDIRECT_NOTICE && due <= Instant::now() + REDIRECT_NOTICE, "exactly the notice's time");
    assert!(core.next_wake().unwrap() <= due, "the core wakes for it");
    core.finish_redirects(due.checked_sub(ONE_MILLISECOND).unwrap()).unwrap();
    assert_eq!((address(&core), channel(&core)), (REDIRECTING, old), "not before the second is up");
    core.finish_redirects(due).unwrap();
    assert_eq!(address(&core), DESTINATION);
    assert_eq!(core.strip().redirecting, "", "the notice has gone");
    assert_eq!(core.redirect_due_at(), None);
}

// REQ-701 items 1 and 3: no question, the address shows the destination, the
// destination has its own channel and fetcher, and the visit is the page's.
#[test]
fn a_redirect_moves_the_tab_with_no_question() {
    let (mut core, stand_in) = opening();
    let old = channel(&core);
    hop(&mut core, DESTINATION);
    assert_eq!(address(&core), DESTINATION);
    assert_eq!(core.strip().address, DESTINATION, "the address field shows the destination");
    assert_ne!(channel(&core), old, "a channel of its own");
    assert!(core.session.active_tab().unwrap().history.last().unwrap().by_page, "recorded as the page's");
    let fetchers = stand_in.started().into_iter().filter(|helper| helper.program == Program::Fetcher).count();
    assert_eq!(fetchers, 2, "the redirecting address's, then the destination's");
    assert!(core.fetchers.contains_key(&channel(&core)), "the destination's fetcher");
    assert!(!core.fetchers.contains_key(&old), "the redirecting channel's is gone");
}

// REQ-701 item 3: Back skips the address that redirected.
#[test]
fn back_returns_to_the_page_before_the_redirecting_address() {
    let (mut core, _stand_in) = opening();
    hop(&mut core, DESTINATION);
    assert_eq!(address(&core), DESTINATION);
    assert_eq!(core.session.active_tab().unwrap().history.len(), 2, "the destination replaced the address that redirected");
    core.intent(ToCore::Back).unwrap();
    assert_eq!(address(&core), BEFORE);
}

// REQ-1686 item 7: Back during the second replaces the redirect.
#[test]
fn back_during_the_second_drops_the_redirect() {
    let (mut core, _stand_in) = opening();
    redirect(&mut core, DESTINATION);
    assert_eq!(address(&core), REDIRECTING, "within its second");
    core.intent(ToCore::Back).unwrap();
    assert_eq!(address(&core), BEFORE);
    assert_eq!(core.strip().redirecting, "", "the notice went with it");
    assert_eq!(core.redirect_due_at(), None);
    core.finish_redirects(Instant::now() + 2 * REDIRECT_NOTICE).unwrap();
    assert_eq!(address(&core), BEFORE, "the tab stays where Back took it");
}

// REQ-1686 item 7: so does any other navigation of the viewer's own.
#[test]
fn the_viewers_own_navigation_drops_the_redirect() {
    for (intent, stays) in [
        (ToCore::Navigate("https://typed.invalid/".into()), "https://typed.invalid/"),
        (ToCore::Reload, REDIRECTING),
        (ToCore::Navigate(format!("{REDIRECTING}302")), "https://moved.invalid/teletext/302"),
    ] {
        let (mut core, _stand_in) = opening();
        redirect(&mut core, DESTINATION);
        core.intent(intent).unwrap();
        assert_eq!(core.redirect_due_at(), None, "{stays}");
        assert_eq!(core.strip().redirecting, "", "{stays}");
        assert_eq!(address(&core), stays);
    }
}

// REQ-1686 item 7: a tab closed takes its redirect.
#[test]
fn closing_the_tab_drops_the_redirect() {
    let (mut core, _stand_in) = opening();
    core.session.open("https://other.invalid/").unwrap();
    core.send_all().unwrap();
    let behind = core.session.tabs[0].channel.id;
    core.fetched(behind, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    assert!(core.redirect_due_at().is_some());
    core.intent(ToCore::CloseTab(0)).unwrap();
    assert_eq!(core.redirect_due_at(), None);
}

// REQ-1686 item 8: another tab, even of the same channel, is not touched.
#[test]
fn a_second_tab_of_the_channel_keeps_its_page() {
    let (mut core, _stand_in) = opening();
    let first = channel(&core);
    core.session.open(REDIRECTING).unwrap();
    core.send_all().unwrap();
    let second = channel(&core);
    core.fetched(first, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    assert_eq!(core.strip().redirecting, "", "the tab in front shows no notice");
    assert_eq!(core.session.tabs[0].address().unwrap().text, REDIRECTING, "within its second");
    elapse(&mut core);
    assert_eq!((address(&core), channel(&core)), (REDIRECTING, second), "the tab in front stays");
    assert_eq!(core.session.tabs[0].address().unwrap().text, DESTINATION, "the tab behind ran its own second");
}

// REQ-1686 item 8: switching to a tab within its second shows its notice,
// and switching away puts it away.
#[test]
fn the_notice_is_the_tabs_own() {
    let (mut core, _stand_in) = opening();
    let first = channel(&core);
    core.session.open("https://other.invalid/").unwrap();
    core.send_all().unwrap();
    core.fetched(first, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    core.intent(ToCore::SelectTab(0)).unwrap();
    assert_eq!(core.strip().redirecting, DESTINATION);
    core.intent(ToCore::SelectTab(1)).unwrap();
    assert_eq!(core.strip().redirecting, "");
}

// REQ-701 item 2: a redirect leads only to an address the rules accept.
#[test]
fn a_refused_destination_is_refused() {
    for location in
        ["file:///etc/passwd", "/etc/passwd", "neoteletext:home", "ftp://new.invalid/", "https://user@new.invalid/", "new.invalid", "javascript:alert(1)", ""]
    {
        let (mut core, _stand_in) = opening();
        let old = channel(&core);
        redirect(&mut core, location);
        assert_eq!(core.strip().redirecting, "", "{location}: no notice");
        elapse(&mut core);
        assert_eq!((address(&core), channel(&core)), (REDIRECTING, old), "{location}");
        assert!(says(&core, "could not open"), "{location}");
    }
}

// REQ-701 item 4, REQ-1686 item 9: twenty are followed, each with its own
// notice, and the twenty-first is refused, as the Fetch standard's limit is.
#[test]
fn a_chain_past_the_bound_fails() {
    let (mut core, _stand_in) = opening();
    for hop_number in 1..=MAX_REDIRECTS {
        let location = format!("https://hop{hop_number}.invalid/");
        redirect(&mut core, &location);
        assert_eq!(core.strip().redirecting, location, "hop {hop_number} has its own notice");
        elapse(&mut core);
        assert_eq!(address(&core), location, "hop {hop_number} is followed");
    }
    let last = channel(&core);
    redirect(&mut core, "https://onetoomany.invalid/");
    assert_eq!(core.strip().redirecting, "", "no notice");
    elapse(&mut core);
    assert_eq!(channel(&core), last, "refused");
    assert!(says(&core, "too many redirects"));
    assert_eq!(core.session.active_tab().unwrap().history.len(), 2);
}

// REQ-701 item 4: a chain that comes round again stops at once.
#[test]
fn a_loop_fails() {
    let (mut core, _stand_in) = opening();
    hop(&mut core, DESTINATION);
    let last = channel(&core);
    redirect(&mut core, REDIRECTING);
    elapse(&mut core);
    assert_eq!((address(&core), channel(&core)), (DESTINATION, last), "refused");
    assert!(says(&core, "redirects in a loop"));
}

// REQ-701 item 4: the chain ends with it; a new navigation counts afresh.
#[test]
fn the_chain_is_the_navigations() {
    let (mut core, _stand_in) = opening();
    hop(&mut core, DESTINATION);
    core.intent(ToCore::Navigate(REDIRECTING.into())).unwrap();
    hop(&mut core, DESTINATION);
    assert_eq!(address(&core), DESTINATION, "not a loop: the viewer went there");
}

// A fetcher says it again and again: its second is not put off.
#[test]
fn saying_it_again_does_not_put_the_second_off() {
    let (mut core, _stand_in) = opening();
    redirect(&mut core, DESTINATION);
    let due = core.redirect_due_at().unwrap();
    redirect(&mut core, "https://later.invalid/");
    assert_eq!(core.redirect_due_at(), Some(due));
    assert_eq!(core.strip().redirecting, DESTINATION);
}

// REQ-701 item 5: https to http is followed, and the strip says so.
#[test]
fn a_downgrade_is_followed_and_shown_not_secure() {
    let (mut core, _stand_in) = opening();
    assert!(core.strip().encrypted);
    hop(&mut core, "http://new.invalid/teletext/");
    assert_eq!(address(&core), "http://new.invalid/teletext/");
    assert!(!core.strip().encrypted);
}

// RFC 9110 section 10.2.2, as REQ-505 keeps a fragment off the wire: the
// page the viewer asked for goes with a redirect that names none.
#[test]
fn a_fragment_goes_with_a_redirect_that_names_none() {
    let (mut core, _stand_in) = core_on(BEFORE);
    core.intent(ToCore::Navigate(format!("{REDIRECTING}302"))).unwrap();
    redirect(&mut core, DESTINATION);
    assert_eq!(core.strip().redirecting, format!("{DESTINATION}302"), "the complete address");
    elapse(&mut core);
    assert_eq!(address(&core), format!("{DESTINATION}302"));
    core.intent(ToCore::Navigate(format!("{REDIRECTING}302"))).unwrap();
    hop(&mut core, &format!("{DESTINATION}401"));
    assert_eq!(address(&core), format!("{DESTINATION}401"));
}

// REQ-701: a page fetched once the channel is open is no navigation, and a
// fetcher, which is untrusted, cannot move its tab by saying one is.
#[test]
fn an_open_channel_is_not_moved() {
    let (mut core, _stand_in) = opening();
    let open = channel(&core);
    core.fetched(open, FromFetcher::InitialPage(0x100)).unwrap();
    hop(&mut core, DESTINATION);
    assert_eq!((address(&core), channel(&core)), (format!("{REDIRECTING}100").as_str(), open));
    assert_eq!(core.strip().redirecting, "");
}

// REQ-701: a message from a channel no tab holds moves nothing.
#[test]
fn a_gone_channel_moves_nothing() {
    let (mut core, _stand_in) = opening();
    let (shown, only) = (address(&core).to_owned(), channel(&core));
    core.fetched(only + 1_000, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    assert_eq!(core.redirect_due_at(), None);
    assert_eq!((address(&core), channel(&core)), (shown.as_str(), only));
}

#[cfg(feature = "interaction")]
mod live;
