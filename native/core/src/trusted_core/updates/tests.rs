//! Tests of updates in the trusted core (`installation-and-updates.md` sections 3.2 and 3.4).
#![allow(clippy::unwrap_used, reason = "a test")]

use super::*;

/// A browser the launcher started, of a build with update keys.
fn launched(version: Version) -> Updates {
    Updates { keyed: true, ..Updates::new(Some(State::installed(version)), None, true) }
}

/// Section 3.4: a newer version is offered once in the strip, nothing
/// is downloaded until the user accepts, and a bad one is never offered.
#[test]
fn a_newer_version_is_offered_once_and_downloaded_only_when_accepted() {
    let mut updates = launched(Version::new(0, 1, 0));
    updates.said(FromUpdater::Latest { version: Some(Version::new(0, 2, 0)), checked: 100 }, 100);
    assert!(updates.strip, "offered in the strip");
    assert_eq!(updates.offer(), Some(Version::new(0, 2, 0)));
    assert_eq!(updates.downloading, None, "nothing downloaded before the user accepts");
    updates.later();
    assert!(!updates.strip);
    updates.said(FromUpdater::Latest { version: Some(Version::new(0, 2, 0)), checked: 200 }, 200);
    assert!(!updates.strip, "offered once");
    assert_eq!(updates.status(true, 200), "NeoTeletext 0.2.0 is available", "the panel keeps offering it");
    updates.install();
    assert_eq!(updates.downloading, Some(Version::new(0, 2, 0)));
    updates.said(FromUpdater::Staged(Version::new(0, 2, 0)), 200);
    assert_eq!(updates.status(true, 200), "NeoTeletext 0.2.0 starts at the next start");
    assert_eq!(updates.offer(), None);

    let mut state = State::installed(Version::new(0, 1, 0));
    state.mark_bad(Version::new(0, 2, 0));
    let mut updates = Updates { keyed: true, ..Updates::new(Some(state), None, true) };
    updates.said(FromUpdater::Latest { version: Some(Version::new(0, 2, 0)), checked: 100 }, 100);
    assert_eq!(updates.offer(), None, "a version rolled back is never offered");
    assert!(!updates.strip);
    updates.said(FromUpdater::Latest { version: Some(Version::new(0, 1, 0)), checked: 100 }, 100);
    assert_eq!(updates.offer(), None, "nor an older or the same version");
}

/// Section 3.4: "Updates unavailable" after 7 days without a successful
/// check, never sooner; and the panel's other states.
#[test]
fn updates_are_unavailable_only_after_seven_days() {
    let unkeyed = Updates::new(Some(State::installed(Version::new(0, 1, 0))), None, true);
    if !unkeyed.keyed {
        assert!(unkeyed.status(true, 0).contains("no update keys"));
    }
    let mut updates = launched(Version::new(0, 1, 0));
    updates.said(FromUpdater::Latest { version: None, checked: 1_000 }, 1_000);
    assert_eq!(updates.status(true, 1_000), "NeoTeletext is up to date");
    updates.said(FromUpdater::Failed { reason: String::from("the timestamp has expired"), last_success: Some(1_000) }, 1_000);
    assert!(updates.status(true, 1_000 + UNAVAILABLE_AFTER).starts_with("Could not check"), "not yet");
    assert_eq!(updates.status(true, 1_001 + UNAVAILABLE_AFTER), "Updates unavailable");
    assert_eq!(updates.status(false, 1_001 + UNAVAILABLE_AFTER), "Not checking for updates");
}

/// Finding F-93: the age of the last successful check is the core's own,
/// from when it received the updater's report; the times the updater
/// reports, on its clock, change nothing.
#[test]
fn the_age_of_a_check_is_on_the_cores_clock() {
    let mut updates = launched(Version::new(0, 1, 0));
    // An updater whose clock is far ahead: its check would seem fresh
    // for ever.
    updates.said(FromUpdater::Latest { version: None, checked: i64::MAX }, 1_000);
    updates.said(FromUpdater::Failed { reason: String::from("the timestamp has expired"), last_success: Some(i64::MAX) }, 2_000);
    assert!(updates.status(true, 1_000 + UNAVAILABLE_AFTER).starts_with("Could not check"), "not yet");
    assert_eq!(updates.status(true, 1_001 + UNAVAILABLE_AFTER), "Updates unavailable");
    // And one whose clock is far behind: its success is not stale at once.
    let mut behind = launched(Version::new(0, 1, 0));
    behind.said(FromUpdater::Latest { version: None, checked: 0 }, 1_000);
    behind.said(FromUpdater::Failed { reason: String::from("no network"), last_success: Some(0) }, 1_000);
    assert!(behind.status(true, 1_000 + UNAVAILABLE_AFTER).starts_with("Could not check"));
    // A failure alone is not a success at any time the updater names.
    let mut failed = launched(Version::new(0, 1, 0));
    failed.said(FromUpdater::Failed { reason: String::from("no network"), last_success: Some(i64::MAX) }, 1_000);
    assert_eq!(failed.status(true, 1_000), "Updates unavailable");
}

