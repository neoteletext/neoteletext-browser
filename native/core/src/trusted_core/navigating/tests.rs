//! REQ-7164 and DEC-7166: the trusted core starts the notice of a tab going
//! to another channel, keeps it for its second at least and until the page
//! the tab has open has gone to the interface, and tells the strip.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use std::time::{Duration, Instant};

use interface_core::browser::{CertificateProblem, Failed};
use interface_core::canvas::PageKey;
use protocol::fetcher::{FromFetcher, RefusedFile};
use protocol::{ToCore, ToInterface};

use super::super::Core;
use super::super::test_support::{core_watched, sent_to_interface};
use crate::session::Navigating;

const BEFORE: &str = "https://before.invalid/teletext/";
const OTHER: &str = "https://other.invalid/teletext/";
const DESTINATION: &str = "https://new.invalid/teletext/";
/// REQ-7164 item 3: the words show for at least one second. The
/// requirement's figure, not the session's constant, so that a change to
/// the constant is caught here.
const NAVIGATING_NOTICE: Duration = Duration::from_secs(1);
const ONE_MILLISECOND: Duration = Duration::from_millis(1);
/// The page the tab opens, and another.
const OPEN_PAGE: u16 = 0x100;
const ANOTHER_PAGE: u16 = 0x200;

fn channel(core: &Core) -> u32 {
    core.session.active_tab().unwrap().channel.id
}

fn notice(core: &Core) -> Option<Navigating> {
    core.session.active_tab().unwrap().channel.navigating
}

