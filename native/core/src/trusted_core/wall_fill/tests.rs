//! REQ-1559 item 3: the SELECTs that fill the wall.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use std::time::Duration;

use interface_core::canvas::PageKey;
use neoteletext_core::limits::SUBCODE_ANY;
use protocol::ToCore;
use protocol::decoder::FromDecoder;

use super::super::Core;
use super::super::test_support::core_on;

const CHANNEL: &str = "https://live.invalid/teletext/";
const FIRST: u16 = 0x101;
const SECOND: u16 = 0x102;
const THIRD: u16 = 0x103;

fn wall() -> (Core, confine::broker::stand_in::StandIn, u32) {
    let (mut core, stand_in) = core_on(CHANNEL);
    core.session.tabs[0].channel.live = true;
    let channel = core.session.tabs[0].channel.id;
    core.send_all().unwrap();
    (core, stand_in, channel)
}

fn missing(core: &mut Core, channel: u32, pages: &[u16]) {
    core.decoded(channel, FromDecoder::Missing(pages.to_vec())).unwrap();
}

fn selected(core: &Core, channel: u32) -> Option<(u16, u16)> {
    core.typing.selection(channel)
}

// REQ-1559: three missing, three SELECTs in screen order, each only after
// the frame of the one before, then SELECT page 0.
#[test]
fn missing_tiles_are_selected_in_turn_then_page_0() {
    let (mut core, _stand_in, channel) = wall();
    missing(&mut core, channel, &[FIRST, SECOND, THIRD]);
    assert_eq!(selected(&core, channel), Some((FIRST, SUBCODE_ANY)));
    missing(&mut core, channel, &[FIRST, SECOND, THIRD]);
    assert_eq!(selected(&core, channel), Some((FIRST, SUBCODE_ANY)), "waits for its frame");
    missing(&mut core, channel, &[SECOND, THIRD]);
    assert_eq!(selected(&core, channel), Some((SECOND, SUBCODE_ANY)));
    missing(&mut core, channel, &[THIRD]);
    assert_eq!(selected(&core, channel), Some((THIRD, SUBCODE_ANY)));
    missing(&mut core, channel, &[]);
    assert_eq!(selected(&core, channel), Some((0, SUBCODE_ANY)), "nothing stays live behind the wall");
}

// REQ-1559: a wall with nothing missing selects no page.
#[test]
fn a_cached_wall_selects_no_page() {
    let (mut core, _stand_in, channel) = wall();
    core.intent(ToCore::WallWithoutPage).unwrap();
    let before = selected(&core, channel);
    missing(&mut core, channel, &[]);
    assert_eq!(selected(&core, channel), before);
}

// REQ-1559: a page that never comes waits the reply limit and no longer,
// and is not selected again.
#[test]
fn a_tile_that_never_answers_gives_way_at_the_reply_limit() {
    let (mut core, _stand_in, channel) = wall();
    missing(&mut core, channel, &[FIRST, SECOND]);
    let due = core.fill_due_at().unwrap();
    assert!(core.next_wake().unwrap() <= due, "the core wakes for it");
    core.fill_tick(due.checked_sub(Duration::from_millis(1)).unwrap()).unwrap();
    assert_eq!(selected(&core, channel), Some((FIRST, SUBCODE_ANY)), "not before the limit");
    core.fill_tick(due).unwrap();
    assert_eq!(selected(&core, channel), Some((SECOND, SUBCODE_ANY)));
    missing(&mut core, channel, &[FIRST]);
    assert_eq!(selected(&core, channel), Some((0, SUBCODE_ANY)), "the one that never came is not asked for again");
}

// REQ-1559 item 3: scrolling fills what comes into view the same way.
#[test]
fn scrolling_fills_the_tiles_that_come_into_view() {
    let (mut core, _stand_in, channel) = wall();
    missing(&mut core, channel, &[FIRST]);
    missing(&mut core, channel, &[]);
    assert_eq!(selected(&core, channel), Some((0, SUBCODE_ANY)));
    missing(&mut core, channel, &[THIRD]);
    assert_eq!(selected(&core, channel), Some((THIRD, SUBCODE_ANY)));
}

// REQ-1559 item 4: opening a tile ends the filling: the page is the tab's
// selection, and a Missing list no longer moves it.
#[test]
fn opening_a_tile_ends_the_filling() {
    let (mut core, _stand_in, channel) = wall();
    missing(&mut core, channel, &[FIRST, SECOND]);
    core.intent(ToCore::Opened(Some(PageKey { channel, page: THIRD }))).unwrap();
    assert_eq!(selected(&core, channel), Some((THIRD, SUBCODE_ANY)), "one SELECT, of the page opened");
    assert_eq!(core.fill_due_at(), None);
    missing(&mut core, channel, &[SECOND]);
    assert_eq!(selected(&core, channel), Some((THIRD, SUBCODE_ANY)));
}

// A file or fetched channel has every page: nothing is selected.
#[test]
fn a_channel_that_is_not_live_selects_nothing() {
    let (mut core, _stand_in, channel) = wall();
    core.session.tabs[0].channel.live = false;
    let before = selected(&core, channel);
    missing(&mut core, channel, &[FIRST]);
    assert_eq!(selected(&core, channel), before);
    assert_eq!(core.fill_due_at(), None);
}