/// Section 3.2: healthy once the window has drawn within 30 seconds,
/// the settings were read and no helper failed, after a moment to fail.
#[test]
fn a_version_is_healthy_only_when_it_drew_read_its_settings_and_lost_no_helper() {
    let mut updates = launched(Version::new(0, 1, 0));
    let now = Instant::now();
    assert!(!updates.health.healthy(now), "nothing drawn");
    updates.drawn();
    assert!(!updates.health.healthy(now), "not settled");
    assert!(updates.health.healthy(now + HEALTH_SETTLE + Duration::from_millis(1)));
    updates.helper_failed("the decoder for channel 1 stopped: cannot apply Landlock");
    assert!(!updates.health.healthy(now + HEALTH_SETTLE * 2), "a helper that did not confine");
    let mut unread = Updates::new(Some(State::installed(Version::new(0, 1, 0))), None, false);
    unread.drawn();
    assert!(!unread.health.healthy(now + HEALTH_SETTLE * 2), "settings unreadable");
    let mut late = launched(Version::new(0, 1, 0));
    late.health.started = now.checked_sub(HEALTH_WINDOW * 2).unwrap();
    late.drawn();
    assert!(!late.health.healthy(Instant::now() + HEALTH_SETTLE * 2), "drawn too late");
}

#[test]
fn a_browser_not_installed_does_not_update() {
    let updates = Updates::default();
    assert_eq!(updates.running(), None);
    assert!(updates.status(true, 0).contains("not installed"));
}

/// Finding F-10: the updater, as every other helper, is held at its own
/// budget when it writes faster than the trusted core handles its
/// messages, on its own pipe, so the core's queue of its messages does not
/// grow without bound.
#[test]
fn an_updater_that_floods_the_core_is_held_at_its_budget() {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicUsize, Ordering};
    /// How long the trusted core handles nothing.
    const HOLD: Duration = Duration::from_secs(1);
    /// Where a flood is stopped when nothing holds it: four times the
    /// budget, far past what the bound lets through.
    const FLOOD_CAP: usize = 4 * neoteletext::budget::MAX_WAITING_BYTES;
    /// What a pipe and the reader's buffer hold beyond the budget: Linux's
    /// largest pipe without privileges (`/proc/sys/fs/pipe-max-size`, 1
    /// MiB) is more than macOS's 64 KiB and the reader's 8 KiB together.
    const PIPE_AND_READER_BUFFERS: usize = 1024 * 1024;
    let (reader, mut writer) = std::io::pipe().unwrap();
    let (events, incoming) = std::sync::mpsc::channel();
    listen(reader, events);
    // The updater's longest message, so the flood is as many bytes as it
    // can be for its messages.
    let mut message = Vec::new();
    let reason = "x".repeat(update::wire::MAX_REASON);
    update::wire::write_from_updater(&mut message, &FromUpdater::Failed { reason, last_success: Some(i64::MAX) }).unwrap();
    let message_bytes = message.len();
    let written = Arc::new(AtomicUsize::new(0));
    let theirs = Arc::clone(&written);
    let flood = std::thread::spawn(move || {
        while theirs.load(Ordering::Relaxed) < FLOOD_CAP && writer.write_all(&message).is_ok() {
            theirs.fetch_add(message.len(), Ordering::Relaxed);
        }
    });
    std::thread::sleep(HOLD);
    let held_at = written.load(Ordering::Relaxed);
    // The core goes: its queued messages are given back, and the updater's
    // writes end.
    drop(incoming);
    flood.join().unwrap();
    let most = neoteletext::budget::MAX_WAITING_BYTES + PIPE_AND_READER_BUFFERS + message_bytes;
    assert!(held_at <= most, "the updater wrote {held_at} bytes while the core handled nothing; at most {most}");
}