/// The strips sent to the interface since the last look.
fn strips(core: &mut Core, written: &std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<bool> {
    sent_to_interface(core, written)
        .into_iter()
        .filter_map(|message| if let ToInterface::Strip(strip) = message { Some(strip.navigating) } else { None })
        .collect()
}

/// A frame of `page` of the active tab's channel goes to the interface.
fn frame(core: &mut Core, page: u16) {
    let key = PageKey { channel: channel(core), page };
    core.send_frame(key, 1, Vec::new()).unwrap();
}

/// The tab opens [`OPEN_PAGE`], as the interface says when it shows it.
fn opened(core: &mut Core) {
    let key = PageKey { channel: channel(core), page: OPEN_PAGE };
    core.intent(ToCore::Opened(Some(key))).unwrap();
}

// REQ-7164 items 1 and 4: the strip says so after a change of channel, and
// not after a reload or a change of page.
#[test]
fn the_strip_says_navigating_only_after_a_change_of_channel() {
    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    assert!(!core.strip().navigating, "a tab newly opened");
    core.intent(ToCore::Navigate(format!("{BEFORE}200"))).unwrap();
    assert!(!core.strip().navigating, "another page of the channel");
    core.intent(ToCore::Reload).unwrap();
    assert!(!core.strip().navigating, "a reload");
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    assert!(core.strip().navigating, "another channel");
}

// REQ-7164 item 3: the deadline is the notice's second after the
// navigation, the run loop wakes for it, and once it has passed with the
// page still awaited there is no deadline left, so the loop does not spin.
#[test]
fn the_deadline_is_one_second_and_goes_once_passed() {
    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    let before = Instant::now();
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    let due = core.navigating_due_at().unwrap();
    assert!(due >= before + NAVIGATING_NOTICE && due <= Instant::now() + NAVIGATING_NOTICE, "{due:?}");
    assert!(core.next_wake().unwrap() <= due, "the core wakes for it");
    core.navigating_tick(due.checked_sub(ONE_MILLISECOND).unwrap()).unwrap();
    assert!(core.strip().navigating && core.navigating_due_at() == Some(due), "not before its second");
    core.navigating_tick(due).unwrap();
    assert!(core.strip().navigating, "no page yet");
    assert_eq!(core.navigating_due_at(), None, "nothing left to wake for");
}

// REQ-7164 item 3: only a frame of the page the tab has open ends it; and
// the strip is sent again when it ends.
#[test]
fn a_frame_of_the_open_page_ends_it_and_another_pages_does_not() {
    let (mut core, _stand_in, written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    opened(&mut core);
    let due = core.navigating_due_at().unwrap();
    core.navigating_tick(due).unwrap();
    strips(&mut core, &written);
    frame(&mut core, ANOTHER_PAGE);
    assert!(core.strip().navigating, "a frame of a page the tab does not have open");
    assert_eq!(strips(&mut core, &written), [] as [bool; 0], "nothing told");
    frame(&mut core, OPEN_PAGE);
    assert!(!core.strip().navigating, "the open page's frame");
    assert_eq!(notice(&core), None);
    assert_eq!(strips(&mut core, &written), [false], "the strip is told once");
}

// REQ-7164 item 3: a page ready before the second does not shorten it.
#[test]
fn a_page_ready_early_waits_for_the_second() {
    let (mut core, _stand_in, written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    opened(&mut core);
    let due = core.navigating_due_at().unwrap();
    frame(&mut core, OPEN_PAGE);
    assert!(core.strip().navigating, "within its second");
    strips(&mut core, &written);
    core.navigating_tick(due).unwrap();
    assert!(!core.strip().navigating);
    assert_eq!(strips(&mut core, &written), [false]);
}

// REQ-7164 item 3: on the wall no page is open, and any frame of the
// channel is the one waited for.
#[test]
fn on_the_wall_any_frame_of_the_channel_ends_it() {
    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    core.navigating_tick(core.navigating_due_at().unwrap()).unwrap();
    frame(&mut core, ANOTHER_PAGE);
    assert!(!core.strip().navigating);
}

// REQ-7164 item 5: a second navigation during the notice gives a new one
// counted from it; Back during it to the first channel starts its own.
#[test]
fn a_navigation_during_the_notice_replaces_it() {
    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    let first = core.navigating_due_at().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let before = Instant::now();
    core.intent(ToCore::Navigate(DESTINATION.into())).unwrap();
    let second = core.navigating_due_at().unwrap();
    assert!(second > first && second >= before + NAVIGATING_NOTICE, "counted from the second navigation");
    std::thread::sleep(Duration::from_millis(20));
    let before = Instant::now();
    core.intent(ToCore::Back).unwrap();
    assert_eq!(core.session.active_tab().unwrap().address().unwrap().text, OTHER);
    let back = core.navigating_due_at().unwrap();
    assert!(back > second && back >= before + NAVIGATING_NOTICE, "Back starts its own");
    assert!(core.strip().navigating);
}

// REQ-7164 item 3: why the tab shows nothing of the channel replaces the
// words: a failed channel and a refused certificate.
#[test]
fn a_failure_or_a_refused_certificate_replaces_the_words() {
    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    let id = channel(&core);
    core.fetched(id, FromFetcher::Refused { file: RefusedFile::Channel, status: Some(404) }).unwrap();
    assert_eq!(core.strip().failed, Some(Failed::Channel(Some(404))));
    assert!(!core.strip().navigating, "a failed channel");

    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    let id = channel(&core);
    core.fetched(id, FromFetcher::CertificateRefused(CertificateProblem::Expired)).unwrap();
    assert_eq!(core.strip().refused, Some(CertificateProblem::Expired));
    assert!(!core.strip().navigating, "a refused certificate");
}

// REQ-7164, with REQ-1686 item 8: a tab behind others runs its own notice
// and ends it without sending the strip of the tab in front.
#[test]
fn a_tab_behind_others_ends_its_notice_without_sending_the_strip() {
    let (mut core, _stand_in, written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    let behind = channel(&core);
    core.session.open("https://front.invalid/teletext/").unwrap();
    assert_ne!(channel(&core), behind);
    let due = core.navigating_due_at().unwrap();
    core.send_frame(PageKey { channel: behind, page: OPEN_PAGE }, 1, Vec::new()).unwrap();
    strips(&mut core, &written);
    core.navigating_tick(due).unwrap();
    assert!(core.session.all_tabs().all(|tab| tab.channel.navigating.is_none()), "ended");
    assert_eq!(strips(&mut core, &written), [] as [bool; 0], "the front tab's strip is not sent");
}

// DEC-7166 item 1: a redirect out of the channel shows its notice for its
// second, then the words show until a frame arrives, with no further second.
#[test]
fn after_a_redirect_the_words_last_only_until_the_page() {
    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    let id = channel(&core);
    core.fetched(id, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    assert_eq!(core.strip().redirecting, DESTINATION, "the redirect's notice");
    let due = core.redirect_due_at().unwrap();
    core.finish_redirects(due).unwrap();
    let strip = core.strip();
    assert_eq!((strip.redirecting.as_str(), strip.navigating), ("", true), "then the words");
    assert_eq!(notice(&core), Some(Navigating::after_redirect()));
    assert_eq!(core.navigating_due_at(), None, "no further second");
    assert!(core.session.active_tab().unwrap().channel.redirecting.is_none(), "the redirect is over");
    frame(&mut core, OPEN_PAGE);
    assert!(!core.strip().navigating, "the page ends it at once");
}

// REQ-7164 item 3: the run loop's tick ends a notice whose second has
// passed and whose page is ready, on the real clock.
#[test]
fn the_run_loops_tick_ends_the_notice_at_its_deadline() {
    let (mut core, _stand_in, _written) = core_watched(BEFORE);
    core.intent(ToCore::Navigate(OTHER.into())).unwrap();
    opened(&mut core);
    frame(&mut core, OPEN_PAGE);
    let due = core.navigating_due_at().unwrap();
    core.tick().unwrap();
    assert!(core.strip().navigating, "within its second");
    std::thread::sleep(due.saturating_duration_since(Instant::now()) + ONE_MILLISECOND);
    core.tick().unwrap();
    assert!(!core.strip().navigating, "the tick ended it");
}
